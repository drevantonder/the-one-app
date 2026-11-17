defmodule OneCore.Plugins do
  @moduledoc """
  Runs a network's plugins on this server, while it keeps running: which versions run, and for whom. Deploying one
  is `OneCore.Plugins.Deploy`, adding and removing one `OneCore.Plugins.Installed`, calling one
  `OneCore.Plugins.Calls`, and what the bucket keeps of one `OneCore.Plugins.Store`.

  Each owner runs the version of each plugin that what's installed for them says (`OneCore.Plugins.Installed`), so
  two owners may run two versions of one plugin at once, each loaded once whoever runs it. A call runs on the version
  it found when it began, which stays loaded until its last call ends, whatever replaces it. A plugin's processes and
  services run its newest version here, holding it the same way (`OneCore.Plugins.Process`).

  A server looks in the bucket at what's installed for the network and for whoever it acts for when it starts, and at
  what's installed for anyone else when it's first asked for them. Only a server that runs the plugins of whoever a
  plugin was installed for (`OneCore.Servers.runs?/1`) loads it and runs it for them: another knows it only by what it
  says of itself (`OneCore.Plugins.Version.described/2`), which lists it, and passes every call to it on to one
  that does. It looks again when it's told to, when it's asked for
  a plugin it doesn't run, when it comes back (`OneCore.Server.reconnected/0`), and behind a request for someone it
  last looked at more than five minutes ago: the request runs on what the server has, and the look is for what it
  wasn't told. So nothing is read for someone nobody uses.
  """
  use GenServer
  require Logger
  alias OneCore.{Call, Hooks, Iroh, Jobs, Server, Servers, Services, User}
  alias OneCore.Plugins.{Installed, Store, Version}

  @network "network"

  # How long after looking at someone a request for them has this server look again, behind it, in milliseconds.
  @looks_after :timer.minutes(5)

  # This, and after it what runs plugins' processes, what tells the plugins that observe a hook, and the sandboxes
  # untrusted plugins run in, which hold versions here.
  def child_spec(_) do
    children = [%{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}]
    children = children ++ __MODULE__.Process.children() ++ Hooks.Observer.children() ++ OneCore.Sandbox.children()
    start = {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}
    %{id: __MODULE__, type: :supervisor, start: start}
  end

  @doc "Whose plugins something that runs for `who` finds, in order: their own, then the network's."
  def chain(who), do: Enum.uniq(List.wrap(User.owner(who)) ++ [@network])

  @doc """
  Runs `fun` with who a plugin was installed for and the version of it and of each plugin it uses that runs for them, by their
  names. The plugin is the first that `name` is for one of `owners`. Those versions stay loaded until `fun`
  returns. Gives what `fun` does, or `{:error, why}` with what it raised, as its caller is told it.
  """
  def run(owners, name, fun) do
    with {:ok, held, at, versions} <- begin(owners, name),
         do: held(held, Map.values(versions), fn -> fun.(at, versions) end)
  end

  @doc "Runs `fun` with the newest version of the plugin `artifact` here, as `run/3` does. `:absent` if it isn't running here."
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
      Logger.error(OneCore.Redact.format(kind, reason, __STACKTRACE__))
      why = Enum.reduce(versions, why(kind, reason, __STACKTRACE__), &Version.own_names(&2, &1))
      {:error, if(said?(kind, reason), do: why, else: "Something went wrong.\n\n" <> why)}
  after
    if held, do: release(held)
  end

  # Whether what was raised is the plugin saying why, `raise "..."`, also in a process the call waited on. Anything
  # else is a fault, in its code or in what it waited on: a tool error's first paragraph is what a person's screen
  # shows of it (`ui`'s `call`), so theirs is only that something went wrong, and the rest is for an agent.
  defp said?(:error, %RuntimeError{}), do: true
  defp said?(:error, %Call.State.Cancelled{}), do: true
  defp said?(:exit, {%RuntimeError{}, raised}) when is_list(raised), do: true
  defp said?(:exit, {reason, {module, function, arguments}}) when is_atom(module) and is_atom(function) and is_list(arguments), do: said?(:exit, reason)
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
  Holds the newest version of the plugin `artifact` loaded here: `{:ok, held, version}`, or `:absent` if it isn't
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

  @doc "The newest version running here of the plugin `artifact`, or `nil`: also where it's only described here."
  def version(artifact), do: ask({:version, artifact})

  @doc "Every plugin this server has looked at that has scheduled jobs: who it was installed for, its name there, the jobs, and whether it's trusted."
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
  # the plugins whose services it runs, which are installed for whoever's they are. Its owner installed those on its
  # machine, so they're loaded as it starts, before anything could say whose plugins it runs.
  defp here(key) do
    services = for {plugin, _service} <- Servers.installed(key), do: Store.whose(plugin)
    Enum.uniq([@network | Servers.acts_for(key)] ++ services)
  end

  @doc """
  Looks in the bucket at what's installed for `owner`, and runs the version of each plugin their record says
  (`OneCore.Plugins.Installed.running/1`). Gives whether anything changed. `loaded` are versions already loaded here,
  each with its key, as `OneCore.Plugins.Store.versions/2` gives one. A plugin whose newest version here changed has
  its processes follow it, and its services installed here that weren't running start.
  """
  def reload(owner, loaded \\ []) do
    installed = Installed.running(owner)
    known = for {key, _loaded} <- loaded, do: key
    runs = Servers.runs?(owner)
    have = ask({:have, runs})
    newer = Store.versions(Enum.uniq(Map.values(installed)) -- (known ++ have), runs)
    {changed, any} = ask({:looked, owner, installed, loaded ++ newer})

    for artifact <- changed do
      # A process whose own code brought the newer version in is the one that can't wait for itself.
      for process <- __MODULE__.Process.all(artifact), process != self(), do: __MODULE__.Process.follow(process)
      Services.start(artifact)
    end

    for {{artifact, number, trusted}, {:ok, version}} <- loaded ++ newer, artifact in changed do
      if Version.described?(version) and version.trusted,
        do: Logger.info("#{artifact}: version #{number} is listed here, and runs on #{owner}'s own servers"),
        else: Logger.info("#{artifact}: version #{number} runs here#{if not trusted, do: ", in a sandbox"}")
    end

    if any do
      Hooks.forget()
      Jobs.reload()
    end

    any
  end

  # A call holds the version of its plugin and of each plugin that one uses, however far that goes. A
  # plugin this server doesn't run is looked for first.
  defp begin(owners, name) do
    with {:absent, _missing} <- ask({:begin, owners, name}),
         _ = Enum.each(Enum.uniq(owners ++ [@network]), &reload(&1)),
         {:absent, missing} <- ask({:begin, owners, name}) do
      if missing == name,
        do: {:error, "There's no plugin named #{name}"},
        else:
          {:error, "#{name} needs #{missing}, which isn't installed for #{Enum.join(owners, " or ")} or the network"}
    end
  end

  @doc "Returns once this server has started the plugins it runs whether or not it's asked for them."
  def started, do: ask(:started)

  # Everything asked of this waits while it starts the plugins, so the server can be reached meanwhile: what's asked
  # of it is answered once they're there.
  defp ask(message), do: GenServer.call(__MODULE__, message, :infinity)

  # A server starts with the plugins it runs whether or not it's asked for them. `installed` is what runs for each
  # owner it has looked at, each plugin's name with its key: its artifact, its version's number and whether it's
  # trusted; and `reloaded` when it last looked. `running` is each key's version, loaded or only described, `broken`
  # the keys that don't compile here, and `calls` the versions each call holds.
  @impl true
  def init(nil) do
    Hooks.keep()
    {:ok, %{installed: %{}, reloaded: %{}, running: %{}, broken: MapSet.new(), calls: %{}}, {:continue, :start}}
  end

  @impl true
  def handle_continue(:start, state) do
    {:ok, key} = Iroh.key(Server.key_file())

    # What couldn't be read or started is left out and said in the log, as when the bucket didn't give or refused it:
    # someone's plugins, or everyone's, if it couldn't say whose plugins run here. A bucket that didn't answer has
    # this server look again once it does (`OneCore.Bucket`).
    state =
      try do
        for owner <- here(key), reduce: state do
          state ->
            try do
              installed = Installed.running(owner)
              newer = Store.versions(Enum.uniq(Map.values(installed)) -- Map.keys(state.running))
              {state, _changed, _any} = took_over(state, owner, installed, newer)
              state
            rescue
              unanswered -> unstarted(state, owner, unanswered)
            end
        end
      rescue
        unanswered -> unstarted(state, "everyone", unanswered)
      end

    {:noreply, state}
  end

  defp unstarted(state, whose, unanswered) do
    Logger.error("#{whose}'s plugins aren't started here: #{Exception.message(unanswered)}")
    state
  end

  @impl true
  def handle_call(:started, _from, state), do: {:reply, :ok, state}

  def handle_call({:looked, owner, installed, loaded}, _from, state) do
    {state, changed, any} = took_over(state, owner, installed, loaded)
    {:reply, {changed, any}, state}
  end

  # The keys a look at someone needn't read again: for someone whose plugins run here, the ones loaded, or that don't
  # compile here, and otherwise any it has, since one loaded here for someone else serves them too.
  def handle_call({:have, true}, _from, state) do
    have = for {key, version} <- state.running, loaded(version), do: key
    {:reply, have ++ MapSet.to_list(state.broken), state}
  end

  def handle_call({:have, false}, _from, state), do: {:reply, Map.keys(state.running) ++ MapSet.to_list(state.broken), state}

  def handle_call(:refresh, _from, state), do: {:reply, :ok, %{state | reloaded: %{}}}
  def handle_call(:installed, _from, state) do
    {:reply, Map.new(state.installed, fn {owner, installed} -> {owner, Map.new(installed, fn {name, {artifact, _, _}} -> {name, artifact} end)} end), state}
  end

  def handle_call({:version, artifact}, _from, state), do: {:reply, newest(state, artifact), state}

  def handle_call({:installed_for, artifact}, _from, state) do
    {:reply, for({owner, installed} <- state.installed, {^artifact, _, _} <- Map.values(installed), uniq: true, do: owner), state}
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
          {name, key} <- installed,
          %Version{jobs: [_ | _] = jobs, trusted: trusted} <- [state.running[key]],
          do: {owner, name, jobs, trusted}

    {:reply, scheduled, state}
  end

  def handle_call({:used, owner, names}, _from, state), do: {:reply, used(state, [owner, @network], names, %{}), state}

  # The caller is watched, so a call that dies has ended too.
  def handle_call({:begin, owners, name}, {caller, _}, state) do
    state = behind(state, owners)

    with {at, _key} <- found(state, owners, name),
         {:ok, versions} <- used(state, [at, @network], [name], %{}) do
      call = Process.monitor(caller)
      {:reply, {:ok, call, at, versions}, put_in(state.calls[call], Map.values(versions))}
    else
      absent -> {:reply, absent, state}
    end
  end

  def handle_call({:hold, artifact}, {caller, _}, state) do
    case newest(state, artifact) |> loaded() do
      %Version{} = version ->
        call = Process.monitor(caller)
        {:reply, {:ok, call, version}, put_in(state.calls[call], [version])}

      nil ->
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

  # What runs for `owner` is `installed` now, each plugin's name with its key, and each version in `loaded` is the
  # one its key names: one loaded takes over from one only described. A version that doesn't compile here leaves
  # nothing running by its key, and is said in the log once. A key that runs for nobody this server has looked at no
  # longer runs here. Gives the plugins whose newest version here changed, and whether anything did.
  defp took_over(state, owner, installed, loaded) do
    newest_before = newest_all(state)

    {state, stale} =
      Enum.reduce(loaded, {state, []}, fn
        {key, :error}, {state, stale} ->
          {%{state | broken: MapSet.put(state.broken, key)}, stale}

        {key, {:ok, version}}, {state, stale} ->
          was = state.running[key]

          if match?(%Version{}, was) and (loaded(was) != nil or loaded(version) == nil),
            do: {state, [version | stale]},
            else: {put_in(state.running[key], version), [was | stale]}
      end)

    any = state.installed[owner] != installed
    state = %{state | installed: Map.put(state.installed, owner, installed)}
    state = put_in(state.reloaded[owner], System.monotonic_time(:millisecond))
    keys = for {_owner, installed} <- state.installed, {_name, key} <- installed, into: MapSet.new(), do: key
    {running, dropped} = Map.split_with(state.running, fn {key, _version} -> key in keys end)
    state = %{state | running: running, broken: MapSet.intersection(state.broken, keys)}
    newest_now = newest_all(state)

    changed =
      for artifact <- Enum.uniq(Map.keys(newest_before) ++ Map.keys(newest_now)),
          newest_before[artifact] != newest_now[artifact],
          do: artifact

    any = any or changed != []

    # Which of each owner's plugins hooks into what, and the hooks they offer, are kept for every call to find
    # without asking.
    for {at, installed} <- state.installed,
        at == owner or Enum.any?(installed, fn {_name, {artifact, _, _}} -> artifact in changed end) do
      hooking =
        for {name, key} <- Enum.sort(installed),
            %Version{hooks: hooks} <- [running[key]],
            hook <- hooks,
            reduce: %{} do
          hooking -> Map.update(hooking, hook, [name], &(&1 ++ [name]))
        end

      offered =
        for {name, key} <- installed,
            %Version{offers: offers} <- [running[key]],
            %{name: hook, kind: kind} <- offers,
            into: %{},
            do: {"#{name}.#{hook}", kind}

      :ets.insert(Hooks, {at, hooking, offered})
    end

    {unload_unused(state, stale ++ Map.values(dropped)), changed, any}
  end

  # The newest version of each plugin running here, by its artifact, as its number and whether it's only described.
  defp newest_all(state) do
    for {{artifact, number, _trusted}, %Version{} = version} <- state.running, reduce: %{} do
      newest ->
        Map.update(newest, artifact, {number, loaded(version) == nil}, &max(&1, {number, loaded(version) == nil}))
    end
  end

  # The newest version of the plugin `artifact` running here, loaded where one is, or `nil`. A plugin's processes and
  # services run in it, whoever they're for.
  defp newest(state, artifact) do
    versions = for {{^artifact, number, _trusted}, %Version{} = version} <- state.running, do: {loaded(version) != nil, number, version}

    case Enum.sort_by(versions, fn {loaded, number, _} -> {loaded, number} end, :desc) do
      [{_loaded, _number, version} | _] -> version
      [] -> nil
    end
  end

  # A version that runs here, or `nil` for one only described.
  defp loaded(%Version{} = version),
    do: if(Version.described?(version) and not untrusted?(version), do: nil, else: version)

  defp loaded(nil), do: nil

  defp untrusted?(version), do: match?(%Version{trusted: false}, version)
  # The versions running of the plugins `installed`, by their names there.
  defp running(state, installed) do
    for {name, key} <- installed, %Version{} = version <- [state.running[key]], into: %{}, do: {name, version}
  end

  # Who the plugin `name` was installed for and the plugin, as the first of `owners` that has one by that name finds
  # it. One of them this server hasn't looked at yet is looked at first.
  defp found(_state, [], name), do: {:absent, name}

  defp found(state, [owner | owners], name) do
    case state.installed do
      %{^owner => %{^name => key}} -> {owner, key}
      %{^owner => _added} -> found(state, owners, name)
      _ -> {:absent, name}
    end
  end

  # The versions a call to `names` holds, by their plugins' names as `owners` find them, or the first plugin that
  # isn't running.
  defp used(_state, _owners, [], found), do: {:ok, found}
  defp used(state, owners, [name | names], found) when is_map_key(found, name), do: used(state, owners, names, found)

  defp used(state, owners, [name | names], found) do
    with {_at, key} <- found(state, owners, name),
         %{^key => %Version{} = version} <- state.running do
      used(state, owners, version.uses ++ names, Map.put(found, name, version))
    else
      _ -> {:absent, name}
    end
  end

  defp ended(state, call) do
    {versions, calls} = Map.pop(state.calls, call, [])
    unload_unused(%{state | calls: calls}, versions)
  end

  # A version nothing runs or holds any more is unloaded. Two keys may name the same version described, which is
  # kept while either does.
  defp unload_unused(state, versions) do
    held = MapSet.new(Map.values(state.running) ++ Enum.concat(Map.values(state.calls)), &under/1)
    unused = for %Version{} = version <- Enum.uniq(versions), under(version) not in held, do: version
    Enum.each(unused, &Version.unload/1)
    state
  end

  defp under(%Version{under: under}), do: under
  defp under(_none), do: nil
end
