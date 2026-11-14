defmodule OneCore.Plugins.Process do
  @moduledoc """
  One process of a plugin's that keeps running on this server between calls, in its plugin's newest version: a
  service installed here, or a process a plugin's own code keeps (`OneCore.Call.process/4`). It's a GenServer the
  plugin wrote.

  A GenServer runs one module for life, and each version of a plugin has module names of its own. So this process
  is the GenServer, and hands every callback to the plugin's module in the version it's at. It holds that version
  as a running call does. When the plugin has a newer one it holds that instead, gives what it holds to the new
  module's `code_change/3`, and lets go of the old one, which is unloaded once no call holds it either. The process
  is the same one throughout, so what it has open stays open.

  It's registered by its plugin's name, its module's as the plugin wrote it, and a key that tells it from the
  module's other processes, with the version it's at, so all are known without asking it.
  """
  use GenServer, restart: :temporary
  alias OneCore.Plugins
  alias OneCore.Plugins.Version

  @registry OneCore.Plugins.Processes
  @supervisor OneCore.Plugins.Supervisor

  # How long a process has to move to newer code, or to stop, in milliseconds: what OTP gives a process to end.
  # A plugin's code may never return, and its plugin's next version is what would fix that.
  @within 5_000

  @doc "What runs every plugin's processes: where they're registered, and what starts them."
  def children, do: [{Registry, keys: :unique, name: @registry}, {DynamicSupervisor, name: @supervisor}]

  def start_link({plugin, name, key, _argument} = process) do
    GenServer.start_link(__MODULE__, process, name: {:via, Registry, {@registry, {plugin, name, key}}})
  end

  @doc "The process of a plugin's module running here under `key`, or `nil` if there's none."
  def whereis(plugin, name, key \\ nil) do
    case Registry.lookup(@registry, {plugin, name, key}) do
      [{process, _version}] -> process
      [] -> nil
    end
  end

  @doc """
  The process of a plugin's module running here under `key`, started if there's none: its `init/1` is given
  `argument`. `nil` if the plugin isn't running here, or has no such module.
  """
  def of(plugin, name, key \\ nil, argument \\ nil) do
    with nil <- whereis(plugin, name, key) do
      case DynamicSupervisor.start_child(@supervisor, {__MODULE__, {plugin, name, key, argument}}) do
        {:ok, process} -> process
        {:error, {:already_started, process}} -> process
        :ignore -> nil
      end
    end
  end

  @doc "Every process of a plugin's running here."
  def all(plugin), do: Registry.select(@registry, [{{{plugin, :_, :_}, :"$1", :_}, [], [:"$1"]}])

  @doc "Which one a process is: its plugin's name, its module's, and the version it's at. `nil` if it isn't one."
  def which(process) do
    with [{plugin, name, _key} = registered] <- Registry.keys(@registry, process),
         [{^process, version}] <- Registry.lookup(@registry, registered) do
      {plugin, name, version}
    else
      _ -> nil
    end
  end

  @doc "Waits for a process to have started. Gives `:ok`, or `{:stopped, why}` if it stopped instead."
  def started(process), do: told(process, :started, :infinity)

  @doc """
  Tells a process that its plugin has a newer version, and waits for it to have moved to it. Gives `:ok`, or
  `{:stopped, why}` if it stopped instead: its plugin no longer has it, or it starts afresh in the new code.
  """
  def follow(process), do: told(process, :follow, @within)

  @doc "Stops a process that is no longer to run here, and waits until it has."
  def stop(process), do: told(process, :stop, @within)

  # Tells a process something, and waits for it to say it's done or to stop. One that does neither in time is
  # stopped.
  defp told(process, what, within) do
    monitor = Process.monitor(process)
    GenServer.cast(process, {__MODULE__, what, self(), monitor})

    receive do
      {^monitor, :done} ->
        Process.demonitor(monitor, [:flush])
        :ok

      {:DOWN, ^monitor, :process, _, why} ->
        {:stopped, why}
    after
      within ->
        Process.exit(process, :kill)

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

  # The plugin's own `init/1` runs once this process has started, so that starting one never waits on a plugin's
  # code.
  @impl true
  def init({plugin, name, key, argument}) do
    case held(plugin, name, key) do
      {:ok, held, module} ->
        {:ok, %{plugin: plugin, name: name, key: key, held: held, module: module}, {:continue, {__MODULE__, argument}}}

      :absent ->
        :ignore
    end
  end

  @impl true
  def handle_continue({__MODULE__, argument}, process) do
    case process.module.init(argument) do
      {:ok, state} -> {:noreply, Map.put(process, :state, state)}
      {:ok, state, more} -> {:noreply, Map.put(process, :state, state), more}
      :ignore -> {:stop, :normal, process}
      {:stop, why} -> {:stop, why, process}
      {:error, why} -> {:stop, why, process}
    end
  end

  def handle_continue(continue, process), do: kept(process.module.handle_continue(continue, process.state), process)

  @impl true
  def handle_call(request, from, process), do: kept(process.module.handle_call(request, from, process.state), process)

  @impl true
  def handle_cast({__MODULE__, :started, asked, monitor}, process) do
    send(asked, {monitor, :done})
    {:noreply, process}
  end

  def handle_cast({__MODULE__, :stop, _asked, _monitor}, process), do: {:stop, :normal, process}

  def handle_cast({__MODULE__, :follow, asked, monitor}, %{held: was_held, module: was} = process) do
    case held(process.plugin, process.name, process.key) do
      {:ok, held, ^was} ->
        Plugins.release(held)
        send(asked, {monitor, :done})
        {:noreply, process}

      {:ok, held, module} ->
        # OTP names the code a process is leaving by its `@vsn`, and so does this.
        [vsn] = Keyword.fetch!(was.module_info(:attributes), :vsn)

        # The old version is let go of last, with nothing of it left here: unloading it ends any process that
        # still holds one of its functions.
        case module.code_change(vsn, process.state, nil) do
          {:ok, state} ->
            process = %{process | held: held, module: module, state: state}
            Plugins.release(was_held)
            send(asked, {monitor, :done})
            {:noreply, process}

          {:error, why} ->
            process = Map.delete(%{process | held: held, module: module}, :state)
            Plugins.release(was_held)
            {:stop, {:code_change, why}, process}
        end

      :absent ->
        {:stop, :normal, process}
    end
  end

  def handle_cast(request, process), do: kept(process.module.handle_cast(request, process.state), process)

  @impl true
  def handle_info(message, process), do: kept(process.module.handle_info(message, process.state), process)

  # A process that never started, or couldn't take what it held to newer code, holds nothing to end with. The
  # version is let go of here, so that it's unloaded by the time anyone sees that the process stopped.
  @impl true
  def terminate(why, process) do
    with %{state: state} <- process, do: process.module.terminate(why, state)
    Plugins.release(process.held)
  end

  # Holds the plugin's newest version, says so where this process is registered, and gives its module in it.
  # `:absent` if the plugin isn't running here, or no longer has the module.
  defp held(plugin, name, key) do
    with {:ok, held, %{^plugin => version}} <- Plugins.hold(plugin) do
      case Version.module(version, name) do
        nil ->
          Plugins.release(held)
          :absent

        module ->
          {_, _} = Registry.update_value(@registry, {plugin, name, key}, fn _ -> version end)
          {:ok, held, module}
      end
    else
      {:absent, _name} -> :absent
    end
  end

  # What a callback returned, with the plugin's state where this process keeps it.
  defp kept(returned, process) do
    at =
      case {elem(returned, 0), tuple_size(returned)} do
        {:reply, _} -> 2
        {:noreply, _} -> 1
        {:stop, 3} -> 2
        {:stop, 4} -> 3
      end

    put_elem(returned, at, %{process | state: elem(returned, at)})
  end
end
