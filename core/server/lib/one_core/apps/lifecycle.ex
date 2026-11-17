defmodule OneCore.Apps.Lifecycle do
  @moduledoc """
  What moves an install through its life (`OneCore.Apps.Installed`) and an app through its updates
  (`OneCore.Apps.Update`): installing, a stage at a time; uninstalling to Recently Deleted, restoring, and removing
  for good 30 days after; and taking a newer version of an app, or switching whether it's trusted, for everyone who
  has it.

  A newer version is a new commit at the git source an app was deployed from, which changes its folder and comes
  after the commit it runs, so taking it loses nothing that's live: servers acting for whoever the app is look
  once a day, and when someone asks (`apps_check`). A trusted app takes it at once; an untrusted one offers it,
  for someone who may to Update once an agent has read what changed. A failed update leaves the version that ran
  running. One deployed with changes not committed has no newer commit to take.

  An app requires others (`OneCore.Apps.Requires`). Installing it installs first each of its requirements that
  isn't there, as their own installs, and fails, naming it, where one can't be. Uninstalling one that others there are
  required by takes them to Recently Deleted with it, as one, once whoever asked confirms it; restoring any of them
  brings them all back. Uninstalling one that only requires others leaves them installed.

  It's a GenServer only for the clock: once a day it looks for newer versions, and removes for good what's been in
  Recently Deleted for 30 days, for whoever this server acts for.
  """
  use GenServer
  require Logger
  alias OneCore.{Bucket, Hooks, Server, Servers}
  alias OneCore.Call.State
  alias OneCore.Apps
  alias OneCore.Apps.{Deploy, Git, Installed, LoadedVersion, Requires, Store, Update}

  @network "network"
  @day :timer.hours(24)
  @kept :timer.hours(24 * 30)
  # How long installing or updating may take before it's taken as cut short, as by its server stopping.
  @partway :timer.minutes(15)

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @impl true
  def init(nil) do
    Process.send_after(self(), :daily, :timer.minutes(5))
    {:ok, nil}
  end

  @impl true
  def handle_info(:daily, state) do
    Process.send_after(self(), :daily, @day)

    Task.start(fn ->
      for owner <- Servers.acts_for(Server.key()), do: reconcile(owner)
      for %{app: artifact} <- Deploy.deployed(), takes?(artifact), installed?(artifact), do: check(artifact)
    end)

    {:noreply, state}
  end

  @doc """
  Installs the app `artifact` for `owner` by `name`, for `by`, who asked, or tries again where it failed to: its
  requirements first. Gives the record it's at once done, with `also`, a line for each requirement it installed too,
  or `{:error, why}` for a move that isn't in the machine.
  """
  def install(owner, name, artifact, by) do
    with :ok <- Installed.named(name),
         {_number, _code, _tag} <- Store.latest(artifact) || {:error, "There's no app #{artifact}"},
         %{} <- move(owner, name, {:install, artifact, by}) do
      case requirements(owner, name, artifact, by) do
        {:ok, also} -> with %{} = record <- installing(owner, name, artifact), do: Map.put(record, "also", also)
        {:error, why} -> failed(owner, name, why)
      end
    end
  end

  # Installs each of the app's requirements that isn't there for `owner`, deepest first, as its own install, or
  # brings it back from their Recently Deleted. Gives a line for each, saying why it's installed too, or `{:error,
  # why}` naming the first that couldn't be.
  defp requirements(owner, name, artifact, by) do
    title = title(artifact, name)
    missing = Requires.missing(owner, artifact, Requires.requires(artifact))
    titles = Map.new(missing, &{&1.name, &1.title}) |> Map.put(name, title)
    # Whoever installs it may add each of them too, or nothing is installed.
    asking = {by || owner, owner}
    closed = Enum.find(missing, &(&1.app && not &1.deleted && opens(asking, &1.app) != :ok))

    if closed,
      do: {:error, "#{title} requires #{closed.title}, which #{elem(asking, 0)} may not add: whoever owns #{closed.app} shares it first."},
      else: install_requirements(owner, missing, titles, title, by)
  end

  defp install_requirements(owner, missing, titles, title, by) do
    Enum.reduce_while(missing, {:ok, []}, fn requirement, {:ok, also} ->
      stage("Installing #{requirement.title}")

      installed =
        cond do
          requirement.app == nil -> {:error, "it isn't in this network, so it can't be installed for #{owner}"}
          requirement.deleted -> restore(owner, requirement.name, {by, owner})
          true -> install(owner, requirement.name, requirement.app, by)
        end

      case installed do
        %{"state" => "installed"} ->
          line = "#{titles[requirement.by] || requirement.by} uses #{requirement.title} #{requirement.why}, so it's installed too."
          {:cont, {:ok, also ++ [line]}}

        other ->
          why = if match?({:error, _}, other), do: elem(other, 1), else: other["why"]
          {:halt, {:error, "#{title} requires #{requirement.title}, which couldn't be installed: #{why}"}}
      end
    end)
  end

  # Fetches, builds and starts what's being installed, whose record is at its first stage. Gives the record it's at.
  defp installing(owner, name, artifact) do
    {number, code, _tag} = Store.latest(artifact)
    trusted = Store.about(artifact)["trusted"] != false
    title = title(artifact, name)
    move(owner, name, {:stage, "fetching"})
    stage("Getting #{title}'s code")

    case Store.version(artifact, code, Servers.runs?(owner), trusted) do
      {:ok, version} ->
        # What it's installed for holds it once it runs; this lets go of it however installing ends.
        try do
          with _ = move(owner, name, {:stage, "building"}),
               _ = stage("Building #{title}"),
               :ok <- Deploy.fits(owner, name, version),
               %{} <- move(owner, name, {:stage, "starting"}) do
            stage("Starting #{title}")
            ran(owner, [{artifact, number, {:ok, version}}])

            case Apps.versions([owner]) do
              %{^name => %{artifact: ^artifact}} -> move(owner, name, :done)
              _ -> failed(owner, name, "It didn't start.")
            end
          else
            {:error, why} -> failed(owner, name, why)
          end
        after
          LoadedVersion.release([version])
        end

      :error ->
        failed(owner, name, "Its latest version doesn't compile here.")
    end
  rescue
    error ->
      Logger.error(Exception.format(:error, error, __STACKTRACE__))
      failed(owner, name, Exception.message(error))
  end

  # A step a person reads while a tool runs as an MCP task: "Installing Google Account", "Getting Gmail's code".
  defp stage(text), do: State.stage(State.id(), text)

  # An app's name where a person reads it.
  defp title(artifact, name) do
    with {_number, code, _tag} <- Store.latest(artifact),
         {:ok, version} <- Store.described(artifact, code),
         title when is_binary(title) <- version.title,
         do: title,
         else: (_ -> name)
  end

  defp failed(owner, name, why) do
    record = move(owner, name, {:fail, why})
    ran(owner)
    record
  end

  @doc """
  Installs for `owner` what the apps that run for them require and isn't there, as their own installs: what a
  app's newer version requires that the one before didn't. A server that takes the requirement's install does
  it, as any of theirs that's told what changed. Gives a line for each, or `{:error, why}` naming one that couldn't be.
  """
  def required(owner) do
    for {name, version} <- Apps.versions([owner]),
        Requires.missing(owner, version.artifact, version.requires) != [],
        Servers.runs?(owner) or (not version.trusted and Servers.here?(owner)),
        reduce: {:ok, []} do
      {:ok, also} ->
        with {:ok, more} <- requirements(owner, name, version.artifact, nil), do: {:ok, also ++ more}

      failed ->
        failed
    end
  end

  @doc """
  Tries again what failed of the app `name` for `owner` (Try Again): installing it, or its update. Gives the
  record of their install, or `{:error, why}`.
  """
  def retry(owner, name) do
    case Installed.read(owner)[name] do
      %{"state" => "not_installed", "app" => artifact} = record ->
        install(owner, name, artifact, record["by"])

      %{"app" => artifact} = record ->
        with nil <- update(artifact), do: Installed.read(owner)[name] || record

      nil ->
        {:error, "#{name} isn't installed for #{owner}."}
    end
  end

  @doc """
  Uninstalls an app for `owner`: it stops running for them, and waits in Recently Deleted, with what it keeps, for
  30 days. Where other apps installed for them are required by it, it's refused saying so, unless whoever asked
  `confirm`s that they go too: then they all go to Recently Deleted, as one. Refused where it's the network's launcher. The apps that observe `uninstalled` are told once, with each
  that went (`with`), so one that keeps what's deleted takes what they kept and made for them along, as one item.
  `asked` is who asked.
  """
  def uninstall(owner, name, {actor, _subject}, confirm \\ false) do
    versions = Map.new(Apps.versions([owner]))

    required_by = Requires.required_by(owner, name)

    cond do
      required_by != [] and confirm != true ->
        {:error, "#{removed_too(versions, name, required_by)}\n\nTo uninstall them all, uninstall #{name} again with confirm, once the person has said yes."}

      true ->
        with :ok <- Enum.find_value([name | required_by], :ok, &with(:ok <- last_launcher(owner, &1), do: nil)),
             :ok <- Enum.find_value(required_by, :ok, &with(%{} <- move(owner, &1, {:uninstall, %{"deleted_with" => name}}), do: nil)),
             %{} = record <- move(owner, name, {:uninstall, %{"with" => required_by}}) do
          ran(owner)
          records = Installed.read(owner)
          went = for other <- required_by, do: %{"name" => other, "app" => records[other]["app"]}
          about = %{"for" => owner, "name" => name, "app" => record["app"], "with" => went}
          Hooks.observe("uninstalled", about, {actor, owner})
          record
        end
    end
  end

  # What a person reads before what's required by an app goes with it: "Gmail, Calendar and Drive use Google Account
  # to reach your Google data. They can't work without it, so they'll be removed too."
  defp removed_too(versions, name, required_by) do
    title = &((versions[&1] && versions[&1].title) || &1)
    reasons = Enum.uniq(for other <- required_by, why = versions[other] && versions[other].requires[name], do: why)
    reason = if length(reasons) == 1, do: " " <> hd(reasons), else: ""

    case required_by do
      [one] -> "#{title.(one)} uses #{title.(name)}#{reason}. It can't work without it, so it'll be removed too."
      many -> "#{list(Enum.map(many, title))} use #{title.(name)}#{reason}. They can't work without it, so they'll be removed too."
    end
  end

  defp list([one]), do: one
  defp list(many), do: Enum.join(Enum.drop(many, -1), ", ") <> " and " <> List.last(many)

  # A network with no launcher has no way in, so the one it has stays until another version replaces it.
  defp last_launcher(owner, name) do
    with %{^name => version} <- Map.new(Apps.versions([owner])),
         true <- Deploy.launcher?(version) do
      {:error,
       "#{name} is this network's launcher, which every client opens first, and a network with none has no way in. " <>
         "To replace it, deploy another as its new version: ONE_SUBJECT=network one deploy --name #{name} --files-from <folder>"}
    else
      _ -> :ok
    end
  end

  @doc """
  Brings an app back from Recently Deleted for `owner`, as it was, with whatever went there with it. Gives its record.
  The apps that observe `restored` are told once, of the one uninstalled, so one that keeps what's deleted brings
  back what they kept and made for them. `asked` is who asked.
  """
  def restore(owner, name, {actor, _subject}) do
    records = Installed.read(owner)
    first = (records[name] || %{})["deleted_with"] || name
    group = [first | (records[first] || %{})["with"] || []]

    with %{} = record <- move(owner, first, :restore) do
      for other <- group -- [first], match?(%{"state" => "recently_deleted"}, records[other]), do: move(owner, other, :restore)
      ran(owner)
      Hooks.observe("restored", %{"for" => owner, "name" => first, "app" => record["app"]}, {actor, owner})
      Installed.read(owner)[name]
    end
  end

  @doc """
  Removes an app in Recently Deleted for `owner` for good, with what it kept, and whatever went there with it:
  `nil`, or `{:error, why}`.
  """
  def purge(owner, name) do
    records = Installed.read(owner)
    group = [name | (records[name] || %{})["with"] || []]

    Enum.reduce(group, nil, fn each, outcome ->
      if match?(%{"state" => "recently_deleted"}, records[each]) do
        with nil <- move(owner, each, :purge) do
          for blob <- Bucket.list("#{owner}/own/#{each}/"), do: Bucket.remove(blob)
          Installed.unmarked(records[each]["app"], owner)
          outcome
        end
      else
        outcome
      end
    end)
  end

  @doc """
  Moves on what's installed for `owner` that the clock moves, as a server acting for them does once a day: one in
  Recently Deleted for 30 days is removed for good, with what it kept, one installing for longer than it could
  has failed, and one that failed isn't in use for them.
  """
  def reconcile(owner) do
    now = System.system_time(:millisecond)

    for {name, record} <- Installed.read(owner) do
      case record do
        %{"state" => "recently_deleted", "deleted" => deleted} when deleted <= now - @kept ->
          with nil <- purge(owner, name),
               do: Logger.info("#{name} was in Recently Deleted for #{owner} for 30 days, and is removed for good")

        %{"state" => "installing", "at" => at} when at <= now - @partway ->
          failed(owner, name, "It stopped partway, as the server installing it did. Try again.")

        # Its server may have stopped between its failing and saying so in its artifact.
        %{"state" => "not_installed", "app" => artifact} ->
          Installed.unmarked(artifact, owner)

        _ ->
          :ok
      end
    end

    :ok
  end

  @doc """
  Looks at the git source of the app `artifact` for a commit after the one it runs that changed its folder: a
  trusted app takes it at once, and an untrusted one offers it. Gives where its updates are.
  """
  def check(artifact) do
    record = Update.read(artifact)

    cond do
      match?(%{"state" => "updating"}, record) and record["at"] > System.system_time(:millisecond) - @partway ->
        record

      true ->
        if match?(%{"state" => "updating"}, record),
          do: Update.change(artifact, {:fail, "It stopped partway, as the server updating it did. Try again."})

        about = Store.about(artifact)

        with %{"source" => source, "commit" => commit} = from <- about["from"] || :none,
             false <- from["dirty"] == true,
             {:ok, offer} when offer != nil <- newer(source, from["ref"], commit, Path.basename(artifact)),
             %{} = record <- Update.change(artifact, {:newer, offer, about["trusted"] != false, Server.key()}) do
          told(artifact)
          if mine?(record), do: updating(artifact, record), else: record
        else
          {:error, why} ->
            Logger.warning("#{artifact}'s git source wasn't checked: #{why}")
            Update.read(artifact)

          _ ->
            Update.read(artifact)
        end
    end
  end

  # The newest commit at a git source that comes after `commit` and changed the app's folder, with the commits
  # between, or `nil`.
  defp newer(source, ref, commit, name) do
    path = "apps/#{name}"

    with {:ok, copy} <- Git.fetch(source),
         {:ok, newest} <- Git.commit(copy, ref) do
      cond do
        newest == commit ->
          {:ok, nil}

        not Git.ancestor?(copy, commit, newest) ->
          {:ok, nil}

        Git.tree(copy, newest, path) == Git.tree(copy, commit, path) ->
          {:ok, nil}

        true ->
          with {:ok, commits} <- Git.commits(copy, commit, newest, path),
               do: {:ok, %{"commit" => newest, "source" => source, "ref" => ref, "commits" => commits}}
      end
    end
  end

  @doc """
  Updates the app `artifact` to the commit its update offers (Update), or tries one that failed again, for everyone
  who has it. Gives where its updates are once done: `nil` for up to date, or `{:error, why}`.
  """
  def update(artifact) do
    with %{} = record <- Update.change(artifact, {:update, Server.key()}) do
      told(artifact)
      updating(artifact, record)
    end
  end

  # Fetches the commit an update takes, and deploys it as the app's next version, for everyone who has it.
  defp updating(artifact, %{"commit" => commit, "source" => source} = record) do
    whose = Store.whose(artifact)
    name = Path.basename(artifact)
    title = title(artifact, name)

    outcome =
      try do
        stage("Getting #{title}'s code")

        with {:ok, copy} <- Git.fetch(source),
             {:ok, files} <- Git.files(copy, commit, "apps/#{name}") do
          step(artifact, "building")
          stage("Building #{title}")
          from = %{"source" => source, "ref" => record["ref"], "commit" => commit}
          installed? = fn _version -> Apps.app(whose, name) == artifact end
          Deploy.deploy(whose, name, files, from, :stored, installed?)
        end
      rescue
        error -> {:error, Exception.message(error)}
      end

    case outcome do
      {:ok, _number, _warnings} ->
        # What the new version requires that isn't there is installed for each that has it, by their servers.
        for owner <- Enum.uniq([whose | Installed.in_use_for(artifact)]), Servers.here?(owner) do
          with {:error, why} <- required(owner), do: Logger.warning("#{artifact}'s update for #{owner}: #{why}")
        end

        told(artifact, Update.change(artifact, :done))

      {:error, why} -> told(artifact, Update.change(artifact, {:fail, why}))
    end
  end

  defp step(artifact, stage), do: told(artifact, Update.change(artifact, {:stage, stage}))

  @doc """
  Makes the app `artifact` trusted or untrusted, for everyone who has it: it's deployed again from the files it
  was, the other way. `{:ok, number}`, or `{:error, why}`.
  """
  def trust(artifact, trusted) do
    whose = Store.whose(artifact)
    name = Path.basename(artifact)

    with {_number, code, _tag} <- Store.latest(artifact) || {:error, "There's no app #{artifact}"},
         {:ok, number, _warnings} <-
           Deploy.deploy(whose, name, Store.source(artifact, code), :stored, trusted, fn _ -> Apps.app(whose, name) == artifact end),
         do: {:ok, number}
  end

  @doc """
  Whether whoever asked may read an app's code and add it: a `viewer` of it, or a member of a group or the network
  that it's whoever's or installed for, as the apps that validate `reach` say. That's VISION's "what's made is open
  to everyone inside once it's in use": what only its author has is theirs and whoever they share it with. Only its
  code is opened: what it keeps for anyone is never the app's artifact. Nothing is asked of what's their own.
  `:ok`, or `{:refuse, why}`.
  """
  def opens({actor, _subject} = principal, app) do
    reach = fn relation, artifact -> Hooks.validate("reach", %{"who" => actor, "relation" => relation, "artifact" => artifact}, principal) end

    with false <- artifact_of?(actor, app),
         {:refuse, why} <- reach.("viewer", app) do
      in_use =
        for of <- [OneCore.User.owner(app) | Installed.in_use_for(app)],
            of == @network or match?("groups/" <> _, of),
            uniq: true,
            do: of

      member = &(Hooks.validate("reach", %{"who" => actor, "relation" => "member", "artifact" => &1}, {actor, &1}) == :ok)
      if Enum.any?(in_use, member), do: :ok, else: {:refuse, why}
    else
      true -> :ok
      :ok -> :ok
    end
  end

  defp artifact_of?(nil, _app), do: false
  defp artifact_of?(actor, app), do: app == actor or String.starts_with?(app, actor <> "/")

  # Whether this server takes an update it moved to: only the one that did.
  defp mine?(record), do: record["state"] == "updating" and record["by"] == Server.key()

  @doc """
  Whether this server takes the app `artifact`'s updates: it acts for whoever it is, and runs their apps where
  it's trusted.
  """
  def takes?(artifact) do
    whose = Store.whose(artifact)
    Servers.here?(whose) and (Store.about(artifact)["trusted"] == false or Servers.runs?(whose))
  end

  defp installed?(artifact),
    do: Installed.installed_for(artifact) != [] or Apps.app(Store.whose(artifact), Path.basename(artifact)) == artifact

  # What runs for `owner` changed: it runs here, with what's loaded already, its clients are told what each of their
  # apps runs at now, and their other servers look.
  defp ran(owner, loaded \\ []) do
    Apps.sync(owner, loaded)
    Server.changed([owner])
    Server.notify_servers([owner])
  end

  # Where an app's updates are moved, told to whoever has it installed, so their clients ask again.
  defp told(artifact, record \\ nil) do
    owners = Enum.uniq([Store.whose(artifact) | Installed.installed_for(artifact)])
    Server.changed(owners)
    Server.notify_servers(owners, "changed")
    record
  end

  # A move of an install's record, told to whoever watches what's installed for `owner`: gives the record it's at,
  # `nil`, or `{:error, why}`.
  defp move(owner, name, event) do
    with record when not is_tuple(record) <- Installed.change(owner, name, event) do
      Server.changed([owner])
      Server.notify_servers([owner], "changed")
      record
    end
  end
end
