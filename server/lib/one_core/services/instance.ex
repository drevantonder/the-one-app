defmodule OneCore.Services.Instance do
  @moduledoc """
  One service running on this server: the process that is its GenServer, in its plugin's newest version.

  A GenServer runs one module for life, and each version of a plugin has module names of its own. So this process
  is the GenServer, and hands every callback to the service's module in the version it's at. It holds that
  version as a running call does. When the plugin has a newer one it holds that instead, gives what the service
  holds to the new module's `code_change/3`, and lets go of the old one, which is unloaded once no call holds it
  either. The process is the same one throughout, so what it has open stays open.

  It's registered by its plugin's name and its own, with the version it's at, so both are known without asking it.
  """
  use GenServer, restart: :temporary
  alias OneCore.Plugins
  alias OneCore.Plugins.Version

  @registry OneCore.Services.Instances

  # How long a service has to move to newer code, or to stop, in milliseconds: what OTP gives a process to end.
  # A service's code may never return, and its plugin's next version is what would fix that.
  @within 5_000

  def start_link({plugin, service}) do
    GenServer.start_link(__MODULE__, {plugin, service}, name: {:via, Registry, {@registry, {plugin, service}}})
  end

  @doc "The process running a plugin's service here, or `nil` if it isn't running."
  def whereis(plugin, service) do
    case Registry.lookup(@registry, {plugin, service}) do
      [{instance, _version}] -> instance
      [] -> nil
    end
  end

  @doc "Which service a process is: its plugin's name, its own, and the version it's at. `nil` if it isn't one."
  def which(instance) do
    with [{plugin, service}] <- Registry.keys(@registry, instance),
         [{^instance, version}] <- Registry.lookup(@registry, {plugin, service}) do
      {plugin, service, version}
    else
      _ -> nil
    end
  end

  @doc "Waits for a service to have started. Gives `:ok`, or `{:stopped, why}` if it stopped instead."
  def started(instance), do: told(instance, :started, :infinity)

  @doc """
  Tells a service that its plugin has a newer version, and waits for it to have moved to it. Gives `:ok`, or
  `{:stopped, why}` if it stopped instead, to start afresh in the new code.
  """
  def follow(instance), do: told(instance, :follow, @within)

  @doc "Stops a service that is no longer to run here, and waits until it has."
  def stop(instance), do: told(instance, :stop, @within)

  # Tells a service something, and waits for it to say it's done or to stop. One that does neither in time is
  # stopped.
  defp told(instance, what, within) do
    monitor = Process.monitor(instance)
    GenServer.cast(instance, {__MODULE__, what, self(), monitor})

    receive do
      {^monitor, :done} ->
        Process.demonitor(monitor, [:flush])
        :ok

      {:DOWN, ^monitor, :process, _, why} ->
        {:stopped, why}
    after
      within ->
        Process.exit(instance, :kill)

        receive do
          {:DOWN, ^monitor, :process, _, why} ->
            # It may have said it was done just as its time ran out.
            receive do
              {^monitor, :done} -> {:stopped, why}
            after
              0 -> {:stopped, why}
            end
        end
    end
  end

  # The service's own `init/1` runs once this process has started, so that starting one never waits on a
  # service's code.
  @impl true
  def init({plugin, service}) do
    case held(plugin, service) do
      {:ok, held, module} ->
        {:ok, %{plugin: plugin, service: service, held: held, module: module}, {:continue, __MODULE__}}

      :absent ->
        :ignore
    end
  end

  @impl true
  def handle_continue(__MODULE__, instance) do
    case instance.module.init(nil) do
      {:ok, state} -> {:noreply, Map.put(instance, :state, state)}
      {:ok, state, more} -> {:noreply, Map.put(instance, :state, state), more}
      :ignore -> {:stop, :normal, instance}
      {:stop, why} -> {:stop, why, instance}
      {:error, why} -> {:stop, why, instance}
    end
  end

  def handle_continue(continue, instance), do: kept(instance.module.handle_continue(continue, instance.state), instance)

  @impl true
  def handle_call(request, from, instance),
    do: kept(instance.module.handle_call(request, from, instance.state), instance)

  @impl true
  def handle_cast({__MODULE__, :started, asked, monitor}, instance) do
    send(asked, {monitor, :done})
    {:noreply, instance}
  end

  def handle_cast({__MODULE__, :stop, _asked, _monitor}, instance), do: {:stop, :normal, instance}

  def handle_cast({__MODULE__, :follow, asked, monitor}, %{held: was_held, module: was} = instance) do
    case held(instance.plugin, instance.service) do
      {:ok, held, ^was} ->
        Plugins.release(held)
        send(asked, {monitor, :done})
        {:noreply, instance}

      {:ok, held, module} ->
        # OTP names the code a process is leaving by its `@vsn`, and so does this.
        [vsn] = Keyword.fetch!(was.module_info(:attributes), :vsn)

        # The old version is let go of last, with nothing of it left here: unloading it ends any process that
        # still holds one of its functions.
        case module.code_change(vsn, instance.state, nil) do
          {:ok, state} ->
            instance = %{instance | held: held, module: module, state: state}
            Plugins.release(was_held)
            send(asked, {monitor, :done})
            {:noreply, instance}

          {:error, why} ->
            instance = Map.delete(%{instance | held: held, module: module}, :state)
            Plugins.release(was_held)
            {:stop, {:code_change, why}, instance}
        end

      :absent ->
        {:stop, :normal, instance}
    end
  end

  def handle_cast(request, instance), do: kept(instance.module.handle_cast(request, instance.state), instance)

  @impl true
  def handle_info(message, instance), do: kept(instance.module.handle_info(message, instance.state), instance)

  # A service that never started, or couldn't take what it held to newer code, holds nothing to end with. The
  # version is let go of here, so that it's unloaded by the time anyone sees that the service stopped.
  @impl true
  def terminate(why, instance) do
    with %{state: state} <- instance, do: instance.module.terminate(why, state)
    Plugins.release(instance.held)
  end

  # Holds the plugin's newest version, says so where this process is registered, and gives the service's module
  # in it. `:absent` if the plugin isn't running here, or no longer has the service.
  defp held(plugin, service) do
    with {:ok, held, %{^plugin => version}} <- Plugins.hold(plugin) do
      case Version.service(version, service) do
        nil ->
          Plugins.release(held)
          :absent

        module ->
          {_, _} = Registry.update_value(@registry, {plugin, service}, fn _ -> version end)
          {:ok, held, module}
      end
    else
      {:absent, _name} -> :absent
    end
  end

  # What a callback returned, with the service's state where this process keeps it.
  defp kept(returned, instance) do
    at =
      case {elem(returned, 0), tuple_size(returned)} do
        {:reply, _} -> 2
        {:noreply, _} -> 1
        {:stop, 3} -> 2
        {:stop, 4} -> 3
      end

    put_elem(returned, at, %{instance | state: elem(returned, at)})
  end
end
