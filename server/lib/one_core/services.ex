defmodule OneCore.Services do
  @moduledoc """
  Runs plugins' services on this server, and knows which servers run which.

  A service runs on each server whose owner installed it there. What a server has installed is a blob of its own,
  `servers/<its key>/services`: each service's name with its plugin's. Only that server writes it. Every server
  keeps what the servers acting for its user have installed, as it last read it, so calling a service asks the
  bucket nothing. It reads again when a server tells it to, when whoever asks isn't a server it knows, and when it
  knows of none that runs a service.

  Each service installed here is a process (`OneCore.Services.Instance`) for as long as its plugin has it. One
  that stops is started again, whatever stopped it, and the server carries on meanwhile. This process only keeps
  track: it never waits on a service's code, so one service being slow holds up no other.

  With no plugin that signs users in there is one user, whom every server on the list acts for.
  """
  use GenServer
  require Logger
  alias OneCore.{Bucket, Plugins, Server, Servers}
  alias OneCore.Plugins.Version
  alias OneCore.Services.Instance

  @registry OneCore.Services.Instances
  @supervisor OneCore.Services.Supervisor
  @anyone "users/anyone"

  # The least time between two starts of one service, in milliseconds. What a service needs can be gone for a
  # while (a program that won't build, a device unplugged), and it would otherwise be started as fast as it stops.
  @apart 5_000

  # The processes services run as, the supervisor that starts and ends them, and this.
  def child_spec(_) do
    children = [
      {Registry, keys: :unique, name: @registry},
      {DynamicSupervisor, name: @supervisor},
      %{id: __MODULE__, start: {GenServer, :start_link, [__MODULE__, nil, [name: __MODULE__]]}}
    ]

    %{id: __MODULE__, type: :supervisor, start: {Supervisor, :start_link, [children, [strategy: :rest_for_one]]}}
  end

  @doc "The users and groups this server acts for."
  def acts_for, do: if(acting?(Server.key()), do: [@anyone], else: [])

  @doc "Whether `key` is a server that acts for this network's user."
  def acting?(key), do: GenServer.call(__MODULE__, {:acting?, key})

  @doc "The keys of the servers that run a plugin's service: this one first if it does, then in the list's order."
  def servers(plugin, service), do: GenServer.call(__MODULE__, {:servers, {plugin, service}})

  @doc "Reads again which servers act and what each has installed, and gives both: the keys, and the services by key."
  def look, do: GenServer.call(__MODULE__, :look)

  @doc """
  Every running plugin's services, as the bucket has them now: each one's `plugin`, its `service` name, what it
  `needs` of a machine, and the `servers` that run it.
  """
  def list do
    {acting, installed} = look()

    for {plugin, version} <- Enum.sort(Plugins.running()), %{name: name, needs: needs} <- version.services do
      %{plugin: plugin, service: name, needs: needs, servers: Enum.filter(acting, &({plugin, name} in installed[&1]))}
    end
  end

  @doc """
  Runs a plugin's service on this server from now on. Gives `:ok` once it's running, or `{:error, why}`: also when
  it's installed but stopped as it started, which `why` then says.
  """
  def install(plugin, service) do
    with :ok <- known(plugin, service),
         {:ok, instance} <- GenServer.call(__MODULE__, {:install, {plugin, service}}) do
      tell()

      case instance && Instance.started(instance) do
        :ok ->
          :ok

        stopped ->
          why = with {:stopped, why} <- stopped, do: ". It stopped because: " <> said(plugin, why)
          {:error, "#{service} is installed on this server, but it stopped as it started#{why}"}
      end
    end
  end

  @doc "Stops running a plugin's service on this server."
  def remove(plugin, service) do
    with {:ok, instance} <- GenServer.call(__MODULE__, {:remove, {plugin, service}}) do
      if instance, do: Instance.stop(instance)
      tell()
    end
  end

  @doc """
  A plugin has a newer version, or none: its services here move to it, start, or stop. The ones that were running
  have, when this returns.
  """
  def follow(plugin) do
    # A service whose own code brought the newer version in is the one process that can't wait for itself.
    for instance <- GenServer.call(__MODULE__, {:follow, plugin}), instance != self(), do: Instance.follow(instance)
    :ok
  end

  @doc """
  Calls a function of a plugin's service running here, with the service's process and then `arguments`. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told.
  """
  def run(plugin, service, function, arguments) do
    Plugins.run(plugin, fn %{^plugin => version} ->
      module = Version.service(version, service)
      instance = Instance.whereis(plugin, service)

      cond do
        module == nil ->
          {:error, "#{plugin} has no service named #{service}"}

        instance == nil ->
          {:error, GenServer.call(__MODULE__, {:stopped, {plugin, service}})}

        function = Plugins.exported(module, function, length(arguments) + 1) ->
          {:ok, apply(module, function, [instance | arguments])}

        true ->
          {:error, "#{service} has no function #{function} that takes its process and #{length(arguments)} more"}
      end
    end)
  end

  @doc "The folder on this machine that is a service's own."
  def folder(plugin, service) do
    Path.join([Application.fetch_env!(:one_core, :home), "services", plugin, service])
  end

  defp known(plugin, service) do
    with true <- Plugins.look(plugin),
         %{^plugin => %{services: services}} <- Plugins.running() do
      cond do
        Enum.any?(services, &(&1.name == service)) ->
          :ok

        services == [] ->
          {:error, "#{plugin} has no services"}

        true ->
          {:error, "#{plugin} has no service named #{service}. It has: #{Enum.map_join(services, ", ", & &1.name)}"}
      end
    else
      _ -> {:error, "There's no plugin named #{plugin}"}
    end
  end

  # The other servers acting are told to read again what's installed, and have, or couldn't be reached, when this
  # returns.
  defp tell do
    message = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/services"})
    others = Servers.acting() -- [Server.key()]
    Stream.run(Task.async_stream(others, &Server.ask(&1, message), timeout: :infinity))
  end

  # Why a service stopped, as its plugin's author would read it.
  defp said(plugin, why) do
    why =
      case why do
        {%{__exception__: true} = error, _stacktrace} -> Exception.message(error)
        _ -> Exception.format_exit(why)
      end

    case Plugins.running() do
      %{^plugin => version} -> Version.own_names(why, version)
      _removed -> why
    end
  end

  # `started` says when each service here last started, `monitors` which service each running one is, and
  # `stopped` why each last stopped.
  @impl true
  def init(nil) do
    state = read(%{key: Server.key(), acting: [], installed: %{}, started: %{}, monitors: %{}, stopped: %{}})
    for service <- here(state), do: send(self(), {:start, service})
    {:ok, state}
  end

  @impl true
  def handle_call({:acting?, key}, _from, state) do
    state = if key in state.acting, do: state, else: read(state)
    {:reply, key in state.acting, state}
  end

  def handle_call({:servers, service}, _from, state) do
    state = if running(state, service) == [], do: read(state), else: state
    {:reply, Enum.sort_by(running(state, service), &(&1 != state.key)), state}
  end

  def handle_call(:look, _from, state) do
    state = read(state)
    {:reply, {state.acting, state.installed}, state}
  end

  def handle_call({:install, {plugin, name} = service}, _from, state) do
    state = if state.key in state.acting, do: state, else: read(state)

    cond do
      state.key not in state.acting ->
        {:reply, {:error, "This server acts for nobody yet, so it runs no services"}, state}

      service in here(state) ->
        {:reply, {:ok, Instance.whereis(plugin, name)}, state}

      true ->
        {instance, state} = start(service, write(state, here(state) ++ [service]))
        {:reply, {:ok, instance}, state}
    end
  end

  def handle_call({:remove, {plugin, name} = service}, _from, state) do
    if service in here(state) do
      state = write(state, here(state) -- [service])
      instance = Instance.whereis(plugin, name)
      if instance == nil, do: File.rm_rf!(folder(plugin, name))
      {:reply, {:ok, instance}, state}
    else
      {:reply, {:error, "#{name} of #{plugin} isn't installed on this server"}, state}
    end
  end

  # Gives the plugin's services that are running here, for whoever asked to tell them, and starts the others.
  def handle_call({:follow, plugin}, _from, state) do
    {instances, state} =
      for {^plugin, name} = service <- here(state), reduce: {[], state} do
        {instances, state} ->
          case Instance.whereis(plugin, name) do
            nil -> {instances, elem(start(service, state), 1)}
            instance -> {[instance | instances], state}
          end
      end

    {:reply, instances, state}
  end

  def handle_call({:stopped, {plugin, name} = service}, _from, state) do
    why =
      cond do
        service not in here(state) -> "#{plugin}'s #{name} isn't installed on this server"
        why = state.stopped[service] -> "#{plugin}'s #{name} stopped, and is being started again: #{said(plugin, why)}"
        true -> "#{plugin}'s #{name} is still starting"
      end

    {:reply, why, state}
  end

  @impl true
  def handle_info({:start, service}, state) do
    if service in here(state) and service not in Map.values(state.monitors) do
      {:noreply, elem(start(service, state), 1)}
    else
      {:noreply, state}
    end
  end

  def handle_info({:DOWN, monitor, :process, _instance, why}, state) do
    {{plugin, name} = service, monitors} = Map.pop(state.monitors, monitor)
    state = %{state | monitors: monitors, stopped: Map.put(state.stopped, service, why)}

    if service in here(state) do
      Logger.warning("#{plugin}'s #{name} stopped, and is started again: #{said(plugin, why)}")
      wait = max(0, state.started[service] + @apart - System.monotonic_time(:millisecond))
      Process.send_after(self(), {:start, service}, wait)
    else
      File.rm_rf!(folder(plugin, name))
    end

    {:noreply, state}
  end

  # The services installed on this server.
  defp here(state), do: Map.get(state.installed, state.key, [])

  defp running(state, service), do: for(key <- state.acting, service in Map.get(state.installed, key, []), do: key)

  # Starts a service, and gives its process: none if its plugin isn't running here, or no longer has it. One
  # that outlived this process is already running, and is watched again.
  defp start({plugin, name} = service, state) do
    case DynamicSupervisor.start_child(@supervisor, {Instance, service}) do
      {:ok, instance} ->
        {instance, watch(instance, service, state)}

      {:error, {:already_started, instance}} ->
        {instance, watch(instance, service, state)}

      :ignore ->
        File.rm_rf!(folder(plugin, name))
        {nil, state}
    end
  end

  defp watch(instance, service, state) do
    monitor = Process.monitor(instance)
    started = Map.put(state.started, service, System.monotonic_time(:millisecond))
    %{state | monitors: Map.put(state.monitors, monitor, service), started: started}
  end

  # Reads which servers act, and what each has installed.
  defp read(state) do
    acting = Servers.acting()

    installed =
      for key <- acting, into: %{} do
        case Bucket.read("servers/#{key}/services") do
          {:ok, services, _tag} -> {key, for([plugin, name] <- JSON.decode!(services), do: {plugin, name})}
          :absent -> {key, []}
        end
      end

    %{state | acting: acting, installed: installed}
  end

  defp write(state, services) do
    Servers.say("servers/#{state.key}/services", JSON.encode!(for {plugin, name} <- services, do: [plugin, name]))
    put_in(state.installed[state.key], services)
  end
end
