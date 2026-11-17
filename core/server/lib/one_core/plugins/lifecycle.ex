defmodule OneCore.Plugins.Lifecycle do
  @moduledoc """
  What moves a plugin through its life for an owner, as `OneCore.Plugins.Installed` keeps it: installing it,
  updating it and switching its trust, a stage at a time (`fetching` its code, `building` it, `starting` it), and
  uninstalling, restoring, and removing it for good 30 days after.

  Newer versions come from two places. A newer version someone deployed: whoever deployed it runs it at once, as it's
  theirs (`OneCore.Plugins.Deploy`), and everyone else who has it sees an update. And a new commit at the git source
  of a plugin of the owner's own, which servers acting for them look for once a day (`check/1`), and whenever a person
  asks (the tool `plugins_check`). A plugin the server ships updates with the server, and never shows an update.

  An update is taken without asking only where all of these hold: the owner's automatic updates are on, the plugin is
  trusted, the newer version requires no plugin the one running doesn't, and it comes from the network's own plugins or the owner's
  own repos. Any other waits for someone who may install for them. Each is decided again when it runs, and a failed
  update leaves the version that ran running.

  Where a plugin comes from decides those, and whether it's trusted when it's installed: the network's own plugins
  (`network/plugins/…`) and the owner's own (what they deployed, and their repos in the network, `one::<owner>/…`)
  are trusted, and a stranger's (another user's or group's plugin, or one `one install` fetched from any other git
  repo, which it installs untrusted) is untrusted until someone who may install for them says it's trusted.

  It's a GenServer only for the clock: once a day it checks the git sources, and removes for good what's been in
  Recently Deleted for 30 days, for whoever this server acts for.
  """
  use GenServer
  require Logger
  alias OneCore.{Bucket, Sandbox, Server, Servers}
  alias OneCore.Plugins
  alias OneCore.Plugins.{Deploy, Git, Installed, Store, Version}

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
      for owner <- Servers.acts_for(Server.key()) do
        reconcile(owner)
        check(owner)
      end
    end)

    {:noreply, state}
  end

  @doc """
  Installs the plugin `artifact` for `owner` by `name`, or tries again where it failed to: `trusted` as given, or as
  where it comes from says where it's `nil`. Gives the record it's at once done, or `{:error, why}` for a move that
  isn't in the machine.
  """
  def install(owner, name, artifact, trusted) do
    trusted = if trusted == nil, do: trusted?(owner, artifact), else: trusted

    with :ok <- Installed.named(name),
         {number, _code, _tag} <- Store.latest(artifact) || {:error, "There's no plugin #{artifact}"},
         %{} <- move(owner, name, {:install, artifact, trusted}) do
      run(owner, name, fn -> version(owner, name, artifact, number, trusted) end)
    end
  end

  @doc """
  Installs a plugin the server ships for `owner`, where another they install uses it: at once, as it updates with
  the server. `:ok`, or `{:error, why}`.
  """
  def bring(owner, name, artifact) do
    with {number, _code, _tag} <- Store.latest(artifact) || {:error, "There's no plugin #{artifact}"},
         %{} <- move(owner, name, {:deployed, artifact, number, true, nil}) do
      ran(owner)
      :ok
    end
  end

  @doc """
  Updates a plugin for `owner` to the version its update offers (Update), or `approve`s one that asks for more
  plugins (Approve). One that does and isn't approved waits for approval. Gives the record it's at.
  """
  def update(owner, name, approve) do
    with %{} = record <- move(owner, name, {:update, approve}) do
      if record["state"] == "updating", do: updating(owner, name, record), else: record
    end
  end

  @doc "Tries again what failed (Try Again): installing it, or the update or switch that failed. Gives the record it's at."
  def retry(owner, name) do
    with %{} = record <- move(owner, name, :retry) do
      case record do
        %{"state" => "installing", "plugin" => artifact, "trusted" => trusted} ->
          with {number, _code, _tag} <- Store.latest(artifact) || fail(owner, name, "There's no plugin #{artifact} any more"),
               do: run(owner, name, fn -> version(owner, name, artifact, number, trusted) end)

        %{"state" => "updating"} ->
          updating(owner, name, record)
      end
    end
  end

  @doc "Switches whether a plugin is trusted for `owner`, restarting it the other way. Gives the record it's at."
  def trust(owner, name, trusted) do
    with %{} = record <- move(owner, name, {:trust, trusted}) do
      if record["state"] == "updating", do: updating(owner, name, record), else: record
    end
  end

  @doc "Moves a record by an event that needs no work: Dismiss, Not now. Gives the record it's at, or `nil`."
  def change(owner, name, event) do
    with record when not is_tuple(record) <- move(owner, name, event) do
      ran(owner)
      record
    end
  end

  @doc """
  Uninstalls a plugin for `owner`: it stops running for them, and waits in Recently Deleted, with what it keeps, for
  30 days. Refused where another plugin they have uses it, or it's the network's launcher. The plugins that observe
  `uninstalled` are told, so one that keeps what's deleted takes what it kept and made for them along. `asked` is who
  asked.
  """
  def uninstall(owner, name, {actor, _subject}) do
    # What uses it still finds it where it's installed for the network too.
    after_it = Map.new(Plugins.versions(Plugins.chain(owner) -- [owner]))

    used_by =
      for {other, version} <- Plugins.versions([owner]), name in version.uses, not is_map_key(after_it, name), do: other

    with [] <- used_by,
         :ok <- last_launcher(owner, name),
         %{} = record <- move(owner, name, :uninstall) do
      ran(owner)
      about = %{"for" => owner, "name" => name, "plugin" => record["plugin"]}
      OneCore.Hooks.observe("uninstalled", about, {actor, owner})
      record
    else
      {:error, why} -> {:error, why}
      used_by -> {:error, "#{name} is used by #{Enum.join(used_by, ", ")}. Remove what uses it first."}
    end
  end

  # A network with no launcher has no way in, so the one it has stays until another version replaces it.
  defp last_launcher(owner, name) do
    with %{^name => version} <- Map.new(Plugins.versions([owner])),
         true <- Deploy.launcher?(version) do
      {:error,
       "#{name} is this network's launcher, which every client opens first, and a network with none has no way in. " <>
         "To replace it, deploy another as its new version: ONE_SUBJECT=network one deploy --name #{name} --files-from <folder>"}
    else
      _ -> :ok
    end
  end

  @doc """
  Brings a plugin back from Recently Deleted for `owner`, as it was. Gives the record it's at. The plugins that observe
  `restored` are told, so one that keeps what's deleted brings back what it kept and made for them. `asked` is who asked.
  """
  def restore(owner, name, {actor, _subject}) do
    with %{} = record <- move(owner, name, :restore) do
      ran(owner)
      OneCore.Hooks.observe("restored", %{"for" => owner, "name" => name, "plugin" => record["plugin"]}, {actor, owner})
      record
    end
  end

  @doc "Removes a plugin in Recently Deleted for `owner` for good, with what it kept: `nil`, or `{:error, why}`."
  def purge(owner, name) do
    {records, _automatic} = Installed.read(owner)

    with nil <- move(owner, name, :purge) do
      for blob <- Bucket.list("#{owner}/own/#{name}/"), do: Bucket.remove(blob)
      Installed.unmarked(records[name]["plugin"], owner)
      nil
    end
  end

  @doc """
  Brings what's installed for `owner` up to date with what's newer, as a server acting for them does whenever it's
  told something changed: a plugin the server ships runs its latest version; any other with a newer version offers
  it as an update, taken at once where it may be; one installing or updating for longer than it could has failed; and
  one in Recently Deleted for 30 days is removed for good, with what it kept.
  """
  def reconcile(owner) do
    {records, automatic} = Installed.read(owner)

    for {name, record} <- records do
      try do
        reconciled(owner, name, record, automatic)
      rescue
        error -> Logger.error("#{name} for #{owner} wasn't brought up to date: #{Exception.message(error)}")
      end
    end

    :ok
  end

  defp reconciled(owner, name, %{"state" => "recently_deleted", "deleted" => deleted}, _automatic) do
    if deleted <= System.system_time(:millisecond) - @kept do
      with nil <- purge(owner, name),
           do: Logger.info("#{name} was in Recently Deleted for #{owner} for 30 days, and is removed for good")
    end
  end

  defp reconciled(owner, name, %{"state" => state, "at" => at}, _automatic) when state in ~w(installing updating) do
    if at <= System.system_time(:millisecond) - @partway,
      do: fail(owner, name, "It stopped partway, as the server doing it did. Try again.")
  end

  defp reconciled(owner, name, %{"plugin" => artifact, "version" => number} = record, automatic) when is_integer(number) do
    with {latest, _code, _tag} when latest > number <- Store.latest(artifact) do
      if bundled?(artifact) do
        move(owner, name, {:follow, latest})
        ran(owner)
      else
        offered = %{"version" => latest, "requires" => added(record, artifact, latest), "commits" => commits(artifact, record, latest)}

        with %{"update" => %{"state" => "available"} = update} = record <- move(owner, name, {:available, offered}),
             true <- record["state"] == "installed" and runs?(owner, record) and automatic?(owner, record, update, automatic) do
          Logger.info("#{name} for #{owner} updates by itself to version #{update["version"]}")
          update(owner, name, false)
        end
      end
    end
  end

  defp reconciled(_owner, _name, _record, _automatic), do: :ok

  @doc """
  Looks at the git source of each plugin of `owner`'s own that came from one, for a commit that changed it since the
  version they run, and offers it as an update, taken at once where it may be. Gives what it found, by name.
  """
  def check(owner) do
    {records, automatic} = Installed.read(owner)

    for {name, %{"plugin" => artifact, "version" => number, "state" => state}} <- records,
        state in ~w(installed failed),
        Store.whose(artifact) == owner,
        not bundled?(artifact),
        about = Store.about(artifact),
        about["number"] == number,
        %{"source" => source, "commit" => commit} = from <- [about["from"]],
        into: %{} do
      case newer_commit(source, from["ref"], commit, name) do
        {:ok, nil} ->
          {name, nil}

        {:ok, offered} ->
          with %{"update" => update} = record <- move(owner, name, {:available, offered}),
               true <- record["state"] == "installed" and update["state"] == "available" and runs?(owner, record),
               true <- automatic?(owner, record, update, automatic) do
            update(owner, name, false)
          end

          {name, offered}

        {:error, why} ->
          Logger.warning("#{name}'s git source wasn't checked for #{owner}: #{why}")
          {name, %{"why" => why}}
      end
    end
  end

  # The newest commit at a git source that changed the plugin's folder since `commit`, with the commits between.
  defp newer_commit(source, ref, commit, name) do
    path = "plugins/#{name}"

    with {:ok, copy} <- Git.fetch(source),
         {:ok, newest} <- Git.commit(copy, ref) do
      if newest == commit or Git.tree(copy, newest, path) == Git.tree(copy, commit, path) do
        {:ok, nil}
      else
        with {:ok, commits} <- Git.commits(copy, commit, newest, path),
             do: {:ok, %{"commit" => newest, "source" => source, "ref" => ref, "commits" => commits, "requires" => []}}
      end
    end
  end

  # Whether this server may take an update for `owner`: one that runs their plugins, or any acting for them for an
  # untrusted plugin, which runs in a sandbox.
  defp runs?(owner, record), do: Servers.runs?(owner) or record["trusted"] == false

  # Whether an update may be taken without asking: the owner's automatic updates are on, it's trusted, it asks for
  # no plugin the one running doesn't, and it comes from the network's own plugins or the owner's own repos.
  defp automatic?(owner, record, update, automatic) do
    automatic and record["trusted"] != false and update["requires"] in [nil, []] and
      ours?(owner, record["plugin"], update)
  end

  defp ours?(owner, artifact, %{"commit" => _} = update), do: repo(update["source"]) in [owner, @network] or bundled?(artifact)
  defp ours?(owner, artifact, _update), do: Store.whose(artifact) in [owner, @network]

  @doc """
  Whether a plugin installed for `owner` from `artifact` is trusted where nobody says: the network's own plugins and the
  owner's own are, as the version deployed says, and a stranger's isn't.
  """
  def trusted?(owner, artifact) do
    Store.whose(artifact) in [owner, @network] and Store.about(artifact)["trusted"] != false
  end

  # Whose a repo is, where it's one in the network (`one::users/alice/made/git-repos/notes`), or `nil`.
  defp repo("one::" <> artifact), do: OneCore.User.owner(artifact)
  defp repo(_source), do: nil

  # A plugin the server ships, which updates with the server.
  defp bundled?(@network <> "/plugins/" <> name), do: is_map_key(Application.get_env(:one_core, :bundled, %{}), name)
  defp bundled?(_artifact), do: false

  # The plugins version `latest` requires that the version `record` runs doesn't.
  defp added(%{"version" => number}, artifact, latest) do
    asks = fn number ->
      with code when is_binary(code) <- Store.code(artifact, number),
           {:ok, version} <- Store.described(artifact, code),
           do: Map.keys(version.requires),
           else: (_ -> [])
    end

    asks.(latest) -- asks.(number)
  end

  # The commits between the version `record` runs and the latest, deployed from the same git repo, newest first, or
  # none where either didn't come from one.
  defp commits(artifact, %{"commit" => from}, latest) do
    with %{"number" => ^latest, "from" => %{"source" => source, "commit" => to}} <- Store.about(artifact),
         {:ok, copy} <- Git.fetch(source),
         {:ok, commits} <- Git.commits(copy, from, to, "plugins/#{Path.basename(artifact)}") do
      commits
    else
      _ -> []
    end
  end

  defp commits(_artifact, _record, _latest), do: []

  # What an update does, from `record`'s target: a version deployed, a commit at its git source, or the same version
  # the other way round.
  defp updating(owner, name, %{"plugin" => artifact, "target" => target} = record) do
    trusted = target["trusted"]

    run(owner, name, fn ->
      cond do
        commit = target["commit"] -> from_git(owner, name, record, commit)
        number = target["version"] -> version(owner, name, artifact, number, trusted)
        true -> version(owner, name, artifact, record["version"], trusted)
      end
    end)
  end

  # A version deployed, fetched, built and checked to fit, as `owner` runs it: `{:ok, version}`, or `{:error, why}`.
  defp version(owner, name, artifact, number, trusted) do
    with code when is_binary(code) <- Store.code(artifact, number) || {:error, "Its version #{number} isn't kept any more."},
         files = Store.files(artifact, code),
         _ = move(owner, name, {:stage, "building"}),
         {:ok, version} <- built(owner, artifact, code, files, trusted),
         version = %{version | artifact: artifact, number: number},
         :ok <- Deploy.fits(owner, name, version) do
      {:ok, version}
    else
      :error -> {:error, "It doesn't compile here."}
      {:error, why} -> {:error, why}
    end
  end

  # Loaded where this server runs the owner's plugins, and otherwise only described.
  defp built(owner, artifact, code, _files, true), do: Store.version(artifact, code, Servers.runs?(owner), true)

  defp built(owner, artifact, code, files, false) do
    described =
      with :error <- Store.version(artifact, code, false, false), do: Sandbox.describe(artifact, files)

    with {:ok, version} <- described,
         {:ok, _warnings} <- Sandbox.check(owner, version, files),
         do: {:ok, %{version | trusted: false}}
  end

  # A commit at its git source, fetched and deployed as the owner's next version, which runs only once it's checked
  # to ask for no more than what they approved.
  defp from_git(owner, name, record, commit) do
    %{"source" => source} = update = record["update"] || %{}
    trusted = record["target"]["trusted"]
    install? = fn version -> record["target"]["approved"] == true or (Map.keys(version.requires) -- current_requires(owner, name)) == [] end

    with {:ok, copy} <- Git.fetch(source),
         {:ok, files} <- Git.files(copy, commit, "plugins/#{name}"),
         _ = move(owner, name, {:stage, "building"}),
         from = %{"source" => source, "ref" => update["ref"], "commit" => commit},
         {:ok, number, _warnings, version} <- Deploy.keep(owner, name, files, from, trusted, install?) do
      if install?.(version) do
        {:ok, version}
      else
        Version.unload(version)
        added = Map.keys(version.requires) -- current_requires(owner, name)
        offered = %{"version" => number, "requires" => added, "commits" => update["commits"], "source" => source, "ref" => update["ref"]}
        {:asks, "It requires more than the version running: #{Enum.join(offered["requires"], ", ")}.", offered}
      end
    end
  end

  defp current_requires(owner, name) do
    case Plugins.versions([owner]) do
      %{^name => version} -> Map.keys(version.requires)
      _ -> []
    end
  end

  # Runs what installs or updates a plugin, whose record is at fetching: `work` fetches and builds its version, then
  # it starts. Gives the record it's at.
  defp run(owner, name, work) do
    case work.() do
      {:ok, version} ->
        key = {version.artifact, version.number, version.trusted}

        about = Store.about(version.artifact)
        commit = if about["number"] == version.number, do: about["from"]["commit"]

        with %{} <- move(owner, name, {:start, version.number, commit}) do
          ran(owner, [{key, {:ok, version}}])

          case Plugins.versions([owner]) do
            %{^name => %Version{number: number, trusted: trusted}} when number == version.number and trusted == version.trusted ->
              move(owner, name, :done)

            _ ->
              fail(owner, name, "It didn't start.")
          end
        end

      {:asks, why, offered} ->
        fail(owner, name, why)
        move(owner, name, {:available, offered})

      {:error, why} ->
        fail(owner, name, why)
    end
  rescue
    error ->
      Logger.error(Exception.format(:error, error, __STACKTRACE__))
      fail(owner, name, Exception.message(error))
  end

  defp fail(owner, name, why) do
    record = move(owner, name, {:fail, why})
    ran(owner)
    record
  end

  # What runs for `owner` changed: it runs here, with what's loaded already, and their other servers look.
  defp ran(owner, loaded \\ []) do
    Plugins.reload(owner, loaded)
    Server.notify_servers([owner])
  end

  # A move of the record, told to whoever watches what's installed for `owner`: gives the record it's at, `nil`, or
  # `{:error, why}`.
  defp move(owner, name, event) do
    with record when not is_tuple(record) <- Installed.change(owner, name, event) do
      Server.changed()
      Server.notify_servers([owner], "changed")
      record
    end
  end
end
