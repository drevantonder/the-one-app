defmodule OneCore.Servers do
  @moduledoc """
  The servers that act for a user or a group: the list One Core keeps under their prefix, as the blob `servers`,
  of the servers' keys. The first is written only if there's no list, and a server on it adds another. The servers
  that act for the network, and so for everyone in it, are the same kind of list under `network/`.

  A server's own blobs sit under `servers/<its key>/`. Its `address` is how to reach it, which it writes each time
  it starts, since it may start somewhere else.
  """
  alias OneCore.{Bucket, User}

  @network "network/servers"
  @list User.anyone() <> "/servers"

  @doc "The keys of the servers acting, the first first."
  def acting, do: listed(@list)

  @doc "How to reach each server acting that says how, as the JSON a client is told it in."
  def addresses, do: for(key <- acting(), {:ok, address} <- [address(key)], do: address)

  @doc "How to reach a server, as the JSON a client is told it in."
  def address(key) do
    with {:ok, address, _tag} <- Bucket.read("servers/#{key}/address"), do: {:ok, address}
  end

  @doc """
  Whether the server whose key is `key` is one of this network's: it's on the list, which a server already on it
  added it to (`add/2`), or the network had no server and this is now its first.
  """
  def joined?(key) do
    first(key)
    key in acting()
  rescue
    Bucket.Refused -> false
  end

  # A network with no list has no server yet. Whichever writes its list is its first: it acts for the network, and
  # so writes the first list of the servers acting. A bucket that refuses says this isn't the one to.
  defp first(key) do
    Bucket.write(@network, JSON.encode!([key]))
    if key in listed(@network), do: Bucket.write(@list, JSON.encode!([key]))
  rescue
    Bucket.Refused -> :ok
  end

  @doc "A server that started says how to reach it now."
  def started(key, address), do: say("servers/#{key}/address", address)

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

  defp listed(list) do
    case Bucket.read(list) do
      {:ok, keys, _tag} -> JSON.decode!(keys)
      :absent -> []
    end
  end
end
