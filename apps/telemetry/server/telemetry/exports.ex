defmodule Telemetry.Exports do
  @moduledoc """
  The exports of the hours not yet rolled up that this server has read, each cut to what `Telemetry.Read` needs
  (`Telemetry.Rollup.cut/1`), by their names, kept between calls: an export never changes once it's kept, and every
  look at Telemetry reads all of an hour's again, a blob each every 30 s from every server, until they're rolled up
  (`Telemetry.Store.prune/1`). Those of hours before `since` are let go as more are kept, so it holds about an hour's.
  """
  use GenServer
  require OpenTelemetry.Tracer, as: Tracer
  alias OneCore.{Artifact, Call}
  alias Telemetry.{Rollup, Store}

  @doc """
  Each of the exports `names`, cut, reading and decoding only those not kept here, and keeping what it read. Those of
  hours whose folder is before `since` are let go.
  """
  def cuts(_call, [], _since), do: []

  def cuts(call, names, since) do
    exports = Call.process(call, __MODULE__, :exports, nil)
    kept = GenServer.call(exports, {:kept, names})
    missing = Enum.reject(names, &is_map_key(kept, &1))

    cut =
      Tracer.with_span "telemetry exports", %{attributes: %{"one.blobs" => length(missing)}} do
        for {name, {:ok, data, _tag}} <- Enum.zip(missing, Artifact.read_all(call.own, missing)),
            into: %{},
            do: {name, Rollup.cut(List.wrap(Store.export(name, data)))}
      end

    GenServer.cast(exports, {:keep, cut, since})
    for name <- names, cut = Map.get_lazy(kept, name, fn -> cut[name] end), do: cut
  end

  @impl true
  def init(nil), do: {:ok, %{}}

  @impl true
  def handle_call({:kept, names}, _from, kept), do: {:reply, Map.take(kept, names), kept}

  @impl true
  def handle_cast({:keep, read, since}, kept) do
    kept = Map.reject(kept, fn {name, _cut} -> Path.dirname(name) < since end)
    {:noreply, Map.merge(kept, read)}
  end
end
