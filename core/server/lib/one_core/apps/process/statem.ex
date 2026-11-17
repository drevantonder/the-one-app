defmodule OneCore.Apps.Process.Statem do
  @moduledoc """
  An app's process whose module is a `:gen_statem` (`OneCore.Apps.Process`). It's a `:gen_statem` whose state
  is the app's, so postponing, state enter calls and every kind of timeout are OTP's own, and it hands every
  event to the app's module in the version it's at, in the callback mode that module says. It stays the callback
  module, so an app's module doesn't change it (`change_callback_module` and `push_callback_module`).
  """
  @behaviour :gen_statem
  alias OneCore.Apps.{LoadedVersion, Process}

  # Entered from `OneCore.Apps.Process.init/1` once the process holds its version: it runs the app's `init/1`
  # and enters the loop with what that gave, as a Ranch protocol's `init/1` does.
  @impl true
  def init({process, argument, registered}) do
    case Process.returned(fn -> process.module.init(argument) end) do
      {:ok, state, data} -> enter(process, state, data, [], registered)
      {:ok, state, data, actions} -> enter(process, state, data, actions, registered)
      refused -> Process.ended(process, refused)
    end
  end

  defp enter(process, state, data, actions, registered),
    do: :gen_statem.enter_loop(__MODULE__, [], state, Map.put(process, :data, data), registered, actions)

  # Each state enter call is made here, and handed on to a module that asks for them.
  @impl true
  def callback_mode, do: [:handle_event_function, :state_enter]

  @impl true
  def handle_event(:cast, {Process, :started, from, monitor}, _state, _process) do
    send(from, {monitor, :done})
    :keep_state_and_data
  end

  def handle_event(:cast, {Process, :stop, _from, _monitor}, _state, _process), do: {:stop, :normal}

  def handle_event(:cast, {Process, :follow, from, monitor}, state, %{held: was_held, module: was} = process) do
    case Process.held(process.app, process.name, process.key) do
      {:ok, held, ^was} ->
        LoadedVersion.release([held])
        send(from, {monitor, :done})
        :keep_state_and_data

      {:ok, held, module} ->
        if Process.statem?(module) do
          # The old version is let go of last, as a GenServer's is. A state other than the one it was in is entered
          # as any other: its state timeout ends, and what was postponed is handled again.
          case code_changed(module, Process.vsn(was), state, process.data) do
            {:ok, state, data} ->
              process = %{process | held: held, module: module, data: data}
              LoadedVersion.release([was_held])
              send(from, {monitor, :done})
              {:next_state, state, process}

            why ->
              process = Map.delete(%{process | held: held, module: module}, :data)
              LoadedVersion.release([was_held])
              {:stop, {:code_change, why}, process}
          end
        else
          # It ends as it ran, in the old code.
          LoadedVersion.release([held])
          {:stop, :normal}
        end

      :absent ->
        {:stop, :normal}
    end
  end

  def handle_event(type, content, state, %{module: module} = process) do
    mode = List.wrap(module.callback_mode())

    cond do
      type == :enter and :state_enter not in mode -> :keep_state_and_data
      :state_functions in mode -> put_data(fn -> apply(module, state, [type, content, process.data]) end, process)
      true -> put_data(fn -> module.handle_event(type, content, state, process.data) end, process)
    end
  end

  # A process that couldn't take what it held to newer code holds nothing to end with, and a module may have no
  # `terminate/3`, which OTP doesn't ask of one.
  @impl true
  def terminate(why, state, process) do
    with %{data: data, module: module} <- process,
         true <- function_exported?(module, :terminate, 3),
         do: module.terminate(why, state, data)

    LoadedVersion.release([process.held])
  end

  # A module with no `code_change/4`, which OTP doesn't ask of one either, carries on with what it holds as it is.
  defp code_changed(module, vsn, state, data) do
    if function_exported?(module, :code_change, 4),
      do: Process.returned(fn -> module.code_change(vsn, state, data, nil) end),
      else: {:ok, state, data}
  end

  # What the app's module returned, with its data where this process keeps it.
  defp put_data(callback, process) do
    returned = Process.returned(callback)

    at =
      case returned do
        {:next_state, _state, _data} -> 2
        {:next_state, _state, _data, _actions} -> 2
        {keep_or_repeat, _data} when keep_or_repeat in [:keep_state, :repeat_state] -> 1
        {keep_or_repeat, _data, _actions} when keep_or_repeat in [:keep_state, :repeat_state] -> 1
        {:stop, _why, _data} -> 2
        {:stop_and_reply, _why, _replies, _data} -> 3
        _without_data -> nil
      end

    if at, do: put_elem(returned, at, %{process | data: elem(returned, at)}), else: returned
  end
end
