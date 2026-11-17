defmodule OneCore.Services do
  @moduledoc """
  Runs plugins' services on this server, and knows which servers run which.

  A service runs on each server whose owner installed it there. What a server has installed is a blob of its own,
  `servers/<its key>/services`: each service's name with its plugin, by its artifact's name. Only that server
  writes it. Every server keeps what the servers it asked about have installed, as it last read it
  (`OneCore.Servers`), so calling a service asks the bucket nothing. It reads again when it looks at whoever the
  servers act for (`OneCore.Server.reload/1`), and when it knows of none that runs a service.

  Each service installed here is a process (`OneCore.Plugins.Process`) for as long as its plugin has it. One that
  stops is started again, whatever stopped it, and the server carries on meanwhile. This process only keeps
  track: it never waits on a service's code, so one service being slow holds up no other.
  """
  @behaviour :gen_statem
  require Logger
  alias OneCore.{Plugins, Server, Servers, User}
  alias OneCore.Plugins.{Calls, Version}

  # The least time between two starts of one service, in milliseconds. What a service needs can be gone for a
  # while (a program that won't build, a device unplugged), and it would otherwise be started as fast as it stops.
  @apart 5_000

  def child_spec(_), do: %{id: __MODULE__, start: {__MODULE__, :start_link, [nil]}}
  def start_link(nil), do: :gen_statem.start_link({:local, __MODULE__}, __MODULE__, nil, [])

  @doc """
  The keys of the servers that run a plugin's service, among those acting for `who`: this one first if it does,
  then in their lists' order.
  """
  def servers(plugin, service, who) do
    running = fn -> for key <- Servers.acting(owner(who)), {plugin, service} in Servers.installed(key), do: key end
    running = with [] <- running.(), :ok <- Servers.forget(), do: running.()
    Enum.sort_by(running, &(&1 != Server.key()))
  end

  @doc """
  The services of the plugins something that runs for `who` finds, as the bucket has them now: each one's
  `plugin`, its `service` name, what it's `called` where a person reads it, what it `needs` of a machine, and the
  `servers` that run it.
  """
  def list(who) do
    Servers.forget()
    acting = Servers.acting(owner(who))

    for {plugin, version} <- Enum.sort(Plugins.running(who)), service <- version.services do
      servers = Enum.filter(acting, &({version.artifact, service.name} in Servers.installed(&1)))
      %{plugin: plugin, service: service.name, called: service.called, needs: service.needs, servers: servers}
    end
  end

  # Whose servers something that runs for `who` is run by.
  defp owner(who), do: User.owner(who) || "network"

  @doc """
  Runs a plugin's service on this server from now on. Gives `:ok` once it's running, or `{:error, why}`: also when
  it's installed but stopped as it started, which `why` then says.
  """
  def install(plugin, service) do
    with :ok <- known(plugin, service),
         {:ok, instance} <- :gen_statem.call(__MODULE__, {:install, {plugin, service}}) do
      Server.notify_servers(Servers.acts_for(Server.key()))

      case instance && Plugins.Process.started(instance) do
        :ok ->
          :ok

        stopped ->
          why = with {:stopped, why} <- stopped, do: ". It stopped because: " <> explained(plugin, why)
          {:error, "#{service} is installed on this server, but it stopped as it started#{why}"}
      end
    end
  end

  @doc "Stops running a plugin's service on this server."
  def remove(plugin, service) do
    with {:ok, instance} <- :gen_statem.call(__MODULE__, {:remove, {plugin, service}}) do
      if instance, do: Plugins.Process.stop(instance)
      Server.notify_servers(Servers.acts_for(Server.key()))
    end
  end

  @doc "A plugin has a newer version: its services installed here that it didn't have before start."
  def start(plugin), do: :gen_statem.call(__MODULE__, {:start, plugin})

  @doc """
  Calls a function of a plugin's service running here, with the service's process and then `arguments`. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told.
  """
  def run(plugin, service, function, arguments) do
    with :absent <- run(plugin, service, function, arguments, Plugins.version(plugin)),
         do: {:error, "#{plugin} isn't running on this server"}
  end

  defp run(_plugin, _service, _function, _arguments, nil), do: :absent

  defp run(plugin, service, function, arguments, _running) do
    Plugins.run(plugin, fn version ->
      module = if Enum.any?(version.services, &(&1.name == service)), do: Version.module(version, service)
      instance = Plugins.Process.whereis(plugin, service)

      cond do
        module == nil ->
          {:error, "#{plugin} has no service named #{service}"}

        instance == nil ->
          {:error, :gen_statem.call(__MODULE__, {:stopped, {plugin, service}})}

        function = Calls.exported(module, function, length(arguments) + 1) ->
          {:ok, apply(module, function, [instance | arguments])}

        true ->
          {:error, "#{service} has no function #{function} that takes its process and #{length(arguments)} more"}
      end
    end)
  end

  @doc "The folder on this machine that is a service's own."
  def folder(plugin, service), do: Path.join([Server.home(), "services", plugin, service])

  defp known(plugin, service) do
    with %{services: services} <- Plugins.version(plugin) do
      cond do
        Enum.any?(services, &(&1.name == service)) ->
          :ok

        services == [] ->
          {:error, "#{plugin} has no services"}

        true ->
          {:error, "#{plugin} has no service named #{service}. It has: #{Enum.map_join(services, ", ", & &1.name)}"}
      end
    else
      _ -> {:error, "#{plugin} isn't running on this server"}
    end
  end

  # Why a service stopped, as its plugin's author would read it.
  defp explained(plugin, why) do
    why =
      case why do
        {%{__exception__: true} = error, _stacktrace} -> Exception.message(error)
        _ -> Exception.format_exit(why)
      end

    case Plugins.version(plugin) do
      nil -> why
      version -> Version.own_names(why, version)
    end
  end

  # The state is each service installed here by its stage:
  #
  # - `:starting`, until its plugin runs here with it;
  # - `{:running, process, monitor, at}`, started at `at`;
  # - `{:restarting, why}`, stopped for `why`, and started again by a timeout named for it, at least `@apart` after
  #   it last started;
  # - `{:removing, monitor}`, no longer installed, while its process stops, and what it built here goes once it has.
  @impl true
  def callback_mode, do: :handle_event_function

  @impl true
  def init(nil) do
    Servers.keep()

    {:ok, Map.new(here(), &{&1, :starting}), nil,
     for(service <- here(), do: {:next_event, :internal, {:start, service}})}
  end

  @impl true
  def handle_event(:internal, {:start, service}, stages, _data), do: {:next_state, start(stages, service), nil}

  def handle_event({:timeout, service}, :start, stages, _data), do: {:next_state, start(stages, service), nil}

  def handle_event({:call, from}, {:install, {plugin, name} = service}, stages, _data) do
    cond do
      Servers.acts_for(Server.key()) == [] ->
        {:keep_state_and_data, [{:reply, from, {:error, "This server acts for nobody yet, so it runs no services"}}]}

      service in here() ->
        {:keep_state_and_data, [{:reply, from, {:ok, Plugins.Process.whereis(plugin, name)}}]}

      true ->
        Servers.install(here() ++ [service])
        stages = start(stages, service)
        {:next_state, stages, nil, [{:reply, from, {:ok, process(stages[service])}}]}
    end
  end

  def handle_event({:call, from}, {:remove, {plugin, name} = service}, stages, _data) do
    if service in here() do
      Servers.install(here() -- [service])
      instance = Plugins.Process.whereis(plugin, name)

      stages =
        if instance do
          Map.put(stages, service, {:removing, Process.monitor(instance)})
        else
          File.rm_rf!(folder(plugin, name))
          Map.delete(stages, service)
        end

      {:next_state, stages, nil, [{:reply, from, {:ok, instance}}, {{:timeout, service}, :cancel}]}
    else
      {:keep_state_and_data, [{:reply, from, {:error, "#{name} of #{plugin} isn't installed on this server"}}]}
    end
  end

  def handle_event({:call, from}, {:start, plugin}, stages, _data) do
    starting = for {^plugin, name} = service <- here(), Plugins.Process.whereis(plugin, name) == nil, do: service
    cancel = for service <- starting, do: {{:timeout, service}, :cancel}
    {:next_state, Enum.reduce(starting, stages, &start(&2, &1)), nil, [{:reply, from, :ok} | cancel]}
  end

  def handle_event({:call, from}, {:stopped, {plugin, name} = service}, stages, _data) do
    why =
      case service in here() && stages[service] do
        false -> "#{plugin}'s #{name} isn't installed on this server"
        {:restarting, why} -> "#{plugin}'s #{name} stopped, and is being started again: #{explained(plugin, why)}"
        _ -> "#{plugin}'s #{name} is still starting"
      end

    {:keep_state_and_data, [{:reply, from, why}]}
  end

  def handle_event(:info, {:DOWN, monitor, :process, _instance, why}, stages, _data) do
    case Enum.find(stages, fn {_service, stage} ->
           match?({:running, _, ^monitor, _}, stage) or match?({:removing, ^monitor}, stage)
         end) do
      {{plugin, name} = service, {:running, _instance, _monitor, at}} ->
        Logger.warning(
          "#{plugin}'s #{name} stopped, and is started again: #{explained(plugin, OneCore.Redact.redact(why))}"
        )

        wait = max(0, at + @apart - System.monotonic_time(:millisecond))
        {:next_state, Map.put(stages, service, {:restarting, why}), nil, [{{:timeout, service}, wait, :start}]}

      {{plugin, name} = service, {:removing, _monitor}} ->
        File.rm_rf!(folder(plugin, name))
        {:next_state, Map.delete(stages, service), nil}

      # One that stopped after another took its place, as when its plugin's newer version started it again.
      nil ->
        :keep_state_and_data
    end
  end

  # The services installed on this server.
  defp here, do: Servers.installed(Server.key())

  defp process({:running, process, _monitor, _at}), do: process
  defp process(_stage), do: nil

  # Starts a service: it's running, or starting while its plugin isn't running here. One whose plugin no longer has
  # it is starting too, until it's removed, and what it built here goes. One that outlived this process is already
  # running, and is watched again.
  defp start(stages, {plugin, name} = service) do
    case Plugins.Process.of(plugin, name) do
      nil ->
        if Plugins.version(plugin), do: File.rm_rf!(folder(plugin, name))
        Map.put(stages, service, :starting)

      instance ->
        Map.put(stages, service, {:running, instance, Process.monitor(instance), System.monotonic_time(:millisecond)})
    end
  end
end
