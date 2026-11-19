defmodule Telemetry.Buffer do
  @moduledoc """
  What servers and clients export, held in memory on this server and kept every `@every` (`Telemetry.Store.keep/4`),
  as the OpenTelemetry Collector's batch processor holds what it's sent: so a hook that's told of an export answers
  at once, however slow the bucket is just then, and an hour's exports are a few blobs a minute rather than one for
  each.

  OTLP's export requests put one after the other are one request holding all of theirs, so what's held of a signal
  is kept as one blob. What couldn't be kept is held for the next time, up to `@most` bytes, past which the oldest
  is let go, saying how much. What's held when the server stops is kept as it stops.
  """
  use GenServer
  require Logger
  alias OneCore.Call
  alias Telemetry.{Otlp, Store}

  @every 30_000
  @most 16 * 1024 * 1024

  @doc "Holds `otlp`, an export request of `signal`, to be kept in `call`'s own artifact with the next."
  def add(%Call{} = call, signal, otlp) when is_binary(otlp) do
    if signal not in Otlp.signals(), do: raise("There's no signal #{signal}: it's one of #{Enum.join(Otlp.signals(), ", ")}")
    buffer = Call.process(call, __MODULE__, call.own.prefix, String.slice(call.server, 0, 8))
    GenServer.cast(buffer, {:add, call.own, signal, otlp})
  end

  # `held` is each signal's exports, newest first, and `own` the artifact they're kept in, as the last call had it.
  @impl true
  def init(source) do
    Process.flag(:trap_exit, true)
    Process.send_after(self(), :keep, @every)
    {:ok, %{source: source, own: nil, held: %{}, bytes: 0}}
  end

  @impl true
  def handle_cast({:add, own, signal, otlp}, buffer) do
    held = Map.update(buffer.held, signal, [otlp], &[otlp | &1])
    {:noreply, at_most(%{buffer | own: own, held: held, bytes: buffer.bytes + byte_size(otlp)})}
  end

  @impl true
  def handle_info(:keep, buffer) do
    Process.send_after(self(), :keep, @every)
    {:noreply, kept(buffer)}
  end

  def handle_info(_message, buffer), do: {:noreply, buffer}

  @impl true
  def terminate(_why, buffer), do: kept(buffer)

  # Keeps each signal's exports as one blob, and holds on to those it couldn't keep.
  defp kept(buffer) do
    held =
      Map.filter(buffer.held, fn {signal, exports} ->
        try do
          Store.keep(buffer.own, buffer.source, signal, IO.iodata_to_binary(Enum.reverse(exports)))
          false
        rescue
          error ->
            Logger.warning("Telemetry couldn't keep #{length(exports)} #{signal} exports, and tries again in #{div(@every, 1000)} s: #{Exception.message(error)}")
            true
        end
      end)

    %{buffer | held: held, bytes: Enum.sum_by(held, fn {_signal, exports} -> Enum.sum_by(exports, &byte_size/1) end)}
  end

  # Lets go of the oldest exports while more than `@most` bytes are held.
  defp at_most(%{bytes: bytes} = buffer) when bytes <= @most, do: buffer

  defp at_most(buffer) do
    {signal, exports} = Enum.max_by(buffer.held, fn {_signal, exports} -> Enum.sum_by(exports, &byte_size/1) end)
    {kept, [oldest]} = Enum.split(exports, -1)
    Logger.warning("Telemetry let go of a #{signal} export of #{byte_size(oldest)} bytes: more than #{div(@most, 1024 * 1024)} MB was waiting to be kept")
    held = if kept == [], do: Map.delete(buffer.held, signal), else: Map.put(buffer.held, signal, kept)
    at_most(%{buffer | held: held, bytes: buffer.bytes - byte_size(oldest)})
  end
end
