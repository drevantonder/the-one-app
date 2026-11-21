defmodule OneCore.Servers do
  @moduledoc """
  Which servers act for whom. One Core keeps, for each user and group, the list of servers that act for them,
  under their prefix: the blob `servers`, of the servers' keys. The network's list is the same blob under
  `network/`, and a server on it acts for everyone in the network. A list's first server is written by a server
  acting for the network, and only if there's no list. A server on a list adds another.

  A server's own blobs sit under `servers/<its key>/`, and only it writes them. Its `address` is how to reach it,
  its `name` what its machine is called, which it writes each time it starts, since it may start somewhere else. Its
  `members` are who it acts for: its owner first, whom it was started for, and then whoever its owner chose. A new
  one is for its owner alone.

  So a server acts for someone once both said so: they put it on their list, which is what gives it their blobs,
  and its owner chose them as a member, which is what has it run their apps on its machine.

  An app's code runs with all of its machine's access, so which servers allow it is `OneCore.Apps.allows?/3`'s.

  What's read of this is cached (`OneCore.Cache`), so a call asks the bucket nothing, and read again once this server
  looks at someone (`reload/1`) or invalidates it all (`invalidate/0`).
  """
  require Logger
  alias OneCore.{Bucket, Builds, Cache, Edge, Protocol, Server}

  @network "network"

  @doc "The keys of the servers on `owner`'s list, the first first."
  def listed(owner), do: cached({:listed, owner}, fn -> read("#{owner}/servers", []) end)

  @doc """
  The keys of the servers to ask for `whose`, a user, a group or the network, narrowest first: those acting for them,
  then those acting for the network, which holds everyone, each both ways (`acts_for/1`).
  """
  def covering(whose), do: Enum.uniq(own(whose) ++ own(@network))

  @doc "The keys of the servers that are up, of every server there is by its own blobs (`ready?/1`)."
  def up, do: Enum.filter(all(), &(ready?(&1) == true))

  @doc "The keys of every server there is, by its own blobs."
  def all,
    do: cached(:all, fn -> for blob <- Bucket.list("servers/"), [key, "members"] <- [Path.split(Path.relative_to(blob, "servers"))], do: key end)

  @doc "The keys of the servers acting for `whose` as their own, both ways (`acts_for/1`)."
  def own(whose), do: for(key <- listed(whose), whose in members(key), do: key)

  @doc """
  Whether the server whose key is `key` acts for `who`, a user, a group or the network: for them, or for the network,
  which holds everyone, both ways: it's on their list, and it chose them (`acts_for/1`).
  """
  def acts_for?(key, who), do: Enum.any?(acts_for(key), &(&1 in [who, @network]))

  @doc "Who the server whose key is `key` acts for: its owner first, and whoever its owner chose."
  def members(key), do: cached({:members, key}, fn -> read("servers/#{key}/members", []) end)

  @doc "Who the server whose key is `key` acts for, both having said so: each member of it that has it on their list."
  def acts_for(key), do: Enum.filter(members(key), &(key in listed(&1)))

  @doc """
  Who this server is for: its owner, as it acts for them, or the network for one of the network's. What happens on it
  is about them, so it tells their apps, and the network's, as nobody (`OneCore.Mcp.Principal.asking/3`).
  """
  def owner, do: List.first(here()) || @network

  @doc """
  Who this server acts for (`acts_for/1`). It's asked while the server starts its apps too, before it's reached,
  whose key is then only in its file.
  """
  def here, do: acts_for(key())

  @doc "Whether this server acts for `who` (`acts_for?/2`)."
  def here?(who), do: acts_for?(key(), who)

  # This server's key, which while it starts is only in its file.
  defp key, do: Server.key() || elem(OneCore.Iroh.key(Server.key_file()), 1)

  @doc """
  What whoever asked for `what`, an app installed for `owner`, is told when none of `servers`, the ones that run
  it, answered.
  """
  def off(what, owner, []),
    do:
      "#{what} runs on #{owner}'s own servers, and there are none. Set one up in Devices, or ask whoever may " <>
        "install apps for everyone to install it for everyone."

  def off(what, owner, servers),
    do:
      "#{what} runs on #{owner}'s own servers, and none is on: " <>
        Enum.map_join(servers, "; ", &Server.error_message(:unreachable, &1)) <> "."

  @doc "The services the server whose key is `key` has installed: each one's app and its name."
  def installed(key) do
    cached({:installed, key}, fn -> for [app, name] <- read("servers/#{key}/services", []), do: {app, name} end)
  end

  @doc "This server's services installed are `services` from now on."
  def install(services) do
    say("servers/#{Server.key()}/services", JSON.encode!(for {app, name} <- services, do: [app, name]))
    invalidate()
  end

  @doc "What the machine the server whose key is `key` runs on is called, as it said when it last started."
  def name(key), do: cached({:name, key}, fn -> read("servers/#{key}/name", nil) end)

  @doc """
  Whether the server whose key is `key` is ready, as the relay at the network's address says of every server and
  device: a map of whether it's `connected` there now, and if not when it was `last_seen`, in milliseconds since
  1970, and of a server its `server_membership` and `server_daemon_status`, as it says them (`report/0`), and from
  those `server_ready` and, where it isn't, `server_ready_reason`: `joining`, `starting`, `draining` or `stopped`.
  `nil` where that isn't known: a network with no address, a relay that says none, or a key never seen. The relay
  tells servers when one comes or goes (`presence_again/0`), so one that isn't ready is asked nothing, and one that's
  starting or stopping is waited for only where it's the one to ask (`back/1`).
  """
  def presence(key) do
    if at = Application.get_env(:one_core, :address) do
      cached({:presence, key}, fn ->
        path = "/relay/seen"
        response = Edge.send!(method: :get, url: at <> path, params: [keys: key], headers: Edge.signed(path, ""))
        if response.status == 200, do: get_in(response.body, ["presence", key])
      end)
    end
  end

  @doc """
  Whether the server whose key is `key` answers the relay at the network's address now: the relay pings it there, as
  it does before it gives one an outsider's call, and it has `within` milliseconds to answer, at most the 5 seconds
  it gives one an outsider's call. A connection that has gone quiet stays connected there for a while (`presence/1`),
  as a machine that has just gone to sleep does, and this doesn't wait for that. `nil` where the network has no
  address, or where the relay doesn't say (not a 200).
  """
  def answers?(key, within) do
    if at = Application.get_env(:one_core, :address) do
      path = "/relay/seen"
      params = [keys: key, ping: 1, within: within]
      response = Edge.send!(method: :get, url: at <> path, params: params, headers: Edge.signed(path, ""))
      if response.status == 200, do: get_in(response.body, ["presence", key, "answered"]) == true
    end
  end

  @doc "Whether the server whose key is `key` is ready (`presence/1`): `nil` where that isn't known."
  def ready?(key), do: presence(key)["server_ready"]

  @doc """
  Whether this server is the one to do `what` (a JSON value, such as the build or the commit it's for), of all the
  servers that would, as the blob `blob` says: the first to ask writes that it does, and the others leave it to that
  one while it's ready. One that isn't any more left it unfinished, and the next to ask takes it on, as it does
  whatever was claimed there for something else.
  """
  def claims?(blob, what) do
    me = Server.key()
    claim = JSON.encode!(%{what: what, server: me})

    case Bucket.read(blob) do
      :absent ->
        Bucket.write(blob, claim) == :ok or claims?(blob, what)

      {:ok, was, tag} ->
        case JSON.decode!(was) do
          %{"what" => ^what, "server" => ^me} -> true
          %{"what" => ^what, "server" => server} -> ready?(server) == false and (Bucket.replace(blob, claim, tag) == :ok or claims?(blob, what))
          _another -> Bucket.replace(blob, claim, tag) == :ok or claims?(blob, what)
        end
    end
  end

  @doc """
  What was kept of who's ready is read again, once one of the network's servers came or went, and whoever waits for
  one to be back (`back/1`) looks again.
  """
  def presence_again do
    for {:presence, _key} = presence <- Cache.keys(__MODULE__), do: Cache.invalidate(__MODULE__, presence)

    Registry.dispatch(OneCore.Waiting, :seen, fn waiting -> for {process, _} <- waiting, do: send(process, :seen) end)
    :ok
  end

  @doc """
  Whether the server whose key is `key` is ready, as `presence/1` says, read again now. What was kept may be from
  before it came back: what the relay told this server of it while this one wasn't listening, or what it read of it
  while being told, is missed.
  """
  def presence_again(key) do
    Cache.invalidate(__MODULE__, {:presence, key})
    presence(key)
  end

  @doc """
  The network's edge, as its relay says, `%{"build" => <its build>, "protocol" => <the protocol it speaks>}`, and
  whether each of `keys`, devices' or servers', is here, as `presence/1` says, in `"presence"`; or `nil` where it can't
  be reached. A call from outside goes only to servers that speak a protocol it does (`OneCore.Protocol`).
  """
  def edge(keys \\ []) do
    if at = Application.get_env(:one_core, :address) do
      path = "/relay/seen"
      params = [keys: Enum.join(keys, ",")]
      response = Edge.send!(method: :get, url: at <> path, params: params, headers: Edge.signed(path, ""))
      build = List.first(Req.Response.get_header(response, "one-version"))
      body = if is_map(response.body), do: response.body, else: %{}
      %{"build" => build, "protocol" => Protocol.of(body["protocol"]), "presence" => body["presence"] || %{}}
    end
  rescue
    Edge.Unreachable -> nil
  end

  @doc """
  Tells the network's updater `said`, through its relay, `%{now: true}`, or with
  none asks it, and gives what it says of the network's update (`OneCore.Release.network/0`): `{:ok, update}`, or
  `{:ok, nil}` from a server with no edge address, or `{:error, why}`.
  """
  def network(said \\ nil) do
    if at = Application.get_env(:one_core, :address) do
      path = "/relay/update"
      body = if said, do: JSON.encode!(said), else: ""
      method = if said, do: :post, else: :get
      response = Edge.send!(method: method, url: at <> path, body: body, headers: Edge.signed(path, body))

      case response.status do
        200 -> {:ok, response.body}
        status -> {:error, "the edge said #{status}: #{inspect(response.body)}"}
      end
    else
      {:ok, nil}
    end
  rescue
    error in Edge.Unreachable -> {:error, Exception.message(error)}
  end

  @doc """
  Says at the network's edge what this server is (`OneCore.Release.status/0`): its `server_membership` and
  `server_daemon_status`, by which it's ready once it's active and running, where its update is, which the relay
  tells the network's updater, what its machine is called, and its build, by which the relay knows whether it talks
  with the edge. Where the edge can't be reached or doesn't take it, it's said again until it does (`Report`), as it is
  then, so that's no reason not to go on. Once it's taken, the program keeping this server running is told
  (`OneCore.Release.reported/1`).
  """
  def report do
    case Process.whereis(__MODULE__.Report) do
      nil -> say()
      reporting -> send(reporting, :say)
    end

    :ok
  end

  @doc false
  def say do
    if at = Application.get_env(:one_core, :address) do
      path = "/relay/seen"
      build = Builds.build()

      body =
        JSON.encode!(
          Map.merge(OneCore.Release.status(), %{
            "version" => build,
            "protocol" => Protocol.range(),
            "name" => Application.get_env(:one_core, :name),
            "installed" => Application.get_env(:one_core, :installed, false)
          })
        )

      response = Edge.send!(method: :post, url: at <> path, body: body, headers: Edge.signed(path, body))

      if response.status == 204 do
        OneCore.Release.reported(build)
      else
        {:error, "the edge didn't take #{body}: #{inspect(response.body)}"}
      end
    else
      :ok
    end
  rescue
    error in Edge.Unreachable -> {:error, Exception.message(error)}
  end

  defmodule Report do
    @moduledoc """
    Says this server's status at the edge (`OneCore.Servers.report/0`), one at a time and never holding up what
    changed it: what asks while it's saying one is said once that's done, as the status is then, so the edge is told
    the latest, in order, never one that's gone. Where the edge doesn't take it, as one that's briefly unreachable or
    being deployed, it's said again until it does: after a second, then twice as long each time, up to a minute.
    """
    use GenServer
    require Logger

    @first 1_000
    @most 60_000

    def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

    # Its state is how long it waits before it tries again, or `nil` while there's nothing to say again. It says the
    # status first as it starts.
    @impl true
    def init(nil) do
      send(self(), :say)
      {:ok, nil}
    end

    @impl true
    def handle_info(:say, wait) do
      said()
      say(wait, wait == nil)
    end

    # Said already since.
    def handle_info(:again, nil), do: {:noreply, nil}
    def handle_info(:again, wait), do: say(min(wait * 2, @most), true)

    # Says it, and where the edge doesn't take it, tries again after `wait`, unless a try is waiting already.
    defp say(wait, again) do
      case OneCore.Servers.say() do
        :ok ->
          {:noreply, nil}

        {:error, why} ->
          # The edge not taking it once is the internet's, and the next try is what does something about it: it's a
          # warning once it has gone on for minutes, waiting the longest between tries.
          Logger.log(
            if(wait == @most, do: :warning, else: :info),
            "Couldn't say this server's status at the edge, so it's said again until the edge takes it: #{why}"
          )

          if again, do: Process.send_after(self(), :again, wait || @first)
          {:noreply, wait || @first}
      end
    end

    # What asked meanwhile is said by this one, as the status is now.
    defp said do
      receive do
        :say -> said()
      after
        0 -> :ok
      end
    end
  end

  # How long a server that's stopping to start again, to update, takes to be back, in milliseconds: it finishes what
  # it took, then starts again, and connects to the edge as it does. Well within what an MCP client waits for an
  # answer, 30 seconds. One that isn't back by then may be verifying a build that doesn't start, which takes minutes.
  @back 20_000

  @doc """
  Waits for one of the servers whose keys are `keys` that's starting or stopping to be ready, for up to `@back`:
  where none of the others answered, and what was asked is that one's to answer, as a service only it runs. Gives
  its key, or `nil` if none is ready by then, or none of them was starting or stopping.
  """
  def back(keys) do
    coming = Enum.filter(keys, &(presence(&1)["server_daemon_status"] in ["starting", "stopping"]))

    if coming != [] do
      Registry.register(OneCore.Waiting, :seen, nil)
      names = Enum.map_join(coming, " or ", &(name(&1) || "a server"))
      OneCore.Call.State.working(OneCore.Call.State.id(), :waiting, "Waiting for #{names} to start again")

      try do
        await(coming, System.monotonic_time(:millisecond) + @back)
      after
        Registry.unregister(OneCore.Waiting, :seen)
      end
    end
  end

  # Looks after each time a server came or went, until one of `keys` is ready, or `until`, and then once more at the
  # edge: one that said it may take less than it did is back without having come or gone.
  defp await(keys, until) do
    case Enum.find(keys, &ready?/1) do
      nil ->
        receive do
          :seen -> await(keys, until)
        after
          max(until - System.monotonic_time(:millisecond), 0) -> Enum.find(keys, &presence_again(&1)["server_ready"])
        end

      key ->
        key
    end
  end

  @doc """
  The protocol the server whose key is `key` speaks (`OneCore.Protocol.of/1`), as it said when it last started or
  updated (`OneCore.Builds.say/1`), or `nil` where it never said.
  """
  def protocol(key) do
    cached({:protocol, key}, fn ->
      said = read("servers/#{key}/build", %{})
      Protocol.of(said["protocol"])
    end)
  end

  @doc "Which protocol the server whose key is `key` speaks is read again: it refused what it was asked for speaking another."
  def protocol_again(key), do: Cache.invalidate(__MODULE__, {:protocol, key})

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
    # A server acts for whoever's list it's on that it chose, both ways, as the presigner has it too: on their list,
    # it says whom it's for, its own blob, before it reads that it's on it. One it chose before keeps them.
    Bucket.write("servers/#{key}/members", JSON.encode!([owner]))
    key in read("#{owner}/servers", [])
  rescue
    Bucket.Refused -> false
  end

  @doc """
  A server that started says how to reach it now, what its machine is called and which build it is, and who it's for
  if it has never said. Each is its own blob, so they're written at once: each is a round trip to the bucket, which a
  server's start waits for.
  """
  def started(key, address, name, owner) do
    [
      fn -> say("servers/#{key}/address", address) end,
      fn -> say("servers/#{key}/name", JSON.encode!(name)) end,
      fn -> Builds.say(key) end,
      fn -> Bucket.write("servers/#{key}/members", JSON.encode!([owner])) end
    ]
    |> Task.async_stream(& &1.(), timeout: :infinity)
    |> Stream.run()

    invalidate()
  end

  @doc """
  Adds a server to `owner`'s list, which only this server, acting for them, may. The first on a list is written
  by a server acting for the network.
  """
  def add(key, owner) do
    added =
      Bucket.update("#{owner}/servers", fn
        nil ->
          if Server.key() in listed(@network),
            do: JSON.encode!([key]),
            else: {:error, "The first server to act for #{owner} is installed by one acting for the network"}

        keys ->
          if key in JSON.decode!(keys), do: keys, else: JSON.encode!(JSON.decode!(keys) ++ [key])
      end)

    invalidate()
    added
  end

  @doc """
  Takes a server off `owner`'s list, which only this server, acting for them, may: it no longer acts for them,
  and the presigner hands it nothing of theirs. One that isn't on it is off it already, as a server leaving that
  stopped part-way asks again. The network keeps a server, since its list is only ever begun once.
  """
  def remove(key, owner) do
    removed =
      Bucket.update("#{owner}/servers", fn
        nil ->
          nil

        listed ->
          case JSON.decode!(listed) do
            [^key] when owner == @network -> {:error, "The network keeps a server. Add another before this one goes."}
            keys -> if key in keys, do: JSON.encode!(keys -- [key]), else: listed
          end
      end)

    invalidate()
    removed
  end

  @doc "This server acts for `member` too from now on, once they have it on their list: its owner chose them."
  def choose(member) do
    :ok =
      Bucket.update("servers/#{Server.key()}/members", fn members ->
        if member in JSON.decode!(members), do: members, else: JSON.encode!(JSON.decode!(members) ++ [member])
      end)

    invalidate()
  end

  @doc """
  Writes one of a server's own blobs. Only that server writes it, so what's there is its own from before, though
  two of its own changes at once may each find the other's there: the later is written over it.
  """
  def say(blob, data), do: Bucket.update(blob, fn _was -> data end)

  @doc """
  Invalidates what's cached of the servers acting for `owner`: their list, and what each server on it acts for, has
  installed and is called. The network's first owner is the network's.
  """
  def reload(owner) do
    Cache.invalidate(__MODULE__, {:listed, owner})
    servers = for key <- listed(owner), what <- [{:members, key}, {:installed, key}, {:name, key}], do: what
    Enum.each(if(owner == @network, do: [:first_owner | servers], else: servers), &Cache.invalidate(__MODULE__, &1))
  end

  @doc "Invalidates everything cached here, so that it's read again when next asked for."
  def invalidate, do: Cache.invalidate(__MODULE__)

  @doc "What `read` gives for `what`, cached until it's invalidated."
  def cached(what, read), do: Cache.fetch(__MODULE__, what, fn -> {:commit, read.()} end)

  defp read(blob, absent) do
    case Bucket.read(blob) do
      {:ok, keys, _tag} -> JSON.decode!(keys)
      :absent -> absent
    end
  end
end
