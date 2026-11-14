defmodule OneCore.Servers do
  @moduledoc """
  The servers that act for a user or a group: the list One Core keeps under their prefix, as the blob `servers`,
  of the servers' keys. The first is written only if there's no list, and a server on it adds another. The servers
  that act for the network, and so for everyone in it, are the same kind of list under `network/`.

  A server's own blobs sit under `servers/<its key>/`. Its `address` is how to reach it, which it writes each time
  it starts, since it may start somewhere else.

  With no plugin that signs users in there is one user, and so one list.
  """
  alias OneCore.Bucket

  @list "users/anyone/servers"

  @doc "The keys of the servers acting, the first first."
  def acting do
    case Bucket.read(@list) do
      {:ok, keys, _tag} -> JSON.decode!(keys)
      :absent -> []
    end
  end

  @doc "How to reach each server acting that says how, as the JSON a client is told it in."
  def addresses, do: for(key <- acting(), {:ok, address} <- [address(key)], do: address)

  @doc "How to reach a server, as the JSON a client is told it in."
  def address(key) do
    with {:ok, address, _tag} <- Bucket.read("servers/#{key}/address"), do: {:ok, address}
  end

  @doc """
  A network's first server acts for the network and for its one user: it's the first on each list, if there's none.
  A server is one of its network's own by a list it's on, so this comes before it reads the bucket or is reached.
  """
  def first(key) do
    for list <- ["network/servers", @list], do: Bucket.write(list, JSON.encode!([key]))
    :ok
  end

  @doc "A server that started says how to reach it now, and is the first on the list if there's no list."
  def started(key, address) do
    say("servers/#{key}/address", address)
    Bucket.write(@list, JSON.encode!([key]))
    :ok
  end

  @doc "Adds a server to the list, which only `by`, a server on it, may."
  def add(key, by) do
    with {:ok, keys, tag} <- Bucket.read(@list),
         keys = JSON.decode!(keys),
         true <- by in keys do
      if key in keys or Bucket.replace(@list, JSON.encode!(keys ++ [key]), tag) == :ok, do: :ok, else: add(key, by)
    else
      _ -> {:error, "Only a server that already acts here can add another"}
    end
  end

  @doc "Writes one of a server's own blobs. Only that server writes it, so what's there is its own from before."
  def say(blob, data) do
    case Bucket.read(blob) do
      {:ok, _was, tag} -> :ok = Bucket.replace(blob, data, tag)
      :absent -> :ok = Bucket.write(blob, data)
    end
  end
end
