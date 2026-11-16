defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, while it keeps running: which versions run, and for whom. Deploying one
  is `OneCore.Plugins.Deploy`, adding and removing one `OneCore.Plugins.Installed`, calling one
  `OneCore.Plugins.Calls`, and what the bucket keeps of one `OneCore.Plugins.Store`.

  A call runs on the version that was latest when it began. That version stays loaded until its last call ends,
  however many versions follow it. A plugin's processes hold a version the same way, and move to the latest
  (`OneCore.Plugins.Process`).

  A server looks in the bucket at what's installed for the network and for whoever it acts for when it starts, and at
  what's installed for anyone else when it's first asked for them. Only a server acting for whoever a plugin was installed for
  runs it for them: another passes the call on to one that does. It looks again when it's told to, when it's asked for
  a plugin it doesn't run, when it comes back (`OneCore.Server.reconnected/0`), and behind a request for someone it
  last looked at more than five minutes ago: the request runs on what the server has, and the look is for what it
  wasn't told. So nothing is read for someone nobody uses.
  """
  use GenServer
  require Logger
  alias OneCore.{App, Call, Hooks, Iroh, Jobs, Server, Servers, Services, User}
  alias OneCore.Plugins.{Installed, Deploy, Store, Version}

  @network "network"

  # How long after looking at someone a request for them has this server look again, behind it, in milliseconds.
  @looks_after :timer.minutes(5)

  # This, and after it what runs plugins' processes and what tells the plugins that hear a hook, which hold
  # versions here.
  def child_spec(_) do
    children = [%{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}]
    children = children ++ __MODULE__.Process.children() ++ Hooks.Hearer.children()
    start = {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}
    %{id: __MODULE__, type: :supervisor, start: start}
  end

  @doc "Whose plugins something that runs for `who` finds, in order: their own, then the network's."
  def chain(who), do: Enum.uniq(List.wrap(User.owner(who)) ++ [@network])

  @doc """
  Runs `fun` with who a plugin was installed for and the latest version of it and of each plugin it uses, by their
  names. The plugin is the first that `name` is for one of `owners`. Those versions stay loaded until `fun`
  returns. Gives what `fun` does, or `{:error, why}` with what it raised, as its caller is told it.
  """
  def run(owners, name, fun) do
    with {:ok, held, at, versions} <- begin(owners, name),
         do: held(held, Map.values(versions), fn -> fun.(at, versions) end)
  end

  @doc "Runs `fun` with the latest version of the plugin `artifact`, as `run/3` does. `:absent` if it isn't running here."
  def run(artifact, fun) do
    with {:ok, held, version} <- hold(artifact), do: held(held, [version], fn -> fun.(version) end)
  end

  @doc """
  Runs `fun`, and gives what it does, or `{:error, why}` with what it raised, as its caller is told it, in the
  names `versions` wrote. A call it had another server run ends there with that server's answer. Lets go of `held`
  after (`release/1`).
  """
  def held(held, versions, fun) do
    fun.()
  catch
    :throw, {Call, outcome} ->
      outcome

    kind, reason ->
      Logger.error(Exception.format(kind, reason, __STACKTRACE__))
      {:error, Enum.reduce(versions, why(kind, reason, __STACKTRACE__), &Version.own_names(&2, &1))}
  after
    if held, do: release(held)
  end

  # What whoever called is told went wrong: what was raised, also where a process the call waited on raised it, and
  # otherwise what Elixir says of it.
  defp why(:exit, {reason, {module, function, arguments}}, stacktrace)
       when is_atom(module) and is_atom(function) and is_list(arguments),
       do: why(:exit, reason, stacktrace)

  defp why(:exit, {%{__exception__: true} = error, raised}, _stacktrace) when is_list(raised),
    do: why(:error, error, raised)

  defp why(:error, error, stacktrace) do
    {blamed, _} = Exception.blame(:error, error, stacktrace)
    Exception.message(blamed)
  end

  defp why(kind, reason, _stacktrace), do: Exception.format_banner(kind, reason)

  @doc """
  Holds the latest version of the plugin `artifact` loaded: `{:ok, held, version}`, or `:absent` if it isn't
  running here. It stays loaded until `release(held)`, or until the process that asked ends.
  """
  def hold(artifact), do: ask({:hold, artifact})

  @doc "Lets go of the versions that were held, which are unloaded once nothing runs or holds them."
  def release(held), do: ask({:end, held})

  @doc "The plugins something that runs for `who` finds, by name: each one's `Version`, which holds its tools, its skill and its apps."
  def running(who), do: versions(chain(who))

  @doc "The plugins installed for `owners`, by name, the first's found first. One this server hasn't looked at is first."
  def versions(owners) do
    with :unknown <- ask({:running, owners}) do
      Enum.each(owners, &reload/1)
      ask({:running, owners})
    end
  end

  @doc "The plugin that `name` is for `owner`, as its artifact's name, or `nil` if none is installed for them by that name."
  def plugin(owner, name) do
    case versions([owner]) do
      %{^name => version} -> version.artifact
      _ -> nil
    end
  end

  @doc "Who a plugin was installed for and its name there, as something that runs for `who` finds the plugin `artifact`, or `nil`."
  def installed(who, artifact) do
    Enum.find_value(chain(who), fn at ->
      Enum.find_value(versions([at]), fn {name, version} -> if version.artifact == artifact, do: {at, name} end)
    end)
  end

  @doc "Everyone this server has looked at that the plugin `artifact` is installed for."
  def installed_for(artifact), do: ask({:installed_for, artifact})

  @doc "The version running here of the plugin `artifact`, or `nil`."
  def version(artifact), do: ask({:version, artifact})

  @doc "Every plugin this server has looked at that has scheduled jobs: who it was installed for, its name there, and the jobs."
  def scheduled, do: ask(:scheduled)

  @doc "What's installed for everyone this server has looked at, by who: each plugin's name there, with the plugin."
  def installed, do: ask(:installed)

  @doc """
  The versions running here of the plugins `names` and of each plugin they use, by their names, as `owner` finds
  them: `{:ok, versions}`, or `{:absent, name}` for the first that isn't running here.
  """
  def used(owner, names), do: ask({:used, owner, names})

  @doc """
  Has everyone this server has looked at looked at again behind the next request for them, and gives who to look
  at now: the network, whoever this server acts for, and whoever's plugins have services installed here.
  """
  def refresh do
    ask(:refresh)
    here(Server.key())
  end

  # Whose plugins a server runs whether or not it's asked for them: the network's, those of whoever it acts for, and
  # the plugins whose services it runs, which are installed for whoever's they are.
  defp here(key) do
    services = for {plugin, _service} <- Servers.installed(key), do: Store.whose(plugin)
    Enum.uniq([@network | Servers.acts_for(key)] ++ services)
  end

  @doc """
  Looks in the bucket at what's installed for `owner`, and runs each plugin's latest version. Gives whether anything
  changed. `loaded` are versions already loaded here, each as `OneCore.Plugins.Store.newer/2` gives one. A plugin
  whose latest version took over has its processes follow it, and its services installed here that weren't
  running start.
  """
  def reload(owner, loaded \\ []) do
    {installed, _tag} = Installed.read(owner)
    known = for {artifact, _number, _loaded} <- loaded, do: artifact
    newer = Store.newer(Enum.uniq(Map.values(installed)) -- known, ask(:latest))
    {changed, any} = ask({:looked, owner, installed, loaded ++ newer})

    for artifact <- changed do
      # A process whose own code brought the newer version in is the one that can't wait for itself.
      for process <- __MODULE__.Process.all(artifact), process != self(), do: __MODULE__.Process.follow(process)
      Services.start(artifact)
    end

    for {artifact, number, {:ok, _version}} <- loaded ++ newer,
        artifact in changed,
        do: Logger.info("#{artifact}: version #{number} runs here")

    if any do
      Hooks.forget()
      Jobs.reload()
    end

    if Enum.any?(changed, &(Path.basename(&1) == "ui")), do: Task.start(&Deploy.rebuild/0)
    any
  end

  # A call holds the latest version of its plugin and of each plugin that one uses, however far that goes. A
  # plugin this server doesn't run is looked for first.
  defp begin(owners, name) do
    with {:absent, _missing} <- ask({:begin, owners, name}),
         _ = Enum.each(Enum.uniq(owners ++ [@network]), &reload(&1)),
         {:absent, missing} <- ask({:begin, owners, name}) do
      if missing == name,
        do: {:error, "There's no plugin named #{name}"},
        else: {:error, "#{name} needs #{missing}, which isn't installed for #{Enum.join(owners, " or ")} or the network"}
    end
  end

  @doc "Returns once this server has started the plugins it runs whether or not it's asked for them."
  def started, do: ask(:started)

  # Everything asked of this waits while it starts the plugins, so the server can be reached meanwhile: what's asked
  # of it is answered once they're there.
  defp ask(message), do: GenServer.call(__MODULE__, message, :infinity)

  # A server starts with the latest version of every plugin it runs whether or not it's asked for them. `installed` is
  # what's installed for each owner it has looked at, and `looked` when it last did; `running` and `latest` are by
  # plugin, and `calls` are the versions each call holds.
  @impl true
  def init(nil) do
    Hooks.keep()
    {:ok, %{installed: %{}, reloaded: %{}, running: %{}, latest: %{}, calls: %{}}, {:continue, :start}}
  end

  @impl true
  def handle_continue(:start, state) do
    {:ok, key} = Iroh.key(Server.key_file())

    state =
      for owner <- here(key), reduce: state do
        state ->
          {installed, _tag} = Installed.read(owner)
          newer = Store.newer(Enum.uniq(Map.values(installed)), state.latest)
          {state, _changed, _any} = took_over(state, owner, installed, newer)
          state
      end

    # What a UI library that stopped running while this server was down built with goes now.
    App.keep_builds(Map.values(state.running))
    {:noreply, state}
  end

  @impl true
  def handle_call(:started, _from, state), do: {:reply, :ok, state}

  def handle_call({:looked, owner, installed, loaded}, _from, state) do
    {state, changed, any} = took_over(state, owner, installed, loaded)
    {:reply, {changed, any}, state}
  end

  def handle_call(:latest, _from, state), do: {:reply, state.latest, state}
  def handle_call(:refresh, _from, state), do: {:reply, :ok, %{state | reloaded: %{}}}
  def handle_call(:installed, _from, state), do: {:reply, state.installed, state}
  def handle_call({:version, artifact}, _from, state), do: {:reply, state.running[artifact], state}

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
          %Version{jobs: [_ | _] = jobs} <- [state.running[artifact]],
          do: {owner, name, jobs}

    {:reply, scheduled, state}
  end

  def handle_call({:used, owner, names}, _from, state), do: {:reply, used(state, [owner, @network], names, %{}), state}

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, owners, name}, {caller, _}, state) do
    state = behind(state, owners)

    with {at, _artifact} <- found(state, owners, name),
         {:ok, versions} <- used(state, [at, @network], [name], %{}) do
      call = Process.monitor(caller)
      {:reply, {:ok, call, at, versions}, put_in(state.calls[call], Map.values(versions))}
    else
      absent -> {:reply, absent, state}
    end
  end

  def handle_call({:hold, artifact}, {caller, _}, state) do
    case state.running do
      %{^artifact => version} ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, version}, put_in(state.calls[call], [version])}

      _ ->
        {:reply, :absent, state}
    end
  end

  def handle_call({:end, call}, _from, state) do
    Process.demonitor(call, [:flush])
    {:reply, :ok, ended(state, call)}
  end

  @impl true
  def handle_info({:DOWN, call, :process, _, _}, state), do: {:noreply, ended(state, call)}

  # Whoever a request is for, and the network, whose plugins it may run, are looked at behind it if this server last
  # looked at them more than five minutes ago, once however many requests come meanwhile. One it has never looked at
  # is looked at before the request instead.
  defp behind(state, owners) do
    now = System.monotonic_time(:millisecond)

    stale =
      for owner <- Enum.uniq(owners ++ [@network]),
          is_map_key(state.installed, owner),
          Map.get(state.reloaded, owner, now - @looks_after) <= now - @looks_after,
          do: owner

    for owner <- stale, do: Task.start(fn -> Server.reload(owner) end)
    %{state | reloaded: Map.merge(state.reloaded, Map.new(stale, &{&1, now}))}
  end

  # What's installed for `owner` is `installed` now, and each version in `loaded` takes over from its plugin's if it's
  # newer: deploys that finish out of order still leave the one with the highest number, as the bucket has it. A
  # version that doesn't compile here leaves the one before it running. A plugin that's installed for nobody this
  # server has looked at no longer runs here. Gives the plugins whose version changed, and whether anything did.
  defp took_over(state, owner, installed, loaded) do
    {state, changed, stale} =
      Enum.reduce(loaded, {state, [], []}, fn {artifact, number, loaded}, {state, changed, stale} ->
        version = with {:ok, version} <- loaded, do: version

        cond do
          number <= Map.get(state.latest, artifact, 0) ->
            {state, changed, [version | stale]}

          version == :error ->
            {put_in(state.latest[artifact], number), changed, stale}

          true ->
            state = %{state | latest: Map.put(state.latest, artifact, number)}
            {put_in(state.running[artifact], version), [artifact | changed], [state.running[artifact] | stale]}
        end
      end)

    any = changed != [] or state.installed[owner] != installed
    state = %{state | installed: Map.put(state.installed, owner, installed)}
    state = put_in(state.reloaded[owner], System.monotonic_time(:millisecond))
    kept = for {_owner, installed} <- state.installed, {_name, artifact} <- installed, into: MapSet.new(), do: artifact
    {running, dropped} = Map.split_with(state.running, fn {artifact, _version} -> artifact in kept end)

    state = %{
      state
      | running: running,
        latest: Map.filter(state.latest, fn {artifact, _number} -> artifact in kept end)
    }

    changed = Enum.uniq(changed ++ Map.keys(dropped))

    # Which of each owner's plugins hooks into what, and the hooks they offer, are kept for every call to find
    # without asking.
    for {at, installed} <- state.installed, at == owner or Enum.any?(installed, fn {_name, artifact} -> artifact in changed end) do
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
            %{name: hook, kind: kind} <- offers,
            into: %{},
            do: {"#{name}.#{hook}", kind}

      :ets.insert(Hooks, {at, hooking, offered})
    end

    {unload_unused(state, stale ++ Map.values(dropped)), changed, any}
  end

  # The versions running of the plugins `installed`, by their names there.
  defp running(state, installed) do
    for {name, artifact} <- installed, version = state.running[artifact], into: %{}, do: {name, version}
  end

  # Who the plugin `name` was installed for and the plugin, as the first of `owners` that has one by that name finds
  # it. One of them this server hasn't looked at yet is looked at first.
  defp found(_state, [], name), do: {:absent, name}

  defp found(state, [owner | owners], name) do
    case state.installed do
      %{^owner => %{^name => artifact}} -> {owner, artifact}
      %{^owner => _added} -> found(state, owners, name)
      _ -> {:absent, name}
    end
  end

  # The versions a call to `names` holds, by their plugins' names as `owners` find them, or the first plugin that
  # isn't running.
  defp used(_state, _owners, [], found), do: {:ok, found}
  defp used(state, owners, [name | names], found) when is_map_key(found, name), do: used(state, owners, names, found)

  defp used(state, owners, [name | names], found) do
    with {_at, artifact} <- found(state, owners, name),
         %{^artifact => version} <- state.running do
      used(state, owners, version.uses ++ names, Map.put(found, name, version))
    else
      _ -> {:absent, name}
    end
  end

  defp ended(state, call) do
    {versions, calls} = Map.pop(state.calls, call, [])
    unload_unused(%{state | calls: calls}, versions)
  end

  # A version nothing runs or holds any more is unloaded, and a UI library's build goes once no UI library that runs builds with it.
  defp unload_unused(state, versions) do
    held = Map.values(state.running) ++ Enum.concat(Map.values(state.calls))
    unused = for %Version{} = version <- Enum.uniq(versions), version not in held, do: version
    Enum.each(unused, &Version.unload/1)
    if unused != [], do: App.keep_builds(held)
    state
  end
end
