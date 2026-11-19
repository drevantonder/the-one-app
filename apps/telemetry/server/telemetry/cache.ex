defmodule Telemetry.Cache do
  @moduledoc """
  What this server has read of what's kept, by each blob's name, as `Telemetry.Read` reads it, kept between calls: a
  blob never changes once it's kept, and every look reads the same ones again, each hour's rollup and the exports of
  the hours not rolled up yet. What no look has asked for in `@unused` is let go, and a new build of Telemetry starts
  with nothing (`code_change/3`), since what an older one kept may be in another shape.
  """
  use GenServer
  alias OpenTelemetry.Tracer
  alias OneCore.{Artifact, Call}

  @unused 600_000

  @doc """
  Each of the blobs `names` that's there, as `read` gives it from its name and what's in it, reading from the bucket
  only those not kept here, and keeping them.
  """
  def read(_call, [], _read), do: []

  def read(call, names, read) do
    cache = Call.process(call, __MODULE__, :read, nil)
    kept = GenServer.call(cache, {:kept, names})
    missing = Enum.reject(names, &is_map_key(kept, &1))
    Tracer.set_attribute("one.read", length(missing))
    got = for {name, {:ok, data, _tag}} <- Enum.zip(missing, Artifact.read_all(call.own, missing)), into: %{}, do: {name, read.(name, data)}
    GenServer.cast(cache, {:keep, got})
    for name <- names, value = Map.get_lazy(kept, name, fn -> got[name] end), do: value
  end

  # Each blob read, by its name, and when a look last asked for it.
  @impl true
  def init(nil), do: {:ok, %{}}

  @impl true
  def handle_call({:kept, names}, _from, kept) do
    now = System.monotonic_time(:millisecond)
    asked = for {name, {value, _at}} <- Map.take(kept, names), into: %{}, do: {name, {value, now}}
    {:reply, Map.new(asked, fn {name, {value, _at}} -> {name, value} end), Map.merge(kept, asked)}
  end

  @impl true
  def handle_cast({:keep, got}, kept) do
    now = System.monotonic_time(:millisecond)
    kept = Map.reject(kept, fn {_name, {_value, at}} -> now - at > @unused end)
    {:noreply, Map.merge(kept, Map.new(got, fn {name, value} -> {name, {value, now}} end))}
  end

  @impl true
  def code_change(_version, _kept, _extra), do: {:ok, %{}}
end
