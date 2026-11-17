defmodule OneCore.Servers do
  @moduledoc """
  Which servers act for whom. One Core keeps, for each user and group, the list of servers that act for them,
  under their prefix: the blob `servers`, of the servers' keys. The network's list is the same blob under
  `network/`, and a server on it acts for everyone in the network. A list's first server is written by a server
  acting for the network, and only if there's no list. A server on a list adds another.

  A server's own blobs sit under `servers/<its key>/`, and only it writes them. Its `address` is how to reach it,
  and its `name` what its machine is called, which it writes each time it starts, since it may start somewhere
  else. Its `members` are who it acts for: its owner first, whom it was started for, and then whoever its owner
  chose. A new one is for its owner alone.

  So a server acts for someone once both said so: they put it on their list, which is what gives it their blobs,
  and its owner chose them as a member, which is what has it run their plugins on its machine.

  A plugin's code runs with all of its machine's access, so a plugin runs only on servers of whoever it's installed
  for (`runs?/1`): the network's on the network's servers, and a user's or a group's on servers acting for them
  alone, never on the network's merely because those act for everyone. The network's owners own the network's
  servers, so what they install for themselves runs there too.

  What's read of this is kept, so a call asks the bucket nothing, and read again when this server looks at someone
  (`look/1`) or forgets it all (`forget/0`).
  """
  require Logger
  alias OneCore.{Bucket, Builds, Edge, Hooks, Server}

  @network "network"

  @doc "The keys of the servers on `owner`'s list, the first first."
  def listed(owner), do: cached({:listed, owner}, fn -> read("#{owner}/servers", []) end)

  @doc "The keys of the servers to ask for `owner`: the ones on their list, then the ones acting for the network."
  def acting(owner), do: Enum.uniq(listed(owner) ++ listed(@network))

  @doc "Whether the server whose key is `key` is one `owner` let act for them: it's on their list, or the network's."
  def acts_for?(key, owner), do: key in listed(owner) or key in listed(@network)

  @doc "Who the server whose key is `key` acts for: its owner first, and whoever its owner chose."
  def members(key), do: cached({:members, key}, fn -> read("servers/#{key}/members", []) end)

  @doc "Who the server whose key is `key` acts for, both having said so: each member of it that has it on their list."
  def acts_for(key), do: Enum.filter(members(key), &(key in listed(&1)))

  @doc """
  Whether this server acts for `owner`: it's a member of this server that has it on their list, or the network
  is, which is everyone. It's asked while the server starts its plugins too, before it's reached, whose key is
  then only in its file.
  """
  def here?(owner) do
    key = Server.key() || elem(OneCore.Iroh.key(Server.key_file()), 1)
    Enum.any?(acts_for(key), &(&1 in [owner, @network]))
  end

  @doc """
  Whether this server runs the plugins installed for `owner`: it acts for them as one of their own servers, not
  only as one of the network's, or it's the network's and they are, or own, the network. Who owns the network is
  what the plugins that validate `reach` say.
  """
  def runs?(owner) do
    cond do
      owner == @network -> here?(@network)
      owner in acts_for(Server.key()) -> true
      true -> here?(@network) and owns_network?(owner)
    end
  end

  @doc "The keys of the servers that run the plugins installed for `owner`, as `runs?/1` says of each, the first first."
  def running(@network), do: listed(@network)

  def running(owner) do
    own = Enum.filter(listed(owner), &(owner in members(&1)))
    Enum.uniq(own ++ if(owns_network?(owner), do: listed(@network), else: []))
  end

  # Asked of the network's plugins, as everything about the network is.
  defp owns_network?(owner) do
    about = %{"who" => owner, "relation" => "owner", "artifact" => @network}
    Hooks.validate("reach", about, {owner, @network}) == :ok
  end

  @doc """
  What whoever asked for `what`, a plugin installed for `owner`, is told when none of `servers`, the ones that run
  it, answered.
  """
  def off(what, owner, []),
    do:
      "#{what} runs on #{owner}'s own servers, and there are none. Set one up in Devices, or ask whoever may " <>
        "install plugins for everyone to install it for everyone."

  def off(what, owner, servers),
    do:
      "#{what} runs on #{owner}'s own servers, and none is on: " <>
        Enum.map_join(servers, "; ", &Server.error_message(:unreachable, &1)) <> "."

  @doc "The services the server whose key is `key` has installed: each one's plugin and its name."
  def installed(key) do
    cached({:installed, key}, fn -> for [plugin, name] <- read("servers/#{key}/services", []), do: {plugin, name} end)
  end

  @doc "This server's services installed are `services` from now on."
  def install(services) do
    say("servers/#{Server.key()}/services", JSON.encode!(for {plugin, name} <- services, do: [plugin, name]))
    forget()
  end

  @doc "What the machine the server whose key is `key` runs on is called, as it said when it last started."
  def name(key), do: cached({:name, key}, fn -> read("servers/#{key}/name", nil) end)

  @doc """
  Whether the server whose key is `key` is on: `:on` while it's connected to the network's edge and up, `:restarting`
  while it's stopping to start again, or starting, and isn't up yet (`restarting/1`), the time its machine was last on there,
  in milliseconds since 1970, if it isn't, or `nil` where that isn't known: a network with no address, or a server
  never seen. The relay at the address keeps it, and tells servers when one comes or goes (`seen_again/0`), so a
  server that's known to be off isn't waited for, and one that's restarting is waited for only where it's the one
  to ask (`back/1`).
  """
  def seen(key) do
    if at = Application.get_env(:one_core, :address) do
      cached({:seen, key}, fn ->
        path = "/relay/seen"
        response = Edge.send!(method: :get, url: at <> path, params: [keys: key], headers: Edge.signed(path, ""))
        if response.status == 200, do: on(response.body[key]), else: nil
      end)
    end
  end

  defp on("on"), do: :on
  defp on("restarting"), do: :restarting
  defp on(at), do: at

  @doc """
  What was kept of who's on is read again, once one of the network's servers came or went, and whoever waits for one
  to be back (`back/1`) looks again.
  """
  def seen_again do
    if :ets.whereis(__MODULE__) != :undefined,
      do: Enum.each(:ets.match(__MODULE__, {{:seen, :"$1"}, :_, :_}), fn [key] -> again({:seen, key}) end)

    Registry.dispatch(OneCore.Waiting, :seen, fn waiting -> for {process, _} <- waiting, do: send(process, :seen) end)
    :ok
  end

  @doc """
  Says at the network's edge that this server is stopping to start again, or starting, and is up within `seconds`:
  until it says it's up (`up/0`), or that's passed, the relay there says it's restarting, not off or on, so the
  other servers ask it nothing and wait for it only where it's the one to ask. Where the edge can't be reached it's
  only taken for off or on meanwhile, so that's no reason not to start.
  """
  def restarting(seconds), do: say_seen(%{restarting: seconds, until: "up"}, "restarting")

  @doc "Says at the network's edge that this server is up (`OneCore.Server.up?/0`), and its plugins started: it's on."
  def up, do: say_seen(%{up: true}, "up")

  defp say_seen(said, what) do
    if at = Application.get_env(:one_core, :address) do
      path = "/relay/seen"
      body = JSON.encode!(said)
      response = Edge.send!(method: :post, url: at <> path, body: body, headers: Edge.signed(path, body))
      if response.status != 204,
        do: Logger.warning("The edge didn't take that this server is #{what}: #{inspect(response.body)}")
    end

    :ok
  rescue
    error in Edge.Unreachable -> Logger.warning("Couldn't say this server is #{what}: #{Exception.message(error)}")
  end

  # How long a server that's stopping to start again, to update, takes to be back, in milliseconds: it finishes what
  # it took, then starts again, and connects to the edge as it does. Well within what an MCP client waits for an
  # answer, 30 seconds. One that isn't back by then may be trying a build that doesn't start, which takes minutes.
  @back 20_000

  @doc """
  Waits for one of the servers whose keys are `keys` that's restarting to be back, for up to `@back`: where none of
  the others answered, and what was asked is that one's to answer, as a service only it runs. Gives its key, or
  `nil` if none is back by then, or none of them was restarting.
  """
  def back(keys) do
    restarting = Enum.filter(keys, &(seen(&1) == :restarting))

    if restarting != [] do
      Registry.register(OneCore.Waiting, :seen, nil)
      names = Enum.map_join(restarting, " or ", &(name(&1) || "a server"))
      OneCore.Call.State.working(OneCore.Call.State.id(), :waiting, "Waiting for #{names} to start again")

      try do
        await(restarting, System.monotonic_time(:millisecond) + @back)
      after
        Registry.unregister(OneCore.Waiting, :seen)
      end
    end
  end

  # Looks after each time a server came or went, until one of `keys` is on, or `until`, and then once more at the
  # edge: one that said it may take less than it did is back without having come or gone.
  defp await(keys, until) do
    case Enum.find(keys, &(seen(&1) == :on)) do
      nil ->
        receive do
          :seen -> await(keys, until)
        after
          max(until - System.monotonic_time(:millisecond), 0) -> Enum.find(keys, &(seen_again(&1) == :on))
        end

      key ->
        key
    end
  end

  @doc """
  The protocol the server whose key is `key` speaks with other servers (`OneCore.Server.protocol/0`), as it said
  when it last started: the first, for one from before servers said it.
  """
  def protocol(key), do: cached({:protocol, key}, fn -> read("servers/#{key}/build", %{})["protocol"] || 1 end)

  @doc "Which protocol the server whose key is `key` speaks is read again: it refused what it was asked in this server's."
  def protocol_again(key), do: again({:protocol, key})

  @doc """
  Whether the server whose key is `key` is on, as `seen/1` says, read again now. What was kept may be from before
  it came back: what the relay told this server of it while this one wasn't listening, or what it read of it while
  being told, is missed.
  """
  def seen_again(key) do
    again({:seen, key})
    seen(key)
  end

  @doc "How long ago `at`, in milliseconds since 1970, was, as a person would say it short: `2 min ago`."
  def ago(at) do
    case div(System.system_time(:millisecond) - at, 60_000) do
      minutes when minutes < 1 -> "less than a minute ago"
      minutes when minutes < 120 -> "#{minutes} min ago"
      minutes when minutes < 48 * 60 -> "#{div(minutes, 60)} h ago"
      minutes -> "#{div(minutes, 24 * 60)} days ago"
    end
  end

  @doc "How to reach a server, as the JSON a client is told it in."
  def address(key) do
    with {:ok, address, _tag} <- Bucket.read("servers/#{key}/address"), do: {:ok, address}
  end

  @doc """
  Whether the server whose key is `key`, started for `owner`, is one of theirs: it's on their list, which a
  server acting for them installed it to (`add/3`), or it's for the network, which had no server, and is now its
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

  @doc """
  A server that started says how to reach it now, what its machine is called and which build it is, and who it's for
  if it has never said.
  """
  def started(key, address, name, owner) do
    say("servers/#{key}/address", address)
    say("servers/#{key}/name", JSON.encode!(name))
    Builds.say(key)
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
      :refused -> {:error, "The first server to act for #{owner} is installed by one acting for the network"}
      _changed -> add(key, owner)
    end
  end

  @doc """
  Takes a server off `owner`'s list, which only this server, acting for them, may: it no longer acts for them,
  and the presigner hands it nothing of theirs. The network keeps a server, since its list is only ever begun once.
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

  @doc """
  Reads again what's kept of the servers acting for `owner`: their list, and what each server on it acts for and
  has installed. Until it's read, what was kept is what's asked. The network's first owner is the network's.
  """
  def reload(owner) do
    again({:listed, owner})
    servers = for key <- listed(owner), what <- [{:members, key}, {:installed, key}, {:name, key}], do: what
    Enum.each(if(owner == @network, do: [:first_owner | servers], else: servers), &again/1)
  end

  defp again(what) do
    with true <- :ets.whereis(__MODULE__) != :undefined,
         [{^what, _value, read}] <- :ets.lookup(__MODULE__, what),
         do: :ets.insert(__MODULE__, {what, read.(), read})
  end

  @doc "Forgets what was read, so that it's read again when next asked for."
  def forget do
    if :ets.whereis(__MODULE__) != :undefined, do: :ets.delete_all_objects(__MODULE__)
    :ok
  end

  @doc "Makes where what's read is kept. Whatever calls this owns it, and it goes when that ends."
  def keep, do: :ets.new(__MODULE__, [:named_table, :public, read_concurrency: true])

  @doc "What `read` gave for `what` when it was last asked, which is asked again where nothing keeps it."
  def cached(what, read) do
    case :ets.whereis(__MODULE__) != :undefined and :ets.lookup(__MODULE__, what) do
      [{^what, value, _read}] ->
        value

      [] ->
        value = read.()
        :ets.insert(__MODULE__, {what, value, read})
        value

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
