defmodule OneCore.Cache do
  @moduledoc """
  Caches, each by its name, in one ETS table, where an invalidate wins over a fetch in flight: what a fetch read
  before its key, or the whole cache, was invalidated is never given out afterwards, though it finishes after the
  invalidate.

  Cachex (its `fetch/4` courier) and ConCache (`get_or_store/3` against a cleared table) each store what such a fetch
  read, and Nebulex doesn't keep a fetch apart at all. So each value is kept here with the generations of its key and
  of the whole cache when its fetch began, and is given out only while both are still those.

  A key that isn't kept is fetched once however many ask for it at once, as Go's `singleflight` does: the first to ask
  fetches it, and the others wait for that and are given what it gave, or raise what it raised. One that asks after an
  invalidate fetches anew, as what's in flight may be from before it. If the process fetching ends without saying, as
  a hook stopped for taking too long does (`OneCore.Hooks.within/3`), the first waiting fetches it instead.

  The table is made once, as the server starts (`table/0`), and a cache is only a name in it.
  """

  @doc "Makes the table every cache is kept in. Whatever calls this owns it, and it goes when that ends."
  def table, do: :ets.new(__MODULE__, [:named_table, :public, read_concurrency: true])

  @doc """
  The value kept for `key` in `cache`, or else what `fetch` gives, as Cachex's `fetch/4` takes it: `{:commit, value}`
  keeps `value`, `{:commit, value, expire: ms}` keeps it for that long, and `{:ignore, value}` gives it without
  keeping it.
  """
  def fetch(cache, key, fetch) do
    generations = generations(cache, key)
    now = :erlang.monotonic_time(:millisecond)

    case :ets.lookup(__MODULE__, {:value, cache, key}) do
      [{_, value, ^generations, until}] when until == nil or until > now -> value
      _ -> flight(cache, key, generations, fetch)
    end
  end

  # Fetches `key`, or waits for the process already fetching it, which tells those waiting (`OneCore.Waiting`) how it
  # went. One that registers to wait after it was told finds it gone, and asks again.
  defp flight(cache, key, generations, fetch) do
    flight = {:flight, cache, key, generations}
    ref = make_ref()

    if :ets.insert_new(__MODULE__, {flight, self(), ref}) do
      went =
        try do
          {:ok, store(cache, key, generations, fetch.())}
        catch
          kind, reason -> {kind, reason, __STACKTRACE__}
        after
          :ets.delete(__MODULE__, flight)
        end

      Registry.dispatch(OneCore.Waiting, {__MODULE__, ref}, fn waiting -> for {pid, _} <- waiting, do: send(pid, {__MODULE__, ref, went}) end)
      fetched(went)
    else
      waited(cache, key, fetch, flight)
    end
  end

  defp waited(cache, key, fetch, flight) do
    with [{^flight, fetching, ref}] when fetching != self() <- :ets.lookup(__MODULE__, flight) do
      {:ok, _} = Registry.register(OneCore.Waiting, {__MODULE__, ref}, nil)
      monitor = Process.monitor(fetching)

      went =
        if :ets.lookup(__MODULE__, flight) == [{flight, fetching, ref}] do
          receive do
            {__MODULE__, ^ref, went} -> went
            {:DOWN, ^monitor, :process, _, _} -> :ets.delete_object(__MODULE__, {flight, fetching, ref}) && :again
          end
        else
          :again
        end

      Process.demonitor(monitor, [:flush])
      Registry.unregister(OneCore.Waiting, {__MODULE__, ref})
      if went == :again, do: fetch(cache, key, fetch), else: fetched(went)
    else
      [] -> fetch(cache, key, fetch)
      # A fetch that asks for its own key again fetches it again, rather than wait for itself.
      [_itself] -> store(cache, key, elem(flight, 3), fetch.())
    end
  end

  defp fetched({:ok, value}), do: value
  defp fetched({kind, reason, stacktrace}), do: :erlang.raise(kind, reason, stacktrace)

  defp store(cache, key, generations, {:commit, value, expire: ms}),
    do: store(cache, key, generations, value, :erlang.monotonic_time(:millisecond) + ms)

  defp store(cache, key, generations, {:commit, value}), do: store(cache, key, generations, value, nil)
  defp store(_cache, _key, _generations, {:ignore, value}), do: value

  # A fetch that began before an invalidate gives what it read to whoever asked, and keeps nothing over what was
  # fetched since.
  defp store(cache, key, generations, value, until) do
    if generations(cache, key) == generations, do: :ets.insert(__MODULE__, {{:value, cache, key}, value, generations, until})
    value
  end

  defp generations(cache, key),
    do:
      {:ets.lookup_element(__MODULE__, {:generation, cache}, 2, 0),
       :ets.lookup_element(__MODULE__, {:generation, cache, key}, 2, 0)}

  @doc "Invalidates everything `cache` keeps, so each key is fetched again when next asked for."
  def invalidate(cache) do
    :ets.update_counter(__MODULE__, {:generation, cache}, 1, {{:generation, cache}, 0})
    :ets.match_delete(__MODULE__, {{:value, cache, :_}, :_, :_, :_})
    :ok
  end

  @doc "Invalidates what `cache` keeps for `key`, so it's fetched again when next asked for."
  def invalidate(cache, key) do
    :ets.update_counter(__MODULE__, {:generation, cache, key}, 1, {{:generation, cache, key}, 0})
    :ets.delete(__MODULE__, {:value, cache, key})
    :ok
  end

  @doc "The keys `cache` keeps a value for."
  def keys(cache), do: for([key] <- :ets.match(__MODULE__, {{:value, cache, :"$1"}, :_, :_, :_}), do: key)
end
