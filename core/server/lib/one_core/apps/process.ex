defmodule OneCore.Apps.Process do
  @moduledoc """
  One process of an app's that keeps running on this server between calls, in its app's latest version: a
  service installed here, or a process an app's own code keeps (`OneCore.Call.process/4`). It's a GenServer or a
  `:gen_statem` the app wrote.

  A GenServer or a `:gen_statem` runs one module for life, and each version of an app has module names of its own.
  So this process is the GenServer, or `OneCore.Apps.Process.Statem` the `:gen_statem`, and hands every callback
  to the app's module in the version it's at. It holds that version as a running call does. When the app has
  a newer one it holds that instead, gives what it holds to the new module's `code_change`, and lets go of the old
  one, which is unloaded once no call holds it either. The process is the same one throughout, so what it has open
  stays open. One whose module is the other behaviour in the newer version stops, and whatever asks for it next
  starts it afresh.

  It's registered by its app, its module's name as the app wrote it, and a key that tells it from the
  module's other processes, with the version it's at, so all are known without asking it.
  """
  use GenServer, restart: :temporary
  alias OneCore.Apps
  alias OneCore.Apps.{LoadedVersion, Version}

  @registry OneCore.Apps.Processes
  @supervisor OneCore.Apps.Supervisor

  # How long a process has to move to newer code, or to stop, in milliseconds: what OTP gives a process to end.
  # An app's code may never return, and its app's next version is what would fix that.
  @within 5_000

  @doc "What runs every app's processes: where they're registered, and what starts them."
  def children, do: [{Registry, keys: :unique, name: @registry}, {DynamicSupervisor, name: @supervisor}]

  def start_link(process), do: :proc_lib.start_link(__MODULE__, :init, [process])

  @doc "The process of an app's module running here under `key`, or `nil` if there's none."
  def whereis(app, name, key \\ nil) do
    # A process that has just ended stays in the registry until it hears so, and `Registry.lookup/2` still gives it.
    with :undefined <- Registry.whereis_name({@registry, {app, name, key}}), do: nil
  end

  @doc """
  The process of an app's module running here under `key`, started if there's none: its `init/1` is given
  `argument`. `nil` if the app isn't running here, or has no such module.
  """
  def of(app, name, key \\ nil, argument \\ nil) do
    with nil <- whereis(app, name, key) do
      case DynamicSupervisor.start_child(@supervisor, {__MODULE__, {app, name, key, argument}}) do
        {:ok, process} -> process
        {:error, {:already_started, process}} -> process
        :ignore -> nil
      end
    end
  end

  @doc "Every process of an app's running here."
  def all(app), do: Registry.select(@registry, [{{{app, :_, :_}, :"$1", :_}, [], [:"$1"]}])

  @doc "Which one a process is: its app's name, its module's, and the version it's at. `nil` if it isn't one."
  def which(process) do
    with [{app, name, _key} = registered] <- Registry.keys(@registry, process),
         [{^process, version}] <- Registry.lookup(@registry, registered) do
      {app, name, version}
    else
      _ -> nil
    end
  end

  @doc "Waits for a process to have started. Gives `:ok`, or `{:stopped, why}` if it stopped instead."
  def started(process), do: request(process, :started, :infinity)

  @doc """
  Tells a process that its app has a newer version, and waits for it to have moved to it. Gives `:ok`, or
  `{:stopped, why}` if it stopped instead: its app no longer has it, or it starts afresh in the new code.
  """
  def follow(process), do: request(process, :follow, @within)

  @doc "Stops a process that is no longer to run here, and waits until it has."
  def stop(process), do: request(process, :stop, @within)

  # Tells a process something, and waits for it to say it's done or to stop. One that does neither in time is
  # stopped.
  defp request(process, what, within) do
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

  # A process knows which behaviour it is only once it holds its app's version. So it starts as OTP has one with
  # more to do first, and as a Ranch protocol does: it says it has started, so that starting one never waits on a
  # app's code, runs the app's `init/1`, and enters the loop of a GenServer or of a `:gen_statem`.
  @impl true
  def init({app, name, key, argument}) do
    registered = {@registry, {app, name, key}}

    with :yes <- Registry.register_name(registered, self()),
         {:ok, held, module} <- held(app, name, key) do
      :proc_lib.init_ack({:ok, self()})
      # What it logs is its app's, as a call's is (`OneCore.Apps.Calls.running/2`), for no call in particular.
      Logger.metadata(who: app)
      process = %{app: app, name: name, key: key, held: held, module: module}
      registered = {:via, Registry, registered}

      if statem?(module) do
        __MODULE__.Statem.init({process, argument, registered})
      else
        case returned(fn -> module.init(argument) end) do
          {:ok, state} -> enter(process, state, :infinity, registered)
          {:ok, state, more} -> enter(process, state, more, registered)
          refused -> ended(process, refused)
        end
      end
    else
      :no -> :proc_lib.init_fail({:error, {:already_started, whereis(app, name, key)}}, {:exit, :normal})
      :absent -> :proc_lib.init_fail(:ignore, {:exit, :normal})
    end
  end

  defp enter(process, state, more, registered),
    do: :gen_server.enter_loop(__MODULE__, [], Map.put(process, :state, state), registered, more)

  @impl true
  def handle_continue(continue, process), do: forward(process, :handle_continue, [continue])

  @impl true
  def handle_call(request, from, process), do: forward(process, :handle_call, [request, from])

  @impl true
  def handle_cast({__MODULE__, :started, from, monitor}, process) do
    send(from, {monitor, :done})
    {:noreply, process}
  end

  def handle_cast({__MODULE__, :stop, _from, _monitor}, process), do: {:stop, :normal, process}

  def handle_cast({__MODULE__, :follow, from, monitor}, %{held: was_held, module: was} = process) do
    case held(process.app, process.name, process.key) do
      {:ok, held, ^was} ->
        LoadedVersion.release([held])
        send(from, {monitor, :done})
        {:noreply, process}

      {:ok, held, module} ->
        if statem?(module) do
          # It ends as it ran, in the old code.
          LoadedVersion.release([held])
          {:stop, :normal, process}
        else
          # The old version is let go of last, with nothing of it left here: unloading it ends any process that
          # still holds one of its functions.
          case returned(fn -> module.code_change(vsn(was), process.state, nil) end) do
            {:ok, state} ->
              process = %{process | held: held, module: module, state: state}
              LoadedVersion.release([was_held])
              send(from, {monitor, :done})
              {:noreply, process}

            {:error, why} ->
              process = Map.delete(%{process | held: held, module: module}, :state)
              LoadedVersion.release([was_held])
              {:stop, {:code_change, why}, process}
          end
        end

      :absent ->
        {:stop, :normal, process}
    end
  end

  def handle_cast(request, process), do: forward(process, :handle_cast, [request])

  @impl true
  def handle_info(message, process), do: forward(process, :handle_info, [message])

  # A process that couldn't take what it held to newer code holds nothing to end with. The version is let go of
  # here, so that it's unloaded by the time anyone sees that the process stopped.
  @impl true
  def terminate(why, process) do
    with %{state: state} <- process, do: process.module.terminate(why, state)
    LoadedVersion.release([process.held])
  end

  @doc false
  # Holds the app's latest version, says so where this process is registered, and gives its module in it.
  # `:absent` if the app isn't running here, or no longer has the module.
  def held(app, name, key) do
    with {:ok, version} <- Apps.hold(app) do
      case Version.module(version, name) do
        nil ->
          LoadedVersion.release([version])
          :absent

        module ->
          {_, _} = Registry.update_value(@registry, {app, name, key}, fn _ -> version end)
          {:ok, version, module}
      end
    end
  end

  @doc false
  def statem?(module), do: :gen_statem in Version.behaviours(module)

  @doc false
  # OTP names the code a process is leaving by its `@vsn`, and so does this.
  def vsn(module), do: hd(Keyword.fetch!(module.module_info(:attributes), :vsn))

  @doc false
  # What an app's callback returned, thrown too, as OTP takes it.
  def returned(callback) do
    callback.()
  catch
    :throw, returned -> returned
  end

  @doc false
  # A process whose app's `init/1` didn't start it lets go of its version and ends, as OTP has one end.
  def ended(process, returned) do
    LoadedVersion.release([process.held])

    case returned do
      :ignore -> exit(:normal)
      {stop_or_error, why} when stop_or_error in [:stop, :error] -> exit(why)
      returned -> exit({:bad_return_value, returned})
    end
  end

  # Hands a callback to the app's module, with its state, and keeps the state it returns where this process keeps
  # it.
  defp forward(process, callback, arguments) do
    returned = returned(fn -> apply(process.module, callback, arguments ++ [process.state]) end)

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
