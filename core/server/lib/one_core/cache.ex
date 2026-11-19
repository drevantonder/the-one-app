defmodule OneCore.Cache do
  @moduledoc """
  Caches, each by its name, in one ETS table, where an invalidate wins over a fetch in flight: what a fetch read
  before its key, or the whole cache, was invalidated is never given out afterwards, though it finishes after the
  invalidate.

  Cachex (its `fetch/4` courier) and ConCache (`get_or_store/3` against a cleared table) each store what such a fetch
  read, and Nebulex doesn't keep a fetch apart at all. So each value is kept here with the generations of its key and
  of the whole cache when its fetch began, and is given out only while both are still those.

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
      _ -> store(cache, key, generations, fetch.())
    end
  end

  defp store(cache, key, generations, {:commit, value, expire: ms}),
    do: store(cache, key, generations, value, :erlang.monotonic_time(:millisecond) + ms)

  defp store(cache, key, generations, {:commit, value}), do: store(cache, key, generations, value, nil)
  defp store(_cache, _key, _generations, {:ignore, value}), do: value

  defp store(cache, key, generations, value, until) do
    :ets.insert(__MODULE__, {{:value, cache, key}, value, generations, until})
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
