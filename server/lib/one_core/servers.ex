defmodule OneCore.Servers do
  @moduledoc """
  Which servers act for whom. One Core keeps, for each user and group, the list of servers that act for them,
  under their prefix: the blob `servers`, of the servers' keys. The network's list is the same blob under
  `network/`, and a server on it acts for everyone in the network. A list's first server is written by a server
  acting for the network, and only if there's no list. A server on a list adds another.

  A server's own blobs sit under `servers/<its key>/`, and only it writes them. Its `address` is how to reach it,
  which it writes each time it starts, since it may start somewhere else. Its `members` are who it acts for: its
  owner first, whom it was started for, and then whoever its owner chose. A new one is for its owner alone.

  So a server acts for someone once both said so: they put it on their list, which is what gives it their blobs,
  and its owner chose them as a member, which is what has it run their plugins on its machine.

  What's read of this is kept until this server next looks (`forget/0`), so a call asks the bucket nothing.
  """
  alias OneCore.{Bucket, Server}

  @network "network"

  @doc "The keys of the servers on `owner`'s list, the first first."
  def listed(owner), do: kept({:listed, owner}, fn -> read("#{owner}/servers", []) end)

  @doc "The keys of the servers to ask for `owner`: the ones on their list, then the ones acting for the network."
  def acting(owner), do: Enum.uniq(listed(owner) ++ listed(@network))

  @doc "Whether the server whose key is `key` is one `owner` let act for them: it's on their list, or the network's."
  def acts_for?(key, owner), do: key in listed(owner) or key in listed(@network)

  @doc "Who the server whose key is `key` acts for: its owner first, and whoever its owner chose."
  def members(key), do: kept({:members, key}, fn -> read("servers/#{key}/members", []) end)

  @doc "Who the server whose key is `key` acts for, both having said so: each member of it that has it on their list."
  def acts_for(key), do: Enum.filter(members(key), &(key in listed(&1)))

  @doc """
  Whether this server acts for `owner`: it's a member of this server that has it on their list, or the network
  is, which is everyone.
  """
  def here?(owner), do: Enum.any?(acts_for(Server.key()), &(&1 in [owner, @network]))

  @doc "The services the server whose key is `key` has installed: each one's plugin and its name."
  def installed(key) do
    kept({:installed, key}, fn -> for [plugin, name] <- read("servers/#{key}/services", []), do: {plugin, name} end)
  end

  @doc "This server's services installed are `services` from now on."
  def install(services) do
    say("servers/#{Server.key()}/services", JSON.encode!(for {plugin, name} <- services, do: [plugin, name]))
    forget()
  end

  @doc "How to reach a server, as the JSON a client is told it in."
  def address(key) do
    with {:ok, address, _tag} <- Bucket.read("servers/#{key}/address"), do: {:ok, address}
  end

  @doc """
  Whether the server whose key is `key`, started for `owner`, is one of theirs: it's on their list, which a
  server acting for them added it to (`add/3`), or it's for the network, which had no server, and is now its
  first.
  """
  def joined?(key, owner) do
    # A network with no list has no server yet, and whichever writes its list is its first. A bucket that refuses
    # says this isn't the one to.
    if owner == @network, do: Bucket.write("#{@network}/servers", JSON.encode!([key]))
    key in read("#{owner}/servers", [])
  rescue
    Bucket.Refused -> false
  end

  @doc "A server that started says how to reach it now, and who it's for if it has never said."
  def started(key, address, owner) do
    say("servers/#{key}/address", address)
    Bucket.write("servers/#{key}/members", JSON.encode!([owner]))
    forget()
  end

  @doc """
  Adds a server to `owner`'s list, which only this server, acting for them, may. The first on a list is written
  by a server acting for the network.
  """
  def add(key, owner) do
    blob = "#{owner}/servers"

    wrote =
      case Bucket.read(blob) do
        {:ok, keys, tag} ->
          keys = JSON.decode!(keys)
          if key in keys, do: :ok, else: Bucket.replace(blob, JSON.encode!(keys ++ [key]), tag)

        :absent ->
          if Server.key() in listed(@network), do: Bucket.write(blob, JSON.encode!([key])), else: :refused
      end

    forget()

    case wrote do
      :ok -> :ok
      :refused -> {:error, "The first server to act for #{owner} is added by one acting for the network"}
      _changed -> add(key, owner)
    end
  end

  @doc """
  Takes a server off `owner`'s list, which only this server, acting for them, may: it no longer acts for them,
  and the signer hands it nothing of theirs. The network keeps a server, since its list is only ever begun once.
  """
  def remove(key, owner) do
    blob = "#{owner}/servers"

    with {:ok, keys, tag} <- Bucket.read(blob),
         keys = JSON.decode!(keys),
         true <- key in keys do
      cond do
        owner == @network and keys == [key] -> {:error, "The network keeps a server. Add another before this one goes."}
        Bucket.replace(blob, JSON.encode!(keys -- [key]), tag) == :ok -> forget()
        true -> remove(key, owner)
      end
    else
      _ -> {:error, "That server isn't on #{owner}'s list"}
    end
  end

  @doc "This server acts for `member` too from now on, once they have it on their list: its owner chose them."
  def choose(member) do
    blob = "servers/#{Server.key()}/members"
    {:ok, members, tag} = Bucket.read(blob)
    members = JSON.decode!(members)
    if member not in members, do: :ok = Bucket.replace(blob, JSON.encode!(members ++ [member]), tag)
    forget()
  end

  @doc "Writes one of a server's own blobs. Only that server writes it, so what's there is its own from before."
  def say(blob, data) do
    case Bucket.read(blob) do
      {:ok, _was, tag} -> :ok = Bucket.replace(blob, data, tag)
      :absent -> :ok = Bucket.write(blob, data)
    end
  end

  @doc "Forgets what was read, so that it's read again when next asked for."
  def forget do
    if :ets.whereis(__MODULE__) != :undefined, do: :ets.delete_all_objects(__MODULE__)
    :ok
  end

  @doc "Makes where what's read is kept. Whatever calls this owns it, and it goes when that ends."
  def keep, do: :ets.new(__MODULE__, [:named_table, :public, read_concurrency: true])

  # What `read` gave for `what` when it was last asked, which is asked again where nothing keeps it.
  defp kept(what, read) do
    case :ets.whereis(__MODULE__) != :undefined and :ets.lookup(__MODULE__, what) do
      [{^what, kept}] ->
        kept

      [] ->
        kept = read.()
        :ets.insert(__MODULE__, {what, kept})
        kept

      false ->
        read.()
    end
  end

  defp read(blob, absent) do
    case Bucket.read(blob) do
      {:ok, keys, _tag} -> JSON.decode!(keys)
      :absent -> absent
    end
  end
end
