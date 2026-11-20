defmodule OneCore.Apps do
  @moduledoc """
  Runs a network's apps on this server, while it keeps running: which versions run, and for whom. Deploying one
  is `OneCore.Apps.Deploy`, adding and removing one `OneCore.Apps.Installed`, calling one
  `OneCore.Apps.Calls`, and what the bucket keeps of one `OneCore.Apps.Store`.

  A call runs on the version that was latest when it began. That version stays loaded until its last call ends,
  however many versions follow it. An app's processes hold a version the same way, and move to the latest
  (`OneCore.Apps.Process`). What's loaded, and who uses it, is `OneCore.Apps.LoadedVersion`'s, which outlives
  this: restarted, this adopts what's loaded rather than loading it again.

  A server looks in the bucket at what's installed for the network and for whoever it acts for when it starts, and at
  what's installed for anyone else when it's first asked for them. Only a server that runs the apps of whoever a
  app was installed for (`runs?/1`) loads it and runs it for them, and one of a network's owner's own servers the
  network's, for their services (`loads?/1`): another knows it only by what it says of itself
  (`OneCore.Apps.Version.Description`), which lists it, and passes every call to it on to one that runs it. It looks
  again when it's told to, when it's asked for an app it doesn't run, when it comes back (`reconnected/0`), and
  behind a request for someone it last looked at more than five minutes ago: the request runs on what the server has,
  and the look is for what it wasn't told. So nothing is read for someone nobody uses.
  """
  use GenServer
  require Logger
  alias OneCore.{Call, Hooks, Iroh, Jobs, Server, Servers, Services, Skills, User}
  alias OneCore.Apps.{Installed, LoadedVersion, Store, Version}

  @network "network"

  # How long after looking at someone a request for them has this server look again, behind it, in milliseconds.
  @looks_after :timer.minutes(5)

  # What's loaded, then this, and after it what runs apps' processes, what tells the apps that observe a hook,
  # and the sandboxes untrusted apps run in, which hold versions here.
  def child_spec(_) do
    children = [
      LoadedVersion,
      %{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}
    ]

    children = children ++ __MODULE__.Process.children() ++ Hooks.Observer.children() ++ OneCore.Sandbox.children()
    start = {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}
    %{id: __MODULE__, type: :supervisor, start: start}
  end

  @doc "Whose apps something that runs for `who` finds, in order: their own, then the network's."
  def chain(who), do: Enum.uniq(List.wrap(User.owner(who)) ++ [@network])

  @doc """
  Runs `fun` with who an app was installed for and the latest version of it and of each app it requires whose code
  runs inside it (`required/2`) for them, by their names. The app is the first that `name` is for one of `owners`.
  Those versions stay loaded until `fun` returns. Gives what `fun` does, or `{:error, why}` with what it raised, as its
  caller is told it.
  """
  def run(owners, name, fun) do
    with {:ok, at, versions} <- begin(owners, name), do: released(Map.values(versions), fn -> fun.(at, versions) end)
  end

  @doc "Runs `fun` with the latest version of the app `artifact`, as `run/3` does. `:absent` if it isn't running here."
  def run(artifact, fun) do
    with {:ok, version} <- hold(artifact), do: released([version], fn -> fun.(version) end)
  end

  defp released(versions, fun) do
    held(versions, fun)
  after
    LoadedVersion.release(versions)
  end

  @doc """
  Runs `fun`, and gives what it does, or `{:error, why}` with what it raised, as its caller is told it, in the
  names `versions` wrote. A call it had another server run ends there with that server's answer.
  """
  def held(versions, fun) do
    fun.()
  catch
    :throw, {Call, outcome} ->
      outcome

    kind, reason ->
      Logger.error(OneCore.Redact.format(kind, reason, __STACKTRACE__))
      why = Enum.reduce(versions, why(kind, reason, __STACKTRACE__), &Version.own_names(&2, &1))
      {:error, if(said?(kind, reason), do: why, else: "Something went wrong.\n\n" <> why)}
  end

  # Whether what was raised is the app saying why, `raise "..."`, also in a process the call waited on. Anything
  # else is a fault, in its code or in what it waited on: a tool error's first paragraph is what a person's screen
  # shows of it (`ui`'s `call`), so theirs is only that something went wrong, and the rest is for an agent.
  defp said?(:error, %RuntimeError{}), do: true
  defp said?(:error, %Call.State.Cancelled{}), do: true
  defp said?(:exit, {%RuntimeError{}, raised}) when is_list(raised), do: true

  defp said?(:exit, {reason, {module, function, arguments}})
       when is_atom(module) and is_atom(function) and is_list(arguments), do: said?(:exit, reason)

  defp said?(_kind, _reason), do: false

  # What whoever called is told went wrong: what was raised, also where a process the call waited on raised it, and
  # otherwise what Elixir says of it. It's redacted (`OneCore.Redact`), since it's passed on, to agents and to
  # Telemetry, and what an error holds may be a person's tokens.
  defp why(:exit, {reason, {module, function, arguments}}, stacktrace)
       when is_atom(module) and is_atom(function) and is_list(arguments),
       do: why(:exit, reason, stacktrace)

  defp why(:exit, {%{__exception__: true} = error, raised}, _stacktrace) when is_list(raised),
    do: why(:error, error, raised)

  defp why(:error, error, stacktrace) do
    {blamed, _} = Exception.blame(:error, error, stacktrace)
    Exception.message(OneCore.Redact.redact(blamed))
  end

  defp why(kind, reason, _stacktrace), do: Exception.format_banner(kind, OneCore.Redact.redact(reason))

  @doc """
  Holds the latest version of the app `artifact` loaded: `{:ok, version}`, or `:absent` if it isn't running here.
  It stays loaded until the process that asked lets go of it (`OneCore.Apps.LoadedVersion.release/1`) or ends.
  """
  def hold(artifact), do: ask({:hold, artifact})

  @doc "The apps something that runs for `who` finds, by name: each one's `Version`, which holds its tools, its skill and its apps."
  def running(who), do: versions(chain(who))

  @doc "The apps installed for `owners`, by name, the first's found first. One this server hasn't looked at is first."
  def versions(owners) do
    with :unknown <- ask({:running, owners}) do
      Enum.each(owners, &sync/1)
      ask({:running, owners})
    end
  end

  @doc "The app that `name` is for `owner`, as its artifact's name, or `nil` if none is installed for them by that name."
  def app(owner, name) do
    case versions([owner]) do
      %{^name => version} -> version.artifact
      _ -> nil
    end
  end

  @doc "Who an app was installed for and its name there, as something that runs for `who` finds the app `artifact`, or `nil`."
  def installed(who, artifact) do
    Enum.find_value(chain(who), fn at ->
      Enum.find_value(versions([at]), fn {name, version} -> if version.artifact == artifact, do: {at, name} end)
    end)
  end

  @doc "Everyone this server has looked at that the app `artifact` is installed for."
  def installed_for(artifact), do: ask({:installed_for, artifact})

  @doc "The version running here of the app `artifact`, or `nil`: also where it's only described here."
  def version(artifact), do: ask({:version, artifact})

  @doc """
  Every app this server has looked at that has scheduled jobs: who it was installed for, its name there, the jobs,
  whether it's trusted, and whose its own artifact is (`OneCore.Apps.Calls.whose/3`).
  """
  def scheduled, do: ask(:scheduled)

  @doc "What's installed for everyone this server has looked at, by who: each app's name there, with the app."
  def installed, do: ask(:installed)

  @doc """
  The versions running here of the apps `version` requires whose code runs inside it, by their names, as `owner`
  finds them: each whose module provides functions (its `exports`, `OneCore.App`), and those such a one requires in
  turn. A requirement that isn't running here runs inside nothing, and an untrusted one runs inside only an untrusted
  app, in its sandbox: its code never loads in this server's VM.
  """
  def required(owner, %Version{} = version), do: ask({:required, owner, version})

  @doc """
  Has everyone this server has looked at looked at again behind the next request for them, and gives who to look
  at now: the network, whoever this server acts for, and whoever's apps have services installed here.
  """
  def refresh do
    ask(:refresh)
    here(Server.key())
  end

  @doc """
  Tells every client watching this server that something changed, so that it asks again (`OneCore.Server.tell/2`).
  It says the UI library's build (`library`), and for each of `owners` whose apps moved, the version each of theirs
  runs at (`apps`): an app's screen loaded against another of either is stale, and its client loads it again. With
  no server started, nobody is told. The versions are looked up apart from whoever says it, which may be what keeps
  them. Where it's an app's call that changed something, it says that app (`app`), so a client asks only what's that
  app's again; left out, it may be anything.
  """
  def changed(owners \\ [], app \\ nil) do
    Task.Supervisor.start_child(OneCore.Asks, fn ->
      apps =
        Map.new(owners, fn owner ->
          {owner, for({name, version} <- versions([owner]), into: %{}, do: {name, version.code})}
        end)

      params = %{library: OneCore.Entrypoint.library().code, apps: apps}
      params = if app, do: Map.put(params, :app, app), else: params
      Server.tell("changed", params)
    end)

    :ok
  end

  @doc """
  Looks in the bucket at what other servers put there for `owner`: the servers on their list, what each acts for
  and the services it runs, and what's installed for them, running the latest version of each app. Its clients are
  told if an app changed.

  A server looks at someone when it's told to, when it's asked for them and last looked more than five minutes
  ago, and when it comes back (`reconnected/0`). Nothing else has it look, so someone nobody uses costs nothing.
  """
  def reload(owner) do
    Servers.reload(owner)
    OneCore.Apps.Deploy.forget()
    if sync(owner), do: changed([owner])
    # An app updated by a server elsewhere may require what isn't installed for them yet.
    if Servers.here?(owner), do: Task.start(fn -> OneCore.Apps.Lifecycle.required(owner) end)
    :ok
  end

  @doc """
  This server can reach the others again, its network's address or its bucket, after it couldn't, so it may have
  missed being told something, and the edge what it is: it says its status again (`OneCore.Servers.report/0`),
  invalidates what it cached of servers and hooks, looks at once at who it acts for, and at everyone else it has
  looked at behind the next request for them (`refresh/0`). Saying its status is how the network's update hears
  it's back, and catches it up if it missed a release.
  """
  def reconnected do
    Servers.report()
    Hooks.invalidate()
    Servers.invalidate()
    Enum.each(refresh(), &reload/1)
  end

  @doc """
  Whether this server runs the apps installed for `owner`: it acts for them as one of their own servers, not
  only as one of the network's, or it's the network's and they are, or own, the network. Who owns the network is
  what the apps that validate `reach` say.
  """
  def runs?(owner) do
    cond do
      owner == @network -> Servers.here?(@network)
      owner in Servers.here() -> true
      true -> Servers.here?(@network) and owns_network?(owner)
    end
  end

  @doc """
  Whether this server loads the code of the apps installed for `owner`: where it runs them (`runs?/1`), and the
  network's on a server of one of the network's owners (`owners/0`) too, whose services run there for them, since
  what a service does is on its machine, which is theirs (`OneCore.Service.acts_for/0`). What else of them is asked
  there still runs on the network's servers.
  """
  def loads?(owner), do: runs?(owner) or (owner == @network and owners() != [])

  @doc """
  Whom of the network's owners this server acts for as one of their own servers, and not as one of the network's:
  the network's apps' services run here for them (`loads?/1`).
  """
  def owners do
    if Servers.here?(@network), do: [], else: Enum.filter(Servers.here(), &owns_network?/1)
  end

  @doc """
  Whether the server whose key is `key` asks the network's apps for `who`, as a service of theirs it runs for them
  does (`owners/0`): it acts for them, and they own the network.
  """
  def asks_for?(key, who), do: who in Servers.acts_for(key) and owns_network?(who)

  @doc """
  Whether this server runs an app installed for `owner`, which is `trusted` or not: a trusted one where it runs the
  apps installed for them (`runs?/1`), and an untrusted one, in a sandbox, wherever it acts for them
  (`OneCore.Servers.here?/1`).
  """
  def runs?(owner, trusted), do: if(trusted, do: runs?(owner), else: Servers.here?(owner))

  @doc "The keys of the servers that run the apps installed for `owner`, as `runs?/1` says of each, the first first."
  def servers(@network), do: Servers.listed(@network)

  def servers(owner) do
    own = Enum.filter(Servers.listed(owner), &(owner in Servers.members(&1)))
    Enum.uniq(own ++ if(owns_network?(owner), do: Servers.listed(@network), else: []))
  end

  # Asked of the network's apps by this server, as nobody, as everything about the network is.
  defp owns_network?(owner) do
    about = %{"who" => owner, "relation" => "owner", "artifact" => @network}
    Hooks.validate("reach", about, {nil, @network}) == :ok
  end

  # Whose apps a server runs whether or not it's asked for them: the network's, those of whoever it acts for, and
  # the apps whose services it runs, which are installed for whoever's they are. Its owner installed those on its
  # machine, so they're loaded as it starts, before anything could say whose apps it runs.
  defp here(key) do
    services = for {app, _service} <- Servers.installed(key), do: Store.whose(app)
    Enum.uniq([@network | Servers.acts_for(key)] ++ services)
  end

  @doc """
  What `fetch` gives for `key`, kept until the apps running here change, as what's worked out from them is, such as
  `OneCore.Search`'s index of their tools.
  """
  def cached(key, fetch), do: OneCore.Cache.fetch(__MODULE__, key, fn -> {:commit, fetch.()} end)

  @doc """
  Says what's installed for `owner` changed, and returns once a sync that read it after that has finished: whether
  anything that runs here changed. A sync reads what runs for them (`OneCore.Apps.Installed.running/1`), and runs
  each app's latest version. `loaded` are versions already loaded here, each as `OneCore.Apps.Store.newer/3`
  gives one, held by the caller. An app whose latest version took over has its processes follow it, and its services installed here
  that weren't running start.

  Each owner's apps are in one state, `app_sync`, with one sync in flight, as a Kubernetes controller's work
  queue has one per key: compiling takes seconds, and a sync that read what was installed before another's would
  otherwise land after it, and run what was installed before.

      synced ─installs changed─► syncing ─done─► synced
      syncing ─installs changed─► out_of_sync ─done─► syncing   (reads what's installed again)
  """
  def sync(owner, loaded \\ []) do
    case ask({:sync, owner, loaded}) do
      {:crashed, why} -> exit(why)
      any -> any
    end
  end

  @doc "Whether this server's apps are `synced`, or `syncing` for anyone, as `sync/2` says."
  def synced, do: if(ask(:syncing) == [], do: "synced", else: "syncing")

  # One sync: what runs for `owner` is read, the versions newer than what's here are loaded, and they take over.
  defp sync_once(owner, loaded) do
    installed = Installed.running(owner)
    known = for {artifact, _number, _loaded} <- loaded, do: artifact
    runs = loads?(owner)
    {latest, here} = ask({:latest, runs})
    newer = Store.newer(Enum.uniq(Map.values(installed)) -- (known ++ here), latest, runs)
    {changed, any} = ask({:looked, owner, installed, loaded ++ newer, Map.keys(latest) ++ here})
    # A process waiting for a sync, as one whose own code brought the newer version in does, can't follow until it ends.
    waiting = ask(:waiting)
    LoadedVersion.release(for {_artifact, _number, {:ok, version}} <- newer, do: version)

    for artifact <- changed do
      for process <- __MODULE__.Process.all(artifact), process not in waiting, do: __MODULE__.Process.follow(process)
      Services.start(artifact)
    end

    for {artifact, number, {:ok, version}} <- loaded ++ newer, artifact in changed do
      case Version.runs(version) do
        :here -> Logger.info("#{artifact}: version #{number} runs here")
        :sandbox -> Logger.info("#{artifact}: version #{number} runs here, in a sandbox")
        :elsewhere -> Logger.info("#{artifact}: version #{number} is listed here, and runs on #{owner}'s own servers")
      end
    end

    if any do
      Hooks.invalidate()
      OneCore.Cache.invalidate(__MODULE__)
      Skills.invalidate()
      Jobs.reload()
    end

    any
  end

  # A call holds the latest version of its app and of each app whose code runs inside it, however far that goes. A
  # app this server doesn't run is looked for first.
  defp begin(owners, name) do
    with {:absent, _missing} <- ask({:begin, owners, name}),
         _ = Enum.each(Enum.uniq(owners ++ [@network]), &sync/1),
         {:absent, _missing} <- ask({:begin, owners, name}),
         do: {:error, "There's no app named #{name}"}
  end

  @doc "Returns once this server has started the apps it runs whether or not it's asked for them."
  def started, do: ask(:started)

  # Everything asked of this waits while it starts the apps, so the server can be reached meanwhile: what's asked
  # of it is answered once they're there.
  defp ask(message), do: GenServer.call(__MODULE__, message, :infinity)

  # A server starts with the latest version of every app it runs whether or not it's asked for them. `installed` is
  # what's installed for each owner it has looked at, and `looked` when it last did; `running` and `latest` are by
  # app. `syncs` is each owner whose apps aren't synced (`sync/2`): the sync in flight, by its task's reference,
  # who waits for it and what they loaded, and with `next` who waits for the one after and what they loaded, while
  # it's out of sync. What they loaded is held by this until the sync that takes it has, however they end meanwhile.
  @impl true
  def init(nil) do
    Hooks.keep()
    # What this server is changing, and the network's update, are told to the clients watching (`changed/2`).
    Registry.register(OneCore.Waiting, :status, nil)
    Registry.register(OneCore.Waiting, :network, nil)
    {:ok, %{installed: %{}, reloaded: %{}, running: %{}, latest: %{}, described: %{}, syncs: %{}}, {:continue, :start}}
  end

  @impl true
  def handle_continue(:start, state) do
    {:ok, key} = Iroh.key(Server.key_file())

    # What couldn't be read or started is left out and said in the log, as when the bucket didn't give or refused it:
    # someone's apps, or everyone's, if it couldn't say whose apps run here. A bucket that didn't answer has
    # this server look again once it does (`OneCore.Bucket`). What's loaded already is adopted, not loaded again.
    {here, state} =
      try do
        here = here(key)

        {here,
         for owner <- here, reduce: state do
           state ->
             try do
               installed = Installed.running(owner)
               newer = Store.newer(Enum.uniq(Map.values(installed)), state.latest)
               {state, _changed, _any} = took_over(state, owner, installed, newer)
               LoadedVersion.release(for {_artifact, _number, {:ok, version}} <- newer, do: version)
               state
             rescue
               unanswered -> unstarted(state, owner, unanswered)
             end
         end}
      rescue
        unanswered -> {[], unstarted(state, "everyone", unanswered)}
      end

    # Whoever else something loaded here is installed for, this looked at before it restarted: they're looked at
    # again beside this, as reading what's installed for them may call an app, and what's theirs is adopted.
    Task.start(fn -> for owner <- LoadedVersion.owners() -- here, do: sync(owner) end)
    {:noreply, state}
  end

  defp unstarted(state, whose, unanswered) do
    Logger.error("#{whose}'s apps aren't started here: #{Exception.message(unanswered)}")
    state
  end

  @impl true
  def handle_call(:started, _from, state), do: {:reply, :ok, state}

  def handle_call({:sync, owner, loaded}, from, state) do
    LoadedVersion.hold(handed(loaded), self())

    case state.syncs do
      %{^owner => %{next: {waiting, more}} = sync} ->
        {:noreply, put_in(state.syncs[owner], %{sync | next: {[from | waiting], more ++ loaded}})}

      %{^owner => sync} ->
        {:noreply, put_in(state.syncs[owner], %{sync | next: {[from], loaded}})}

      _ ->
        {:noreply, syncing(state, owner, [from], loaded)}
    end
  end

  def handle_call(:syncing, _from, state), do: {:reply, Map.keys(state.syncs), state}

  def handle_call(:waiting, _from, state) do
    waiting = for {_owner, sync} <- state.syncs, {pid, _tag} <- sync.waiting ++ elem(sync.next || {[], []}, 0), do: pid
    {:reply, waiting, state}
  end

  # An app the sync took to be here already, `here`, that another owner's sync dropped meanwhile, isn't running:
  # the owner is out of sync, and the next sync reads it.
  def handle_call({:looked, owner, installed, loaded, here}, _from, state) do
    {state, changed, any} = took_over(state, owner, installed, loaded)

    gone =
      for artifact <- Map.values(installed), artifact in here, not is_map_key(state.running, artifact), do: artifact

    state = if gone != [] and state.syncs[owner].next == nil, do: put_in(state.syncs[owner].next, {[], []}), else: state
    {:reply, {changed, any}, state}
  end

  # What a look at someone compares the bucket with: for someone whose apps run here, the versions loaded, and
  # otherwise the ones described, leaving out the apps loaded here for someone else, which serve them too.
  def handle_call({:latest, true}, _from, state), do: {:reply, {state.latest, []}, state}
  # An untrusted app runs here for anyone, so it's compared as one described is.
  def handle_call({:latest, false}, _from, state) do
    {untrusted, loaded} =
      Map.split_with(state.latest, fn {artifact, _} -> runs(state.running[artifact]) == :sandbox end)

    {:reply, {Map.merge(state.described, untrusted), Map.keys(loaded)}, state}
  end

  def handle_call(:refresh, _from, state), do: {:reply, :ok, %{state | reloaded: %{}}}
  def handle_call(:installed, _from, state), do: {:reply, state.installed, state}
  def handle_call({:version, artifact}, _from, state), do: {:reply, loaded(state.running[artifact]), state}

  def handle_call({:installed_for, artifact}, _from, state) do
    {:reply, for({owner, installed} <- state.installed, artifact in Map.values(installed), do: owner), state}
  end

  def handle_call({:running, owners}, _from, state) do
    state = behind(state, owners)

    if Enum.all?(owners, &is_map_key(state.installed, &1)) do
      # Their own are found before the network's.
      {:reply, running(state, Enum.reduce(Enum.reverse(owners), %{}, &Map.merge(&2, state.installed[&1]))), state}
    else
      {:reply, :unknown, state}
    end
  end

  def handle_call(:scheduled, _from, state) do
    scheduled =
      for {owner, installed} <- state.installed,
          {name, artifact} <- installed,
          %Version{jobs: [_ | _] = jobs, trusted: trusted, own: own} <- [state.running[artifact]],
          do: {owner, name, jobs, trusted, own}

    {:reply, scheduled, state}
  end

  def handle_call({:required, owner, version}, _from, state),
    do: {:reply, inside(state, [owner, @network], version.trusted, Map.keys(version.requires), %{}), state}

  # The caller holds them until it lets go of them or ends.
  def handle_call({:begin, owners, name}, {caller, _}, state) do
    state = behind(state, owners)

    with {at, artifact} <- found(state, owners, name),
         %Version{} = version <- state.running[artifact] || {:absent, name} do
      versions = inside(state, [at, @network], version.trusted, Map.keys(version.requires), %{name => version})
      :ok = LoadedVersion.hold(Map.values(versions), caller)
      {:reply, {:ok, at, versions}, state}
    else
      absent -> {:reply, absent, state}
    end
  end

  def handle_call({:hold, artifact}, {caller, _}, state) do
    case loaded(state.running[artifact]) do
      %Version{} = version ->
        :ok = LoadedVersion.hold([version], caller)
        {:reply, {:ok, version}, state}

      nil ->
        {:reply, :absent, state}
    end
  end

  @impl true
  def handle_info(changed, state) when changed in [:status, :network] do
    changed()
    {:noreply, state}
  end

  def handle_info({ref, any}, state) when is_reference(ref) do
    Process.demonitor(ref, [:flush])
    if sync?(state, ref), do: {:noreply, done(state, ref, any)}, else: {:noreply, state}
  end

  def handle_info({:DOWN, ref, :process, _, why}, state) do
    if sync?(state, ref), do: {:noreply, done(state, ref, {:crashed, why})}, else: {:noreply, state}
  end

  defp handed(loaded), do: for({_artifact, _number, {:ok, version}} <- loaded, do: version)

  defp sync?(state, ref), do: Enum.any?(state.syncs, fn {_owner, sync} -> sync.ref == ref end)

  # A sync runs beside this, which it asks what it needs, so whatever's asked meanwhile is answered.
  defp syncing(state, owner, waiting, loaded) do
    task = Task.Supervisor.async_nolink(OneCore.Asks, fn -> sync_once(owner, loaded) end)
    put_in(state.syncs[owner], %{ref: task.ref, waiting: waiting, loaded: loaded, next: nil})
  end

  # A sync ended: whoever waited for it is told what it gave, and an owner out of sync syncs again.
  defp done(state, ref, outcome) do
    {owner, sync} = Enum.find(state.syncs, fn {_owner, sync} -> sync.ref == ref end)
    for from <- sync.waiting, do: GenServer.reply(from, outcome)
    LoadedVersion.release(handed(sync.loaded))

    case sync.next do
      nil -> %{state | syncs: Map.delete(state.syncs, owner)}
      {waiting, loaded} -> syncing(state, owner, waiting, loaded)
    end
  end

  # Whoever a request is for, and the network, whose apps it may run, are looked at behind it if this server last
  # looked at them more than five minutes ago, once however many requests come meanwhile. One it has never looked at
  # is looked at before the request instead.
  defp behind(state, owners) do
    now = System.monotonic_time(:millisecond)

    stale =
      for owner <- Enum.uniq(owners ++ [@network]),
          is_map_key(state.installed, owner),
          Map.get(state.reloaded, owner, now - @looks_after) <= now - @looks_after,
          do: owner

    for owner <- stale, do: Task.start(fn -> reload(owner) end)
    %{state | reloaded: Map.merge(state.reloaded, Map.new(stale, &{&1, now}))}
  end

  # What's installed for `owner` is `installed` now, and each version in `loaded` takes over from its app's if it's
  # newer: deploys that finish out of order still leave the one with the highest number, as the bucket has it. A
  # version that doesn't compile here leaves the one before it running. An app that's installed for nobody this
  # server has looked at no longer runs here. Each owner's versions are installed for them
  # (`OneCore.Apps.LoadedVersion.installed/1`), which lets go of what they no longer run. Gives the apps whose
  # version changed, and whether anything did.
  defp took_over(state, owner, installed, loaded) do
    {state, changed} =
      Enum.reduce(loaded, {state, []}, fn {artifact, number, loaded}, {state, changed} ->
        version = with {:ok, version} <- loaded, do: version
        # A version described is kept apart from the ones loaded, which it never takes over from. An untrusted
        # one runs here as one loaded does.
        numbers =
          if runs(version) == :elsewhere, do: :described, else: :latest

        cond do
          numbers == :described and is_map_key(state.latest, artifact) ->
            {state, changed}

          number <= Map.get(Map.fetch!(state, numbers), artifact, 0) ->
            {state, changed}

          version == :error ->
            {put_in(state.latest[artifact], number), changed}

          true ->
            state = Map.update!(state, numbers, &Map.put(&1, artifact, number))
            state = if numbers == :latest, do: %{state | described: Map.delete(state.described, artifact)}, else: state
            {put_in(state.running[artifact], version), [artifact | changed]}
        end
      end)

    any = changed != [] or state.installed[owner] != installed
    state = %{state | installed: Map.put(state.installed, owner, installed)}
    state = put_in(state.reloaded[owner], System.monotonic_time(:millisecond))

    artifacts =
      for {_owner, installed} <- state.installed, {_name, artifact} <- installed, into: MapSet.new(), do: artifact

    {running, dropped} = Map.split_with(state.running, fn {artifact, _version} -> artifact in artifacts end)

    state = %{
      state
      | running: running,
        latest: Map.filter(state.latest, fn {artifact, _number} -> artifact in artifacts end),
        described: Map.filter(state.described, fn {artifact, _number} -> artifact in artifacts end)
    }

    LoadedVersion.installed(
      for {at, installed} <- state.installed,
          into: %{},
          do: {at, for({_name, artifact} <- installed, %Version{} = version <- [running[artifact]], do: version)}
    )

    changed = Enum.uniq(changed ++ Map.keys(dropped))

    # Which of each owner's apps hooks into what, and the hooks they offer, are kept for every call to find
    # without asking.
    for {at, installed} <- state.installed,
        at == owner or Enum.any?(installed, fn {_name, artifact} -> artifact in changed end) do
      hooking =
        for {name, artifact} <- Enum.sort(installed),
            %Version{hooks: hooks} <- [running[artifact]],
            hook <- hooks,
            reduce: %{} do
          hooking -> Map.update(hooking, hook, [name], &(&1 ++ [name]))
        end

      offered =
        for {name, artifact} <- installed,
            %Version{offers: offers} <- [running[artifact]],
            %{name: hook, kind: kind} = offer <- offers,
            into: %{},
            do: {"#{name}.#{hook}", {kind, Map.get(offer, :untrusted, false)}}

      :ets.insert(Hooks, {at, hooking, offered})
    end

    {state, changed, any}
  end

  # A version that runs here, or `nil` for one only described.
  defp loaded(version), do: if(runs(version) == :elsewhere, do: nil, else: version)

  # Where a version runs (`OneCore.Apps.Version.runs/1`), or `nil` for none.
  defp runs(%Version{} = version), do: Version.runs(version)
  defp runs(_none), do: nil

  # The versions running of the apps `installed`, by their names there.
  defp running(state, installed) do
    for {name, artifact} <- installed, version = state.running[artifact], into: %{}, do: {name, version}
  end

  # Who the app `name` was installed for and the app, as the first of `owners` that has one by that name finds
  # it. One of them this server hasn't looked at yet is looked at first.
  defp found(_state, [], name), do: {:absent, name}

  defp found(state, [owner | owners], name) do
    case state.installed do
      %{^owner => %{^name => artifact}} -> {owner, artifact}
      %{^owner => _added} -> found(state, owners, name)
      _ -> {:absent, name}
    end
  end

  # `found` with the versions of the apps `names` whose code runs inside whatever requires them, by their names as
  # `owners` find them, and of those they require in turn: the ones running here whose modules provide functions, and
  # only trusted ones inside a `trusted` app, whose call runs in this server's VM.
  defp inside(_state, _owners, _trusted, [], found), do: found

  defp inside(state, owners, trusted, [name | names], found) when is_map_key(found, name),
    do: inside(state, owners, trusted, names, found)

  defp inside(state, owners, trusted, [name | names], found) do
    with {_at, artifact} <- found(state, owners, name),
         %{^artifact => %Version{exports: [_ | _]} = version} when version.trusted or not trusted <- state.running do
      inside(state, owners, trusted, Map.keys(version.requires) ++ names, Map.put(found, name, version))
    else
      _ -> inside(state, owners, trusted, names, found)
    end
  end
end
