defmodule OneCore.Services do
  @moduledoc """
  Runs apps' services on this server, and knows which servers run which.

  A service runs on each server whose owner installed it there. What a server has installed is a blob of its own,
  `servers/<its key>/services`: each service's name with its app, by its artifact's name. Only that server
  writes it. Every server keeps what the servers it asked about have installed, as it last read it
  (`OneCore.Servers`), so calling a service asks the bucket nothing. It reads again when it looks at whoever the
  servers act for (`OneCore.Apps.reload/1`), and for whoever it knows of no server running a service for.

  Each service installed here is a process (`OneCore.Apps.Process`) for as long as its app has it. One that
  stops is started again, whatever stopped it, and the server carries on meanwhile. This process only keeps
  track: it never waits on a service's code, so one service being slow holds up no other.
  """
  @behaviour :gen_statem
  require Logger
  alias OneCore.{Apps, Server, Servers, User}
  alias OneCore.Apps.{Calls, Installed, Version}

  # The least time between two starts of one service, in milliseconds. What a service needs can be gone for a
  # while (a program that won't build, a device unplugged), and it would otherwise be started as fast as it stops.
  @apart 5_000

  def child_spec(_), do: %{id: __MODULE__, start: {__MODULE__, :start_link, [nil]}}
  def start_link(nil), do: :gen_statem.start_link({:local, __MODULE__}, __MODULE__, nil, [])

  @doc """
  The keys of the servers that run an app's service, among those acting for `who`: this one first if it does,
  then in their lists' order.
  """
  def servers(app, service, who) do
    running = fn -> for key <- OneCore.Apps.covering(owner(who)), {app, service} in Servers.installed(key), do: key end
    running = with [] <- running.(), :ok <- reload(who), do: running.()
    Enum.sort_by(running, &(&1 != Server.key()))
  end

  @doc """
  The services of the apps something that runs for `who` finds, as the bucket has them now: each one's
  `app`, its `service` name, what it's `called` where a person reads it, what it `needs` of a machine, the `os` it
  runs on (`nil` for any), and the `servers` that run it.
  """
  def list(who) do
    reload(who)
    acting = OneCore.Apps.covering(owner(who))

    for {app, version} <- Enum.sort(Apps.running(who)), service <- version.services do
      servers = Enum.filter(acting, &({version.artifact, service.name} in Servers.installed(&1)))
      %{app: app, service: service.name, called: service.called, needs: service.needs, os: Map.get(service, :os), servers: servers}
    end
  end

  # Invalidates what's cached of the servers acting for `who`, which is all that a service running depends on.
  defp reload(who), do: Enum.each(Enum.uniq([owner(who), "network"]), &Servers.reload/1)

  # Whose servers something that runs for `who` is run by.
  defp owner(who), do: User.owner(who) || "network"

  @doc """
  Runs an app's service on this server from now on. Gives `:ok` once it's running, or `{:error, why}`: also when
  it's installed but stopped as it started, which `why` then says.
  """
  def install(app, service) do
    with :ok <- known(app, service),
         {:ok, instance} <- call({:install, {app, service}}) do
      Server.notify_servers(Servers.here())

      case instance && Apps.Process.started(instance) do
        :ok ->
          :ok

        stopped ->
          why = with {:stopped, why} <- stopped, do: ". It stopped because: " <> explained(app, why)
          {:error, "#{service} is installed on this server, but it stopped as it started#{why}"}
      end
    end
  end

  @doc "Stops running an app's service on this server."
  def remove(app, service) do
    with {:ok, instance} <- call({:remove, {app, service}}) do
      if instance, do: Apps.Process.stop(instance, {:shutdown, :removed})
      Server.notify_servers(Servers.here())
    end
  end

  @doc """
  An app has a newer version, or runs for someone else here: its services installed here that it didn't have before
  start. Where it's a copy of an app that's superseded by another (`OneCore.Apps.Lifecycle.supersede/2`), or the
  copy that superseded one, each service of the superseded copy installed here moves to the one that stays, with its
  folder, as its installs did.
  """
  def start(app) do
    moving = call({:supersede, app})

    for {{copy, name}, to, instance} <- moving do
      if instance, do: Apps.Process.stop(instance, :shutdown)
      {from, into} = {folder(copy, name), folder(to, name)}

      if File.exists?(from) and not File.exists?(into) do
        File.mkdir_p!(Path.dirname(into))
        File.rename!(from, into)
      end
    end

    for app <- Enum.uniq([app | for({_service, to, _instance} <- moving, do: to)]), do: call({:start, app})
    :ok
  end

  @doc """
  Calls a function of an app's service running here, with the service's process and then `arguments`. Gives
  `{:ok, answer}`, or `{:error, why}` with what the caller is told.
  """
  def run(app, service, function, arguments) do
    with :absent <- run(app, service, function, arguments, Apps.version(app)),
         do: {:error, "#{app} isn't running on this server"}
  end

  defp run(_app, _service, _function, _arguments, nil), do: :absent

  defp run(app, service, function, arguments, _running) do
    Apps.run(app, fn version ->
      module = if Enum.any?(version.services, &(&1.name == service)), do: Version.module(version, service)
      instance = Apps.Process.whereis(app, service)

      cond do
        module == nil ->
          {:error, "#{app} has no service named #{service}"}

        instance == nil ->
          {:error, call({:stopped, {app, service}})}

        function = Calls.exported(module, function, length(arguments) + 1) ->
          {:ok, apply(module, function, [instance | arguments])}

        true ->
          {:error, "#{service} has no function #{function} that takes its process and #{length(arguments)} more"}
      end
    end)
  end

  @doc "The folder on this machine that is a service's own."
  def folder(app, service), do: Path.join([Server.home(), "services", app, service])

  # The system this server runs on, as npm's `os` names it.
  defp system do
    case :os.type() do
      {:win32, _} -> "win32"
      {:unix, name} -> Atom.to_string(name)
    end
  end

  defp runs_here(%{os: [_ | _] = os}, service) do
    if system() in os, do: :ok, else: {:error, "#{service} runs on #{Enum.join(os, ", ")}, and this server runs on #{system()}"}
  end

  defp runs_here(_service, _name), do: :ok

  defp known(app, service) do
    with %{services: services} <- Apps.version(app) do
      cond do
        Enum.any?(services, &(&1.name == service)) ->
          runs_here(Enum.find(services, &(&1.name == service)), service)

        services == [] ->
          {:error, "#{app} has no services"}

        true ->
          {:error, "#{app} has no service named #{service}. It has: #{Enum.map_join(services, ", ", & &1.name)}"}
      end
    else
      _ -> {:error, "#{app} isn't running on this server"}
    end
  end

  # Why a service stopped, as its app's author would read it.
  defp explained(app, why) do
    why =
      case why do
        {%{__exception__: true} = error, _stacktrace} -> Exception.message(error)
        _ -> Exception.format_exit(why)
      end

    case Apps.version(app) do
      nil -> why
      version -> Version.own_names(why, version)
    end
  end

  # The state is each service installed here by its stage:
  #
  # - `:starting`, until its app runs here with it;
  # - `{:running, process, monitor, at}`, started at `at`;
  # - `{:restarting, why}`, stopped for `why`, and started again by a timeout named for it, at least `@apart` after
  #   it last started;
  # - `{:removing, monitor}`, no longer installed, while its process stops, and what it built here goes once it has.
  @impl true
  def callback_mode, do: :handle_event_function

  @impl true
  def init(nil) do
    {:ok, Map.new(here(), &{&1, :starting}), nil,
     for(service <- here(), do: {:next_event, :internal, {:start, service}})}
  end

  @impl true
  def handle_event(:internal, {:start, service}, stages, _data), do: {:next_state, start(stages, service), nil}

  def handle_event({:timeout, service}, :start, stages, _data), do: {:next_state, start(stages, service), nil}

  # What's asked of this reads which services are installed here from the bucket, through the edge, which may not
  # answer for a while (`OneCore.Edge.Unreachable`): that fails whoever asked, saying why (`call/1`), and not this.
  def handle_event({:call, from}, ask, stages, _data) do
    called(from, ask, stages)
  rescue
    unreachable in OneCore.Edge.Unreachable -> {:keep_state_and_data, [{:reply, from, {:unreachable, unreachable}}]}
  end

  def handle_event(:info, {:DOWN, monitor, :process, _instance, why}, stages, _data) do
    case Enum.find(stages, fn {_service, stage} ->
           match?({:running, _, ^monitor, _}, stage) or match?({:removing, ^monitor}, stage)
         end) do
      {{app, name} = service, {:running, _instance, _monitor, at}} ->
        Logger.warning(
          "#{app}'s #{name} stopped, and is started again: #{explained(app, OneCore.Redact.redact(why))}"
        )

        wait = max(0, at + @apart - System.monotonic_time(:millisecond))
        {:next_state, Map.put(stages, service, {:restarting, why}), nil, [{{:timeout, service}, wait, :start}]}

      {{app, name} = service, {:removing, _monitor}} ->
        File.rm_rf!(folder(app, name))
        {:next_state, Map.delete(stages, service), nil}

      # One that stopped after another took its place, as when its app's newer version started it again.
      nil ->
        :keep_state_and_data
    end
  end

  defp called(from, {:install, {app, name} = service}, stages) do
    cond do
      Servers.here() == [] ->
        {:keep_state_and_data, [{:reply, from, {:error, "This server acts for nobody yet, so it runs no services"}}]}

      service in here() ->
        {:keep_state_and_data, [{:reply, from, {:ok, Apps.Process.whereis(app, name)}}]}

      true ->
        Servers.install(here() ++ [service])
        stages = start(stages, service)
        {:next_state, stages, nil, [{:reply, from, {:ok, process(stages[service])}}]}
    end
  end

  defp called(from, {:remove, {app, name} = service}, stages) do
    if service in here() do
      Servers.install(here() -- [service])
      instance = Apps.Process.whereis(app, name)

      stages =
        if instance do
          Map.put(stages, service, {:removing, Process.monitor(instance)})
        else
          File.rm_rf!(folder(app, name))
          Map.delete(stages, service)
        end

      {:next_state, stages, nil, [{:reply, from, {:ok, instance}}, {{:timeout, service}, :cancel}]}
    else
      {:keep_state_and_data, [{:reply, from, {:error, "#{name} of #{app} isn't installed on this server"}}]}
    end
  end

  # The services installed here of a copy of `app` that's in use for nobody now, while another copy runs by its name for
  # whoever this server acts for, with that service: each moves to that one, and is no longer watched as the copy's.
  defp called(from, {:supersede, app}, stages) do
    moving =
      for {copy, name} = service <- here(),
          Path.basename(copy) == Path.basename(app),
          to = successor(copy, name),
          to != nil,
          Installed.in_use_for(copy) == [],
          do: {service, to}

    moved = Map.new(moving)
    if moving != [], do: Servers.install(Enum.uniq(for({_copy, name} = service <- here(), do: if(to = moved[service], do: {to, name}, else: service))))

    for {service, _to} <- moving, match?({:running, _, _, _}, stages[service]),
        do: Process.demonitor(elem(stages[service], 2), [:flush])

    replies = for {service, to} <- moving, do: {service, to, process(stages[service])}
    cancel = for {service, _to} <- moving, do: {{:timeout, service}, :cancel}
    {:next_state, Map.drop(stages, Map.keys(moved)), nil, [{:reply, from, replies} | cancel]}
  end

  defp called(from, {:start, app}, stages) do
    starting = for {^app, name} = service <- here(), Apps.Process.whereis(app, name) == nil, do: service
    cancel = for service <- starting, do: {{:timeout, service}, :cancel}
    {:next_state, Enum.reduce(starting, stages, &start(&2, &1)), nil, [{:reply, from, :ok} | cancel]}
  end

  defp called(from, {:stopped, {app, name} = service}, stages) do
    why =
      case service in here() && stages[service] do
        false -> "#{app}'s #{name} isn't installed on this server"
        {:restarting, why} -> "#{app}'s #{name} stopped, and is being started again: #{explained(app, why)}"
        _ -> "#{app}'s #{name} is still starting"
      end

    {:keep_state_and_data, [{:reply, from, why}]}
  end

  defp call(ask) do
    case :gen_statem.call(__MODULE__, ask) do
      {:unreachable, unreachable} -> raise unreachable
      answer -> answer
    end
  end

  # The services installed on this server.
  defp here, do: Servers.installed(Server.key())

  # The other copy of the app `copy` that runs by its name for whoever this server acts for, and has the service
  # `name`, or `nil`.
  defp successor(copy, name) do
    Enum.find_value(Servers.here(), fn owner ->
      case Apps.running(owner)[Path.basename(copy)] do
        %Version{artifact: to, services: services} when to != copy ->
          if Enum.any?(services, &(&1.name == name)), do: to

        _ ->
          nil
      end
    end)
  end

  defp process({:running, process, _monitor, _at}), do: process
  defp process(_stage), do: nil

  # Starts a service: it's running, or starting while its app isn't running here. One whose app no longer has
  # it is starting too, until it's removed, and what it built here goes. One that outlived this process is already
  # running, and is watched again.
  defp start(stages, {app, name} = service) do
    case Apps.Process.of(app, name) do
      nil ->
        if Apps.version(app), do: File.rm_rf!(folder(app, name))
        Map.put(stages, service, :starting)

      instance ->
        Map.put(stages, service, {:running, instance, Process.monitor(instance), System.monotonic_time(:millisecond)})
    end
  end
end
