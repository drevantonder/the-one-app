defmodule OneCore.Cache do
  @moduledoc """
  A cache in an ETS table where an invalidate wins over a fetch in flight: what a fetch read before its key, or the
  whole cache, was invalidated is never given out afterwards, though it finishes after the invalidate.

  Cachex (its `fetch/4` courier) and ConCache (`get_or_store/3` against a cleared table) each store what such a fetch
  read, and Nebulex doesn't keep a fetch apart at all. So each value is kept here with the generations of its key and
  of the whole cache when its fetch began, and is given out only while both are still those.
  """

  @doc "Makes the cache `cache`, an ETS table by that name. Whatever calls this owns it, and it goes when that ends."
  def new(cache), do: :ets.new(cache, [:named_table, :public, read_concurrency: true])

  @doc """
  The value kept for `key`, or else what `fetch` gives, as Cachex's `fetch/4` takes it: `{:commit, value}` keeps
  `value`, `{:commit, value, expire: ms}` keeps it for that long, and `{:ignore, value}` gives it without keeping
  it. Where the cache isn't made yet, nothing is kept.
  """
  def fetch(cache, key, fetch) do
    if :ets.whereis(cache) == :undefined do
      elem(fetch.(), 1)
    else
      generations = generations(cache, key)
      now = :erlang.monotonic_time(:millisecond)

      case :ets.lookup(cache, {:value, key}) do
        [{_, value, ^generations, until}] when until == nil or until > now -> value
        _ -> store(cache, key, generations, fetch.())
      end
    end
  end

  defp store(cache, key, generations, {:commit, value, expire: ms}),
    do: store(cache, key, generations, value, :erlang.monotonic_time(:millisecond) + ms)

  defp store(cache, key, generations, {:commit, value}), do: store(cache, key, generations, value, nil)
  defp store(_cache, _key, _generations, {:ignore, value}), do: value

  defp store(cache, key, generations, value, until) do
    :ets.insert(cache, {{:value, key}, value, generations, until})
    value
  end

  defp generations(cache, key),
    do: {:ets.lookup_element(cache, :generation, 2, 0), :ets.lookup_element(cache, {:generation, key}, 2, 0)}

  @doc "Invalidates everything `cache` keeps, so each key is fetched again when next asked for."
  def invalidate(cache) do
    if :ets.whereis(cache) != :undefined do
      :ets.update_counter(cache, :generation, 1, {:generation, 0})
      :ets.match_delete(cache, {{:value, :_}, :_, :_, :_})
    end

    :ok
  end

  @doc "Invalidates what `cache` keeps for `key`, so it's fetched again when next asked for."
  def invalidate(cache, key) do
    if :ets.whereis(cache) != :undefined do
      :ets.update_counter(cache, {:generation, key}, 1, {{:generation, key}, 0})
      :ets.delete(cache, {:value, key})
    end

    :ok
  end

  @doc "The keys `cache` keeps a value for."
  def keys(cache) do
    if :ets.whereis(cache) == :undefined,
      do: [],
      else: for([key] <- :ets.match(cache, {{:value, :"$1"}, :_, :_, :_}), do: key)
  end
end
