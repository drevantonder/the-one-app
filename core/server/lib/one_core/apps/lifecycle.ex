defmodule OneCore.Apps.Lifecycle do
  @moduledoc """
  What moves an install through its life (`OneCore.Apps.Installed`) and an app through its updates
  (`OneCore.Apps.Update`): installing, a stage at a time; uninstalling to Recently Deleted, and restoring or removing
  for good as Recently Deleted does; and taking a newer version of an app, or switching whether it's trusted, for everyone who
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

  Recently Deleted is the app soft-delete's, as for everything a person deletes: hearing `uninstalled`, it keeps what
  the app kept and made for them as one item, and its clock and its Restore are the only ones. Its Restore brings the
  app back here (`restore/2`), and purging the item removes it here for good (`purge/2`). A network without it
  keeps what was uninstalled in Recently Deleted.

  Each install and update is the MCP task that runs it (`OneCore.Tasks.within/3`), which its record names: however
  the task ends, it moves the record. Done or failed, the run moves it itself; where its server stopped, or started
  again, which loses what it ran, whoever finds the task over moves it (`settle/0`): this server as it starts, and
  each time the network's edge says a server came or went, and Try Again.

  It's a GenServer only for the clock and for that: once a day it looks for newer versions, for whoever this server
  acts for; and it settles what lost its task as it starts and as servers come and go, one settle at a time: a server
  coming or going while one runs has it run once more after, however many did, as each reads what's there anew.
  During a release servers say they changed every few seconds, and each settle reads every app's updates.
  """
  use GenServer
  require Logger
  alias OneCore.{Bucket, Hooks, Server, Servers, Tasks}
  alias OneCore.Call.State
  alias OneCore.Apps
  alias OneCore.Apps.{Deploy, Git, Installed, LoadedVersion, Requires, Store, Update, Version}

  @network "network"
  @day :timer.hours(24)
  # What an install or update whose task ended before it did says, where its task doesn't say why.
  @stopped "It stopped partway. Try again."

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @impl true
  def init(nil) do
    Process.send_after(self(), :daily, :timer.minutes(5))
    Registry.register(OneCore.Waiting, :seen, nil)
    {:ok, %{settling: nil, again: false}, {:continue, :settle}}
  end

  # Starting, what this server ran before is lost, and a server that came or went may have lost what it ran.
  @impl true
  def handle_continue(:settle, state), do: {:noreply, settling(state)}

  defp settling(state) do
    {settling, _ref} = spawn_monitor(&settle/0)
    %{state | settling: settling}
  end

  @impl true
  def handle_info(:seen, %{settling: nil} = state), do: {:noreply, settling(state)}
  def handle_info(:seen, state), do: {:noreply, %{state | again: true}}

  def handle_info({:DOWN, _ref, :process, settling, _why}, %{settling: settling, again: again} = state) do
    state = %{state | settling: nil, again: false}
    {:noreply, if(again, do: settling(state), else: state)}
  end

  def handle_info(:daily, state) do
    Process.send_after(self(), :daily, @day)

    Task.start(fn ->
      for owner <- Servers.here(), do: reconcile(owner)
      for %{app: artifact} = app <- Deploy.deployed(), takes?(app), installed?(artifact), do: check(artifact)
    end)

    {:noreply, state}
  end

  # The apps a network starts with, which whoever makes its first owner installs for it, each after the ones it
  # uses: database and encryption keep what the others keep, permissions decides who may do what, sign-in says whose
  # each device is, sandbox runs untrusted apps, vault keeps accounts and secrets, soft-delete keeps what's
  # deleted, notifications tells people things, and home-screen is the launcher every client opens first.
  @starting ~w(database encryption permissions sign-in sandbox vault soft-delete notifications home-screen)

  # Where the core is published, whose releases a new network's first server is installed from, with the apps at the
  # same commit.
  @core "https://github.com/ptc-au/the-one-app"

  @doc """
  Makes a new network of `who`, its first owner, on its first server, installed from the core's releases, as
  `one serve on` installs it (`clients/cli/src/serving.rs`): every app of the core at this server's commit is deployed
  for the network (`update_core_apps/0`), and the ones a network starts with are installed (`make_first_owner/1`). A
  network that has a first owner has nothing done: `:ok`, or `{:error, why}`.
  """
  def found(who) do
    if OneCore.User.first_owner() == nil and Servers.here?("network") do
      with :ok <- update_core_apps(), do: make_first_owner(who)
    else
      :ok
    end
  end

  @doc """
  Takes the network's core apps to this server's build, as each of its servers starts on one: every app of the core
  at the build's commit is deployed for the network from git, as their author publishes them, and taken by whatever
  has it installed, as an update is (`updating/2`). So a release of the core is a release of its apps to every
  network, with nothing told which networks there are. Only a server on the network's latest build does it, and
  only for an app that isn't deployed yet, was deployed with no commit, or was deployed from a commit this one has
  whose folder changed since: one deployed from a newer commit is never taken back. `:ok`, or `{:error, why}`.
  """
  def update_core_apps do
    [_version, metadata] = String.split(OneCore.Builds.build(), "+", parts: 2)
    [_count, short | _] = String.split(metadata, ".")
    latest = OneCore.Builds.latest()["server-code"]["build"]

    with true <- Servers.here?("network") and latest in [nil, OneCore.Builds.build()],
         {:ok, copy} <- Git.fetch(@core, true),
         {:ok, commit} <- Git.commit(copy, short),
         # One of the network's servers does it for each build, and the others leave it to that one: two deploying
         # the same app at once, one of them was refused. It's the claim the network's apps are built again under
         # (`Deploy.rebuild/0`), which the same server does next, so neither races the other.
         true <- Servers.claims?("network/rebuild", OneCore.Builds.build()),
         {:ok, files} <- Git.files(copy, commit, "apps") do
      from = %{"source" => @core, "ref" => nil, "commit" => commit}
      folders = Enum.group_by(files, &hd(Path.split(elem(&1, 0))), fn {path, data} -> {Path.relative_to(path, hd(Path.split(path))), data} end)

      for {name, files} <- folders, behind?(copy, name, commit) do
        installed? = fn _version -> Apps.app("network", name) == "network/apps/#{name}" end

        case Deploy.deploy("network", name, Map.new(files), from, true, installed?) do
          {:ok, number, _warnings} -> Logger.info("#{name} is deployed for the network at the core's #{short}, as version #{number}")
          failed -> Logger.error("#{name} wasn't deployed for the network at the core's #{short}: #{inspect(failed)}")
        end
      end

      # What the new versions require that isn't there is installed, as for an update.
      with {:error, why} <- required("network"), do: Logger.warning("The core's apps at #{short}: #{why}")
      :ok
    else
      false -> :ok
      {:error, why} -> {:error, "The core's apps weren't taken to #{short}: #{why}"}
    end
  end

  # Whether the network's core app `name` is behind the core at `commit`, as `update_core_apps/0` says.
  defp behind?(copy, name, commit) do
    case Store.about("network/apps/#{name}") do
      about when about == %{} -> true
      %{"from" => %{"commit" => was}} when is_binary(was) ->
        was != commit and Git.ancestor?(copy, was, commit) and Git.tree(copy, was, "apps/#{name}") != Git.tree(copy, commit, "apps/#{name}")
      _deployed_with_no_commit -> true
    end
  end

  @doc """
  Makes the user `who` the network's first owner, which a network gets once: `:ok`, or `{:error, why}`. Then the
  apps a network starts with are installed for it as the first owner installs any, from what the network has
  deployed of them (`network/apps/<name>`), and only then are the apps that validate `owner` asked, so the one
  that keeps who may do what is there to write it down, and the one that signs people in to say where they join.
  Doing it again for the same user finishes what an earlier try left undone.
  """
  def make_first_owner(who) do
    with :ok <- OneCore.User.user(who) do
      wrote = Bucket.write("network/owner", who)
      Servers.invalidate()

      if wrote == :ok or OneCore.User.first_owner() == who do
        Server.notify_servers(["network"])
        failed = for name <- @starting, why = starting(name, who), do: "#{name}: #{why}"

        case Hooks.validate("owner", %{"who" => who}, {nil, "network"}) do
          {:refuse, why} ->
            {:error, why}

          :ok when failed != [] ->
            {:error,
             "#{who} is the network's first owner, but these weren't installed for it: " <> Enum.join(failed, "; ")}

          :ok ->
            :ok
        end
      else
        {:error, "The network's first owner is #{OneCore.User.first_owner()}"}
      end
    else
      {:error, why} -> {:error, "The network's first owner is a user. #{why}"}
    end
  end

  # Installs one of the apps a network starts with for it, unless it's there: `nil`, or why it wasn't.
  defp starting(name, who) do
    case Installed.read("network")[name] do
      %{"state" => "installed"} ->
        nil

      _ ->
        case install("network", name, "network/apps/#{name}", who) do
          %{"state" => "installed"} -> nil
          %{"state" => state} = record -> record["why"] || "it's #{state}"
          {:error, why} -> why
        end
    end
  end

  @doc """
  Installs the app `artifact` for `owner` by `name`, for `by`, who asked, or tries again where it failed to: its
  requirements first. Gives the record it's at once done, with `also`, a line for each requirement it installed too,
  or `{:error, why}` for a move that isn't in the machine.
  """
  def install(owner, name, artifact, by) do
    doing({:install, owner, name}, owner, "install", fn task ->
      with :ok <- Installed.named(name),
           {_number, _code, _tag} <- Store.latest(artifact) || {:error, "There's no app named #{Path.basename(artifact)}"},
           %{} <- move(owner, name, {:install, artifact, by, task}) do
        case requirements(owner, name, artifact, by) do
          {:ok, also} -> with %{} = record <- installing(owner, name, artifact), do: Map.put(record, "also", also)
          {:error, why} -> failed(owner, name, why)
        end
      end
    end)
  end

  # Installs each of the app's requirements that isn't there for `owner`, deepest first, as its own install. Gives a
  # line for each, saying why it's installed too, or `{:error, why}` naming the first that couldn't be: one in their
  # Recently Deleted is restored there, with what it kept, never installed again without it.
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
          requirement.deleted -> {:error, "it's in Recently Deleted for #{owner}: restore it there first"}
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
    trusted = Store.trusted?(artifact)
    title = title(artifact, name)
    move(owner, name, {:stage, "fetching"})
    stage("Getting #{title}'s code")

    case Store.version(artifact, code, Apps.loads?(owner), trusted) do
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
              %{^name => %{artifact: ^artifact}} ->
                with %{} = record <- move(owner, name, :done) do
                  supersede(owner, artifact)
                  record
                end

              _ ->
                failed(owner, name, "It didn't start.")
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
  catch
    # What it waits on exiting (`Apps.sync/2` does), which would otherwise end it with its record left installing.
    :exit, why ->
      Logger.error(Exception.format(:exit, why, __STACKTRACE__))
      failed(owner, name, Exception.format_exit(why))
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
        Apps.runs?(owner, version.trusted),
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
    # One whose task is over has failed, and is tried again; one that's still installing is refused, saying so.
    case settled(owner, name, Installed.read(owner)[name]) do
      %{"state" => "not_installed", "app" => artifact} = record ->
        install(owner, name, artifact, record["by"])

      %{"state" => "installing", "app" => artifact} = record ->
        move(owner, name, {:install, artifact, record["by"], record["task"]})

      %{"app" => artifact} = record ->
        case settled(artifact) do
          %{"state" => "failed"} -> with nil <- update(artifact), do: Installed.read(owner)[name] || record
          _ -> {:error, "Nothing of #{name} failed for #{owner}, so there's nothing to try again."}
        end

      nil ->
        {:error, "#{name} isn't installed for #{owner}."}
    end
  end

  # The task an install or update's record names, as it was kept once it ended, or `nil` while it runs. A record from
  # before records named their task was cut short by the restart that updated its server.
  defp over(nil), do: %{}
  defp over(task), do: Tasks.over(task)

  # Why a task ended before what it ran did: what it says where it failed or was cancelled.
  defp why(%{"status" => status, "statusMessage" => why}) when status in ["failed", "cancelled"], do: why
  defp why(_task), do: @stopped

  # An install's record, with one that's installing once its task is over failed, saying why.
  defp settled(owner, name, %{"state" => "installing"} = record) do
    case over(record["task"]) do
      nil -> record
      task -> with {:error, _why} <- failed(owner, name, why(task)), do: Installed.read(owner)[name]
    end
  end

  defp settled(_owner, _name, record), do: record

  @doc """
  Settles every install and update whose task is over, for whoever this server acts for: as it starts, which loses
  what it ran before, and when a server came or went.
  """
  def settle do
    for owner <- Servers.here(), {name, record} <- Installed.read(owner), do: settled(owner, name, record)
    for %{app: artifact} = app <- Deploy.deployed(), takes?(app), do: settled(artifact)
    :ok
  end

  # Installing or updating holds a lock on what it does, on this server, while it does it, and is a task for
  # `subject` as the tool `tool`, which `fun` is given. `{:error, why}` where a process here holds it already.
  defp doing(what, subject, tool, fun) do
    case :global.trans({{__MODULE__, what}, self()}, fn -> Tasks.within(subject, tool, fun) end, [node()], 0) do
      :aborted -> {:error, "It's being installed or updated here already. Wait until it's done."}
      done -> done
    end
  end

  @doc """
  Uninstalls an app for `owner`: it stops running for them, and waits in Recently Deleted, with what it keeps, until
  that restores or purges it. Where other apps installed for them are required by it, it's refused saying so, unless whoever asked
  `confirm`s that they go too: then they all go to Recently Deleted, as one. Refused where it's the network's launcher. The apps that observe `uninstalled` are told once, with each
  that went (`with`) and the artifacts that go with them, what they kept and made (`artifacts`, `kept/2`), so one that
  keeps what's deleted takes those along, as one item. `asked` is who asked.

  One of someone's own that the network installs for everyone too leaves what it made for them where it is, and where
  the network's keeps each person's data apart, what it kept too: it's what the network's reads for them now. One
  whose data the network's wouldn't read, since it keeps everyone's together, is refused while it keeps anything,
  saying why.
  """
  def uninstall(owner, name, {actor, _subject}, confirm \\ false) do
    versions = Map.new(Apps.versions([owner]))

    required_by = Requires.required_by(owner, name)
    group = [name | required_by]

    cond do
      required_by != [] and confirm != true ->
        {:error, "#{removed_too(versions, name, required_by)}\n\nTo uninstall them all, uninstall #{name} again with confirm, once the person has said yes."}

      why = Enum.find_value(group, &stranded(owner, versions, &1)) ->
        {:error, why}

      true ->
        artifacts = Enum.uniq(Enum.flat_map(group, &kept(owner, &1)))

        with :ok <- Enum.find_value(group, :ok, &with(:ok <- last_launcher(owner, &1), do: nil)),
             :ok <- Enum.find_value(required_by, :ok, &with(%{} <- move(owner, &1, {:uninstall, %{"deleted_with" => name}}), do: nil)),
             %{} = record <- move(owner, name, {:uninstall, %{"with" => required_by}}) do
          ran(owner)
          records = Installed.read(owner)
          went = for other <- required_by, do: %{"name" => other, "app" => records[other]["app"]}
          about = %{"for" => owner, "name" => name, "app" => record["app"], "with" => went, "artifacts" => artifacts}
          Hooks.observe("uninstalled", about, {actor, owner})
          record
        end
    end
  end

  @doc """
  What the app installed for `owner` as `name` kept and made that goes with it, by the names of the artifacts: those of
  its own artifacts that keep anything (`owns/2`), and each it made for `owner`. What it made for them stays where the
  same name is installed for everyone too, which makes and reads them at the same names for them now.
  """
  def kept(owner, name) do
    made =
      if beyond(owner, name),
        do: [],
        else:
          for(
            blob <- Bucket.list("#{owner}/made/#{name}/"),
            [artifact] <- [Regex.run(~r{^[^/]+(?:/[^/]+)?/made/[^/]+/[^/]+}, blob)],
            uniq: true,
            do: artifact
          )

    made ++ for(own <- owns(owner, name), Bucket.list(own <> "/") != [], do: own)
  end

  @doc """
  The own artifacts that the app installed for `owner` as `name` keeps and nothing else reads, by their names, which go
  to Recently Deleted with it and are removed for good with it: whoever it was installed for's, unless the same name
  installed for everyone keeps each person's data apart (`OneCore.Apps.Calls.whose/3`), and so reads it for them now;
  and where it keeps each person's apart, the own artifact of each user or group with something in theirs, which is
  all that says who keeps something with it, and hasn't it installed for themselves.
  """
  def owns(owner, name) do
    mine = if match?({_at, %Version{own: :subject}}, beyond(owner, name)), do: [], else: ["#{owner}/own/#{name}"]

    theirs =
      for folder <- Bucket.folders("users/") ++ Bucket.folders("groups/"),
          who = String.trim_trailing(folder, "/"),
          who != owner,
          Bucket.list("#{who}/own/#{name}/") != [],
          not Map.has_key?(Map.new(Apps.versions([who])), name),
          do: "#{who}/own/#{name}"

    mine ++ theirs
  end

  # What `owner` uses as `name` once theirs goes: the install of it further along whose apps they find (the network's),
  # as who it was installed for and its version, or `nil`.
  defp beyond(owner, name) do
    Enum.find_value(Apps.chain(owner) -- [owner], fn at ->
      with %Version{} = version <- Map.new(Apps.versions([at]))[name], do: {at, version}
    end)
  end

  # Why uninstalling `name` for `owner` would leave what it keeps where nothing reads it, or `nil`: they'd go on using
  # the same name installed for everyone, which keeps everyone's data together, so what theirs kept would only wait in
  # Recently Deleted.
  defp stranded(owner, versions, name) do
    with {at, %Version{own: :installed}} <- beyond(owner, name),
         [_ | _] <- Bucket.list("#{owner}/own/#{name}/") do
      title = (versions[name] && versions[name].title) || name

      "This #{title} keeps data that the #{title} installed for everyone doesn't read, since that one keeps " <>
        "everyone's together. So it isn't uninstalled: its data would only wait in Recently Deleted.\n\n" <>
        "#{at}'s #{name} says its data is #{at}'s (own/0 is :installed), so #{owner}/own/#{name} isn't what it " <>
        "reads for #{owner}. Keep this one, or move what it keeps into #{at}'s first."
    else
      _ -> nil
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
  Brings an app back from Recently Deleted for `owner`, as it was, with whatever went there with it, as Recently
  Deleted's Restore does, which brings back what it kept and made. Gives its record, or `{:error, why}`.
  """
  def restore(owner, name) do
    records = Installed.read(owner)
    first = (records[name] || %{})["deleted_with"] || name
    group = [first | (records[first] || %{})["with"] || []]

    with %{} <- move(owner, first, :restore) do
      for other <- group -- [first], match?(%{"state" => "recently_deleted"}, records[other]), do: move(owner, other, :restore)
      ran(owner)
      Installed.read(owner)[name]
    end
  end

  @doc """
  Removes an app in Recently Deleted for `owner` for good, with what it kept, and whatever went there with it, as
  Recently Deleted purging it does: `nil`, or `{:error, why}`.
  """
  def purge(owner, name) do
    records = Installed.read(owner)
    group = [name | (records[name] || %{})["with"] || []]

    Enum.reduce(group, nil, fn each, outcome ->
      if match?(%{"state" => "recently_deleted"}, records[each]) do
        with nil <- move(owner, each, :purge) do
          for own <- owns(owner, each), blob <- Bucket.list(own <> "/"), do: Bucket.remove(blob)
          Installed.unmarked(records[each]["app"], owner)
          outcome
        end
      else
        outcome
      end
    end)
  end

  @doc """
  Keeps one copy of an app (`OneCore.Apps.Store.same?/2`). Once the app `artifact` is installed for `owner`, a group or
  the network, each other copy of the same app that only those `owner` covers have, its members or everyone, is
  superseded by it (`OneCore.Apps.Installed`'s Superseded): where the install for `owner` is one their calls find
  after their own, the network's, each of the copy's records is in it now, and it acts for them; where it isn't, a
  group's, which runs only for the group, the record moves to `artifact`. Then the copy, installed for nobody, goes to
  Recently Deleted (the hook `superseded`), so it comes back as it was if it's restored. What each keeps with it stays
  theirs, where it is, since it's kept by the name the app is installed by, not its artifact. A copy someone outside
  `owner` has stays; so does one whose record is installing, until it's done, and one whose data `artifact` wouldn't
  read, since it keeps everyone's together (`own/0` is `:installed`). Gives the copies superseded.
  """
  def supersede(owner, artifact) do
    {_at, version} = Enum.find(Apps.versions([owner]), {nil, nil}, fn {_name, version} -> version.artifact == artifact end)

    copies =
      if version && (owner == @network or String.starts_with?(owner, "groups/")),
        do: for(%{app: copy} <- Deploy.deployed(), copy != artifact, Store.same?(copy, artifact), do: copy),
        else: []

    for copy <- copies,
        holders = Enum.uniq([Store.whose(copy) | Installed.installed_for(copy)]),
        Enum.all?(holders, &covers?(owner, &1)),
        records = for(holder <- holders, {name, %{"app" => ^copy}} <- Installed.read(holder), do: {holder, name}),
        not Enum.any?(records, fn {holder, name} -> stranded?(owner, holder, name, version) end),
        moved = for({holder, name} <- records, do: superseded(owner, version, holder, name)),
        not Enum.any?(moved, &match?({:error, _why}, &1)) do
      # What says it's installed for someone with no record of it any more, as a move from before moves unmarked
      # left, goes, so it's installed for nobody.
      for holder <- holders, do: Installed.unmarked(copy, holder)
      Hooks.observe("superseded", %{"app" => copy, "by" => artifact}, {nil, Store.whose(copy)})
      copy
    end
  end

  # Moves `holder`'s record of `name` into the install of the same app for `owner`, or to its artifact.
  defp superseded(owner, version, holder, name) do
    inside = owner in Apps.chain(holder)

    with record when not is_tuple(record) <- move(holder, name, {:superseded, version.artifact, inside}) do
      ran(holder)
      record
    end
  end

  # Whether what `holder` kept with their copy is what the install for `owner` wouldn't read for them: it keeps
  # everyone's together, as its own.
  defp stranded?(owner, holder, name, version),
    do: owner in Apps.chain(holder) and version.own == :installed and Bucket.list("#{holder}/own/#{name}/") != []

  # Whether what's installed for `owner` is for `who` too: the network is for everyone, and a group for its members.
  defp covers?(owner, owner), do: true
  defp covers?(@network, _who), do: true

  defp covers?("groups/" <> _ = group, "users/" <> _ = who),
    do: Hooks.validate("reach", %{"who" => who, "relation" => "member", "artifact" => group}, {nil, group}) == :ok

  defp covers?(_owner, _who), do: false

  @doc """
  Moves on what's installed for `owner` that the clock moves, as a server acting for them does once a day: one whose
  task is over has failed, one that failed isn't in use for them, and one installed supersedes the narrower copies of
  itself (`supersede/2`).
  """
  def reconcile(owner) do
    for {name, record} <- Installed.read(owner) do
      case record do
        %{"state" => "installing"} ->
          settled(owner, name, record)

        %{"state" => "installed", "app" => artifact} ->
          supersede(owner, artifact)

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
    case settled(artifact) do
      %{"state" => "updating"} = record ->
        record

      _ ->
        about = Store.about(artifact)

        with %{"source" => source, "commit" => commit} = from <- about["from"] || :none,
             false <- from["dirty"] == true,
             {:ok, offer} when offer != nil <- newer(source, from["ref"], commit, Path.basename(artifact), Store.trusted?(about)),
             %{} = record <-
               doing({:update, artifact}, Store.whose(artifact), "apps_update", fn task ->
                 with %{} = record <- Update.change(artifact, {:newer, offer, Store.trusted?(about), Server.key(), task}) do
                   told(artifact)
                   if mine?(record), do: updating(artifact, record), else: record
                 end
               end) do
          record
        else
          {:error, why} ->
            Logger.warning("#{artifact}'s git source wasn't checked: #{why}")
            Update.read(artifact)

          # Nothing at the source is newer than what runs. An offer or a failure still kept is of a commit it has gone
          # past since, as a deploy from a later one does, and is no update.
          {:ok, nil} ->
            case Update.read(artifact) do
              %{"state" => state} when state != "updating" -> told(artifact, Update.change(artifact, :current))
              record -> record
            end

          _ ->
            Update.read(artifact)
        end
    end
  end

  # Where the app's updates are, with one whose task is over ended: done where the version it took went live before
  # its task ended, and failed, saying why, where it didn't.
  defp settled(artifact) do
    record = Update.read(artifact)

    with %{"state" => "updating"} <- record,
         %{} = task <- over(record["task"]) do
      if (Store.about(artifact)["from"] || %{})["commit"] == record["commit"],
        do: told(artifact, Update.change(artifact, :done)),
        else: told(artifact, Update.change(artifact, {:fail, why(task)}))
    else
      _ -> record
    end
  end

  # The newest commit at a git source that comes after `commit` and changed the app's folder, with the commits
  # between, or `nil`.
  defp newer(source, ref, commit, name, trusted) do
    path = "apps/#{name}"

    with {:ok, copy} <- Git.fetch(source, trusted),
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
    doing({:update, artifact}, Store.whose(artifact), "apps_update", fn task ->
      with %{} = record <- Update.change(artifact, {:update, Server.key(), task}) do
        told(artifact)
        updating(artifact, record)
      end
    end)
  end

  # Fetches the commit an update takes, and deploys it as the app's next version, for everyone who has it.
  defp updating(artifact, %{"commit" => commit, "source" => source} = record) do
    whose = Store.whose(artifact)
    name = Path.basename(artifact)
    title = title(artifact, name)

    outcome =
      try do
        stage("Getting #{title}'s code")

        with {:ok, copy} <- Git.fetch(source, Store.trusted?(Store.about(artifact))),
             {:ok, files} <- Git.files(copy, commit, "apps/#{name}") do
          step(artifact, "building")
          stage("Building #{title}")
          from = %{"source" => source, "ref" => record["ref"], "commit" => commit}
          installed? = fn _version -> Apps.app(whose, name) == artifact end
          Deploy.deploy(whose, name, files, from, :stored, installed?)
        end
      rescue
        error -> {:error, Exception.message(error)}
      catch
        :exit, why -> {:error, Exception.format_exit(why)}
      end

    case outcome do
      {:ok, _number, _warnings} ->
        # What the new version requires that isn't there is installed for each that has it, by their servers.
        for owner <- Enum.uniq([whose | Installed.in_use_for(artifact)]), Servers.here?(owner) do
          with {:error, why} <- required(owner), do: Logger.warning("#{artifact}'s update for #{owner}: #{why}")
        end

        told(artifact, Update.change(artifact, :done))

      {:error, why} -> told(artifact, Update.change(artifact, {:fail, why}))
      {:error, why, _details} -> told(artifact, Update.change(artifact, {:fail, why}))
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

    with {_number, code, _tag} <- Store.latest(artifact) || {:error, "There's no app named #{Path.basename(artifact)}"},
         {:ok, number, _warnings} <-
           Deploy.deploy(whose, name, Store.source(artifact, code), :stored, trusted, fn _ -> Apps.app(whose, name) == artifact end) do
      {:ok, number}
    else
      {:error, why, _details} -> {:error, why}
      other -> other
    end
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
  it's trusted. An app as everyone's deployed apps list it (`OneCore.Apps.Deploy.deployed/0`) says whether it's
  trusted, which isn't read again.
  """
  def takes?(%{app: artifact, trusted: trusted}), do: Apps.runs?(Store.whose(artifact), trusted)
  def takes?(artifact), do: takes?(%{app: artifact, trusted: Store.trusted?(artifact)})

  defp installed?(artifact),
    do: Installed.installed_for(artifact) != [] or Apps.app(Store.whose(artifact), Path.basename(artifact)) == artifact

  # What runs for `owner` changed: it runs here, with what's loaded already, its clients are told what each of their
  # apps runs at now, and their other servers look.
  defp ran(owner, loaded \\ []) do
    Apps.sync(owner, loaded)
    Apps.changed([owner])
    Server.notify_servers([owner])
  end

  # Where an app's updates are moved, told to whoever has it installed, so their clients ask again.
  defp told(artifact, record \\ nil) do
    owners = Enum.uniq([Store.whose(artifact) | Installed.installed_for(artifact)])
    Apps.changed(owners)
    Server.notify_servers(owners, "changed")
    record
  end

  # A move of an install's record, told to whoever watches what's installed for `owner`: gives the record it's at,
  # `nil`, or `{:error, why}`.
  defp move(owner, name, event) do
    with record when not is_tuple(record) <- Installed.change(owner, name, event) do
      Apps.changed([owner])
      Server.notify_servers([owner], "changed")
      record
    end
  end
end
