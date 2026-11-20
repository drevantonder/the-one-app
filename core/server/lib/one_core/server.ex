defmodule OneCore.Server do
  @moduledoc """
  A server on its machine: it holds its key, in the folder that is its `:home` setting, answers what clients and
  other servers ask, each message apart from the others, and asks other servers. It tells the clients connected to
  it when something changed, and the other servers when there's something to look for in the bucket. Its network's
  address, if the network has one, is its `:address` setting.

  Everything a server keeps on its machine is in its home, where the clients watching it are among them, so one that
  starts again tells them too.
  """
  use GenServer
  require Logger
  alias OneCore.{Builds, Edge, Iroh, Protocol, Servers}
  alias OpenTelemetryAPIExperimental.Histogram
  require Histogram
  alias OneCore.Call.State

  @doc """
  Starts this server's endpoint. What it can't do itself is handed it, as a web server is handed what answers its
  requests: `handle`, which answers a message (`OneCore.Mcp.handle/3`); `wake`, which has the apps that can wake a
  device wake it, and says whether one did (`OneCore.Hooks.wake/1`); and `started`, which tells the clients watching
  it that something changed (`OneCore.Apps.changed/2`), as it starts.
  """
  def start_link(options), do: GenServer.start_link(__MODULE__, Map.new(options), name: __MODULE__)

  @doc "Tells every client watching this server `notifications/one/<what>`, with `params`."
  def tell(what, params) do
    notification = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/" <> what, params: params})
    GenServer.cast(__MODULE__, {:notify, notification})
  end

  @doc """
  Keeps that the client whose key is `client`, a run of `device`, wants to hear what changes, and is reached at
  `address` to be told it. With no address, it no longer does. A client that can't be reached there is forgotten.
  """
  def watch(client, address, device) do
    with {nif, endpoint} <- endpoint(), do: nif.watch(endpoint, client, address && JSON.encode!(address), device)

    if address && device,
      do: Registry.dispatch(OneCore.Waiting, {:watching, device}, &for({process, _} <- &1, do: send(process, {:watching, device})))
  end

  @doc """
  The devices whose app is watching this server, by their keys: each is reached by `ask_device/3` while it runs.
  None with no server started.
  """
  def devices do
    case endpoint() do
      {nif, endpoint} -> nif.devices(endpoint)
      nil -> []
    end
  end

  # How long an endpoint closed as the server stops answers what it took already (`terminate/2`).
  @drains 5_000

  # How long a device that was woken has to watch again: a push reaches a phone in a second or two, and its app
  # starts and connects in a few more.
  @wakes 20_000

  @doc """
  Asks the device whose key is `device` one JSON-RPC request, `message`, as its app runs: a device's app answers a
  call of a tool that acts on the device itself (`client::DeviceTools`), as MCP's `tools/call`. Gives `{:ok, answer}`,
  `:unreachable` where its app isn't watching this server or didn't answer, so it was asked nothing, or `:stopped`
  where it stopped answering after it was asked, so it may have done it. Who may ask it what is the asking app's to
  decide: the device answers any server acting for its person.

  A device whose app isn't running is woken first: the apps installed for the network that can wake one (the hook
  `wake`, as Notifications does by a push) are asked to, and it's asked once its app watches again, within
  #{div(@wakes, 1000)} seconds. One that no app woke, or that didn't watch in time, is `:unreachable`. With
  `wake: false` it isn't, as for what's only asked of devices that are awake.
  """
  def ask_device(device, message, options \\ []) do
    ask = fn ->
      case endpoint() do
        {nif, endpoint} -> await(&nif.ask_client(endpoint, device, message, self(), &1))
        nil -> :unreachable
      end
    end

    with :unreachable <- ask.(),
         true <- Keyword.get(options, :wake, true),
         :watching <- wake(device) do
      ask.()
    else
      false -> :unreachable
      outcome -> outcome
    end
  end

  # Has the apps that can wake a device wake it, and waits for it to watch this server again: `:watching`, or
  # `:unreachable` where none woke it or it didn't watch in time.
  defp wake(device) do
    Registry.register(OneCore.Waiting, {:watching, device}, nil)

    try do
      if :persistent_term.get({__MODULE__, :wake}).(device) do
        receive do
          {:watching, ^device} -> :watching
        after
          @wakes -> :unreachable
        end
      else
        :unreachable
      end
    after
      Registry.unregister(OneCore.Waiting, {:watching, device})
    end
  end

  @doc "The folder this server keeps what's its own on this machine in."
  def home, do: Application.fetch_env!(:one_core, :home)

  @doc "The file this server's key is kept in."
  def key_file, do: Path.join(home(), "key")

  @doc "This server's key, or `nil` where no server is running."
  def key, do: elem(:persistent_term.get(__MODULE__, {nil, nil}), 0)

  # This server's endpoint, as `{nif, endpoint}`: the build of its NIF that opened it (`OneCore.Nif`), which is the one
  # it's used with. `nil` while none is open.
  defp endpoint, do: elem(:persistent_term.get(__MODULE__, {nil, nil}), 1)

  @doc """
  Whether this server is ready: it's active, one of its network's servers, and running, which a build starting is
  once it has answered by its key, as the program keeping it running says (`OneCore.Release`). One that's joining,
  leaving or stopping isn't. One that isn't ready takes on nothing that others then go to it for, such as a
  database's lease or building every app for them (`OneCore.Apps.Deploy.rebuild/0`): it still runs what it's
  asked, and asks for the rest.
  """
  def up?, do: ready?(OneCore.Release.status())

  defp ready?(status),
    do: status["server_membership"] == "active" and status["server_daemon_status"] == "running"

  @doc "Returns once this server is ready (`up?/0`)."
  def await_up, do: OneCore.Release.await(&ready?/1)

  @doc "The JSON-RPC error a server answers a call that was passed on to it and that it would pass on in turn."
  def forwarding_loop, do: -32001

  @doc "The JSON-RPC error a server answers where it would pass `what` it was asked on in turn."
  def forwarding_loop(what), do: {:error, forwarding_loop(), "This server would pass #{what} on in turn"}

  @doc """
  Asks the server whose key is `server` for `method`. Gives `{:ok, result}`, or `{:error, code, why}` if it refused;
  `:unreachable` if it couldn't be reached, isn't ready, or speaks only a protocol older than this server's, so they
  don't talk (`OneCore.Protocol`); or `:stopped` if it stopped answering after it was asked, so it may have done what
  was asked. One that speaks only a newer one refuses, saying this one is behind.

  Every request says the protocol this server speaks, as `_meta.protocol`, and its build, as `_meta.version`, and one
  of a protocol the server asked doesn't speak is refused before anything of it is read (`OneCore.Mcp`). One behind
  is skipped, as if it were off, and one ahead refuses what this one asks.

  Only a server that's ready (`OneCore.Servers.presence/1`), or isn't known not to be, is asked. With `ready: :any`
  one that isn't is asked too, unless it's stopped, for what only it can answer: a server that's starting answers
  once its apps have, and is ready only once it has answered by its key.
  """
  def request(server, method, params, opts \\ []) do
    # A call that's passed on is one trace on both servers. It asks for MCP's progress notifications, which are how
    # this one knows the other is still running it, however long it takes (`client::Reached::ask`).
    # Its id is its own, so cancelling the call it's for tells that server which of its calls to cancel.
    id = System.unique_integer([:positive])
    meta = %{progressToken: id, protocol: Protocol.range(), version: Builds.build()}
    meta = Map.merge(meta, Map.new(:otel_propagator_text_map.inject([])))
    params = Map.update(params, :_meta, meta, &Map.merge(&1, meta))
    message = JSON.encode!(%{jsonrpc: "2.0", id: id, method: method, params: params})

    asked? = fn presence ->
      presence["server_ready"] != false or (opts[:ready] == :any and presence["server_daemon_status"] != "stopped")
    end

    with true <- asked?.(Servers.presence(server)) or asked?.(Servers.presence_again(server)) or :unreachable,
         {:ok, response} <- passed(method, server, id, fn -> ask(server, message, State.answering()) end) do
      case JSON.decode(response) do
        {:ok, %{"result" => result}} -> {:ok, result}
        {:ok, %{"error" => %{"code" => -32022, "data" => %{} = data}}} -> refused(server, data)
        {:ok, %{"error" => %{"code" => code, "message" => why}}} -> {:error, code, why}
        # An answer cut short, or none, is from a server that went away after it was asked.
        _ -> :stopped
      end
    end
  end

  # A call passed on is routed there while it runs, so cancelling it tells that server: anything else asked, such
  # as a hook while it's checked, is part of what it's doing here.
  defp passed("tools/call", server, id, ask), do: State.forwarding(server, id, ask)
  defp passed(_method, _server, _id, ask), do: ask.()

  # A server that doesn't speak this one's protocol refused it (`OneCore.Mcp`), saying the range it speaks. One that's behind is updated in its turn: it's as if it were off, so whoever
  # asked asks another. One that's ahead was updated first, and this one is the one behind.
  defp refused(server, %{} = data) do
    Servers.protocol_again(server)
    theirs = Protocol.of(data["supported"])

    why =
      "#{named(server)} speaks protocol #{Protocol.name(theirs)} and #{named(key())} #{Protocol.name(Protocol.range())}, so they don't talk until one is updated"

    Logger.warning(why)

    if Protocol.behind?(theirs),
      do: :unreachable,
      else: {:error, -32022, "#{why}: #{named(key())} is the one behind, and is asked nothing until it's updated"}
  end

  @doc """
  Stops taking what's asked, as a server does while it stops: whoever asks it or connects to it is refused, so they
  ask another, and what it took already is answered, for up to `within` milliseconds. Then its endpoint closes.
  """
  def close(within) do
    case endpoint() do
      {nif, endpoint} -> nif.close(endpoint, within)
      nil -> :ok
    end
  end

  @doc "What whoever asked is told of the server `server` that didn't answer `request/3`, by its machine's name."
  def error_message(:unreachable, server) do
    presence = Servers.presence(server) || %{}

    cond do
      not Protocol.speaks?(Servers.protocol(server)) ->
        "#{named(server)} speaks protocol #{Protocol.name(Servers.protocol(server))} and #{named(key())} #{Protocol.name(Protocol.range())}, so it's asked nothing until it's updated"

      presence["last_seen"] ->
        "#{named(server)} was last seen #{Servers.ago(presence["last_seen"])}"

      why = presence["server_ready_reason"] ->
        "#{named(server)} isn't ready: it's #{why}"

      true ->
        "#{named(server)} isn't answering"
    end
  end

  def error_message(:stopped, server),
    do: "#{named(server)} went away before it answered, as a server does while it restarts, so this may or may not have been done. Look before trying again"

  defp named(server), do: Servers.name(server) || "That server"

  @doc """
  Tells the other servers running what's `owners'` that there's something new of theirs in the bucket to `look` for: a
  app's version, a server, a service installed, with `params` saying more: `apps`, which changed, or for `invalidate`,
  the `hooks` whose results were decided from what has since changed. Each has looked or forgotten, or couldn't be
  reached or didn't within its deadline (`OneCore.Asks.each/3`), when this returns.
  """
  def notify_servers(owners, what \\ "reload", params \\ %{}),
    do: notify(Enum.uniq(Enum.flat_map(owners, &told/1)), owners, what, params)

  # The network's apps run on every server, a person's own too, for them (`OneCore.Apps.loads?/1`), and anyone else's on
  # the servers acting for them.
  defp told("network"), do: Servers.up()
  defp told(owner), do: Servers.covering(owner)

  @doc "Tells `server` to cancel the call it's running for what this server asked it as `request` (`request/3`)."
  def cancel(server, request) do
    notification = %{jsonrpc: "2.0", method: "notifications/cancelled", params: %{requestId: request}}
    Task.Supervisor.start_child(OneCore.Asks, fn -> ask(server, JSON.encode!(notification)) end)
    :ok
  end

  @doc "Tells `servers`, by their keys, what `notify_servers/3` tells the servers acting for `owners`."
  def notify(servers, owners, what \\ "reload", params \\ %{}) do
    params = Map.put(params, :owners, owners)
    notification = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/" <> what, params: params})
    OneCore.Asks.each(servers -- [key()], &ask(&1, notification))
    :ok
  end

  # A connection is opened when there's none, to where the server says it is now. One whose connection no longer
  # answered may only have lost what it was sent, as a network now and then does, so it's connected to again, which
  # one that's there answers at once (`Iroh.connect/4`), one that started again too. What the server asked says it's
  # doing is said of `relay`, the message this one is answering by asking it, if that came over this endpoint: one
  # that came over another, closed since, has nobody to say it to.
  defp ask(server, message, relay \\ nil) do
    case endpoint() do
      {nif, endpoint} ->
        relay = with {^nif, ask} <- relay, do: ask, else: (_ -> nil)
        request = fn -> await(&nif.ask_server(endpoint, server, message, self(), &1, relay)) end

        case request.() do
          :closed -> connect(nif, endpoint, server, request)
          {:gone, _address} -> connect(nif, endpoint, server, request)
          response -> response
        end

      nil ->
        :unreachable
    end
  end

  defp connect(nif, endpoint, server, request) do
    with {:ok, address} <- Servers.address(server),
         :connected <- connected(server, fn -> await(&nif.connect(endpoint, address, self(), &1)) end) do
      case request.() do
        :closed -> :unreachable
        {:gone, _address} -> :unreachable
        response -> response
      end
    else
      _ -> :unreachable
    end
  end

  # Connects, and says how: how long it took and the path it's on (Telemetry's `one.server.connect.duration`), and
  # where it didn't, why and how this server reached its network's relay then, which is the way a server that can't
  # be reached directly is reached.
  defp connected(server, connect) do
    began = System.monotonic_time(:millisecond)
    connected = connect.()
    took = System.monotonic_time(:millisecond) - began
    to = Servers.name(server) || String.slice(server, 0, 8)

    case connected do
      {:connected, path} ->
        Histogram.record(:"one.server.connect.duration", took, %{"one.server" => to, "one.path" => to_string(path)})
        :connected

      {:unreachable, why, relay} ->
        Histogram.record(:"one.server.connect.duration", took, %{"one.server" => to, "one.path" => "unreachable"})
        Logger.warning("Couldn't reach #{to} in #{took} ms: #{why}. This server's relay: #{relay}")
        :unreachable
    end
  end

  defp await(ask) do
    reference = make_ref()
    ask.(reference)

    receive do
      {^reference, outcome} -> outcome
    end
  end

  @impl true
  def init(options) do
    # What it opened closes as it stops (`terminate/2`).
    Process.flag(:trap_exit, true)
    network = if at = Application.get_env(:one_core, :address), do: {at, Edge.token()}
    nif = OneCore.Nif.build(Iroh)
    {:ok, {endpoint, address}} = nif.start(key_file(), network, self())
    key = JSON.decode!(address)["id"]
    :persistent_term.put(__MODULE__, {key, {nif, endpoint}})
    :persistent_term.put({__MODULE__, :wake}, options.wake)
    {:ok, machine} = :inet.gethostname()
    name = Application.get_env(:one_core, :name) || to_string(machine)
    owner = Application.get_env(:one_core, :owner, "network")
    Servers.started(key, address, name, owner)
    # The servers that answer for whom it's for (the network's too) look again at who acts for them: one that cached
    # them before this one said it's theirs, as a group's server's first start, otherwise took it for nobody's.
    Task.start(fn -> notify_servers([owner]) end)
    # The clients that were watching before it stopped may have missed something meanwhile, a new build of its
    # apps among them, so they're told to ask again.
    options.started.()
    {:ok, %{nif: nif, endpoint: endpoint, handle: options.handle}}
  end

  # It closes its endpoint as it stops: it takes nothing new from now on, and what it took already is answered over it
  # for up to `@drains`.
  @impl true
  def terminate(_why, server) do
    :persistent_term.put(__MODULE__, {key(), nil})
    Task.start(fn -> server.nif.close(server.endpoint, @drains) end)
  end

  @impl true
  def handle_cast({:notify, notification}, server) do
    server.nif.notify(server.endpoint, notification)
    {:noreply, server}
  end

  @impl true
  def handle_info({:ask, ask, from, message}, server) do
    reply = fn -> server.nif.reply(ask, server.handle.(message, from, {server.nif, ask})) end

    Task.Supervisor.start_child(OneCore.Asks, reply)
    {:noreply, server}
  end
end
