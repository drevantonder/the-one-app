defmodule Telemetry.Exports do
  @moduledoc """
  The exports this server has read of the hours not yet put together, by their names, kept between calls: an export
  never changes once it's kept, and every look at Telemetry reads all of an hour's again, one blob each, until
  they're put together (`Telemetry.Store.prune/1`). Those of hours before `since` are let go as more are kept, so it
  holds about an hour's.
  """
  use GenServer
  alias OneCore.{Artifact, Call}

  @doc """
  Each of the exports `names`, as `OneCore.Artifact.read_all/2` gives it, reading from the bucket only what isn't
  kept here, and keeping what it read. Those of hours whose folder is before `since` are let go.
  """
  def read(_call, [], _since), do: []

  def read(call, names, since) do
    exports = Call.process(call, __MODULE__, :exports, nil)
    kept = GenServer.call(exports, {:kept, names})
    missing = Enum.reject(names, &is_map_key(kept, &1))
    read = Map.new(Enum.zip(missing, Artifact.read_all(call.own, missing)))
    GenServer.cast(exports, {:keep, for({name, {:ok, _data, _tag} = got} <- read, do: {name, got}), since})
    Enum.map(names, &Map.get_lazy(kept, &1, fn -> read[&1] end))
  end

  @impl true
  def init(nil), do: {:ok, %{}}

  @impl true
  def handle_call({:kept, names}, _from, kept), do: {:reply, Map.take(kept, names), kept}

  @impl true
  def handle_cast({:keep, read, since}, kept) do
    kept = Map.reject(kept, fn {name, _got} -> Path.dirname(name) < since end)
    {:noreply, Map.merge(kept, Map.new(read))}
  end
end
