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
  alias OneCore.{Edge, Hooks, Iroh, Mcp, Plugins, Servers, Trace}

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Tells every client that's watching that something changed, so that it asks again. With no server started, nobody is told."
  def changed do
    GenServer.cast(__MODULE__, {:notify, JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/changed"})})
  end

  @doc """
  Keeps that the client whose key is `client` wants to hear what changes, and is reached at `address` to be told
  it. With no address, it no longer does. A client that can't be reached there is forgotten.
  """
  def watch(client, address) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)
    Iroh.watch(endpoint, client, address && JSON.encode!(address))
  end

  @doc "The folder this server keeps what's its own on this machine in."
  def home, do: Application.fetch_env!(:one_core, :home)

  @doc "The file this server's key is kept in."
  def key_file, do: Path.join(home(), "key")

  @doc "This server's key, or `nil` where no server is running."
  def key, do: elem(:persistent_term.get(__MODULE__, {nil, nil}), 0)

  @doc """
  Whether this server is up: reached by its key, and not stopping. A build being tried isn't up until it has
  answered by its key, which the program keeping it running says by ending its `starting` stage
  (`OneCore.Release`), and a server stopping, to start again or for good, isn't up any more. One that isn't up
  takes on nothing that others then go to it for, such as a database's lease or building every app for them
  (`OneCore.Plugins.Deploy.rebuild/0`): it still runs what it's asked, and asks for the rest.
  """
  def up? do
    OneCore.Builds.update() not in ["starting", "restarting"] and not Application.get_env(:one_core, :stopping, false)
  end

  @doc "Returns once this server is up (`up?/0`). A server that's stopping isn't up again."
  def await_up do
    Registry.register(OneCore.Waiting, :update, nil)
    awaiting_up()
  after
    Registry.unregister(OneCore.Waiting, :update)
  end

  defp awaiting_up do
    if not up?(), do: receive(do: ({:update, _stage} -> awaiting_up()))
    :ok
  end

  @doc "The JSON-RPC error a server answers a call that was passed on to it and that it would pass on in turn."
  def forwarding_loop, do: -32001

  @doc """
  The version of what servers ask each other: the methods only a server may ask (`OneCore.Mcp`), what a call passed
  on says of itself (`_meta`) and the answers each gives. Every request a server sends another says it, as
  `_meta.protocol`, and one that isn't of this version is refused before anything of it is read, as MCP refuses a
  protocol version it doesn't speak (`-32022`, with the versions it does). So a change that a server on the build
  before would read wrongly, such as a renamed field, adds one here, and while a release puts it on each server in
  turn, a server asks only those on its own version: one on the version before is skipped, as if it were off, and
  one that's on the next refuses what this one asks. A notification only has another server look again at what's in
  the bucket, which no version reads wrongly.
  """
  def protocol, do: 1

  @doc """
  Asks the server whose key is `server` for `method`. Gives `{:ok, result}`, or `{:error, code, why}` if it refused;
  `:unreachable` if it couldn't be reached, is known to be off or restarting, or is on the protocol before this
  server's (`protocol/0`), so it was asked nothing; or `:stopped` if it stopped answering after it was asked, so it
  may have done what was asked. One on a newer protocol than this server refuses, saying this one is behind.
  """
  def request(server, method, params) do
    # A call that's passed on is one trace on both servers. It asks for MCP's progress notifications, which are how
    # this one knows the other is still running it, however long it takes (`client::Reached::ask`).
    meta = %{progressToken: System.unique_integer([:positive]), protocol: protocol()}
    meta = if parent = Trace.traceparent(), do: Map.put(meta, :traceparent, parent), else: meta
    params = Map.update(params, :_meta, meta, &Map.merge(&1, meta))

    with true <- Servers.seen(server) in [:on, nil] or Servers.seen_again(server) in [:on, nil] or :unreachable,
         {:ok, response} <- ask(server, JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params})) do
      case JSON.decode(response) do
        {:ok, %{"result" => result}} -> {:ok, result}
        {:ok, %{"error" => %{"code" => -32022, "data" => %{"supported" => [theirs]}}}} -> refused(server, theirs)
        {:ok, %{"error" => %{"code" => code, "message" => why}}} -> {:error, code, why}
        # An answer cut short, or none, is from a server that went away after it was asked.
        _ -> :stopped
      end
    end
  end

  # A server on another protocol asked nothing. One that's behind is updated in its turn, which a release does
  # next: it's as if it were off, so whoever asked asks another. One that's ahead was updated first, and this one
  # is the one behind.
  defp refused(server, theirs) do
    Servers.protocol_again(server)

    if theirs < protocol() do
      :unreachable
    else
      {:error, -32022, "#{named(key())} is on an older build than #{named(server)}, so it asks it nothing until it's updated"}
    end
  end

  @doc """
  Stops taking what's asked, as a server does while it stops: whoever asks it or connects to it is refused, so they
  ask another, and what it took already is answered, for up to `within` milliseconds. Then its endpoint closes.
  """
  def close(within) do
    case :persistent_term.get(__MODULE__, nil) do
      {_key, endpoint} -> Iroh.close(endpoint, within)
      nil -> :ok
    end
  end

  @doc "What whoever asked is told of the server `server` that didn't answer `request/3`, by its machine's name."
  def error_message(:unreachable, server) do
    seen = Servers.seen(server)

    cond do
      seen == :restarting -> "#{named(server)} is restarting"
      is_integer(seen) -> "#{named(server)} was last seen #{Servers.ago(seen)}"
      Servers.protocol(server) < protocol() -> "#{named(server)} is on an older build until it's updated"
      true -> "#{named(server)} isn't answering"
    end
  end

  def error_message(:stopped, server),
    do: "#{named(server)} went away before it answered, as a server does while it restarts, so this may or may not have been done. Look before trying again"

  defp named(server), do: Servers.name(server) || "That server"

  @doc """
  Tells the other servers acting for `owners` that there's something new of theirs in the bucket to `look` for: a
  plugin's version, a server, a service installed. Or that what they kept of `hooks` was `decided` from what has
  since changed. Each has looked or forgotten, or couldn't be reached, when this returns.
  """
  def notify_servers(owners, what \\ "reload", hooks \\ []),
    do: notify(Enum.uniq(Enum.flat_map(owners, &Servers.acting/1)), owners, what, hooks)

  @doc "Tells `servers`, by their keys, what `notify_servers/3` tells the servers acting for `owners`."
  def notify(servers, owners, what \\ "reload", hooks \\ []) do
    params = %{owners: owners, hooks: hooks}
    notification = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/" <> what, params: params})
    Stream.run(Task.async_stream(servers -- [key()], &ask(&1, notification), timeout: :infinity))
  end

  @doc """
  Looks in the bucket at what other servers put there for `owner`: the servers on their list, what each acts for
  and the services it runs, and what's installed for them, running the latest version of each plugin. Looking at the
  network forgets whose each device is too, which the network's plugins say. Its clients are told if a plugin
  changed.

  A server looks at someone when it's told to, when it's asked for them and last looked more than five minutes
  ago (`OneCore.Plugins`), and when it comes back (`reconnected/0`). Nothing else has it look, so someone nobody
  uses costs nothing.
  """
  def reload(owner) do
    Servers.reload(owner)
    OneCore.Plugins.Deploy.forget()
    if owner == "network", do: Hooks.forget(["device"])
    if Plugins.reload(owner), do: changed()
    :ok
  end

  @doc """
  This server can reach the others again, its network's address or its bucket, after it couldn't, so it may have
  missed being told something: it forgets what it kept of servers and hooks, looks at once at who it acts for, and
  at everyone else it has looked at behind the next request for them (`OneCore.Plugins.refresh/0`).
  """
  def reconnected do
    Hooks.forget()
    Servers.forget()
    Enum.each(Plugins.refresh(), &reload/1)
  end

  # A connection is opened when there's none, to where the server says it is now. One whose connection no longer
  # answered may only have lost what it was sent, as a network now and then does, so it's connected to again, which
  # one that's there answers at once (`Iroh.connect/4`), one that started again too.
  defp ask(server, message) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)
    request = fn -> await(&Iroh.ask_server(endpoint, server, message, self(), &1)) end

    case request.() do
      :closed -> connect(endpoint, server, request)
      {:gone, _address} -> connect(endpoint, server, request)
      response -> response
    end
  end

  defp connect(endpoint, server, request) do
    with {:ok, address} <- Servers.address(server),
         :ok <- await(&Iroh.connect(endpoint, address, self(), &1)) do
      case request.() do
        :closed -> :unreachable
        {:gone, _address} -> :unreachable
        response -> response
      end
    else
      _ -> :unreachable
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
  def init(nil) do
    network = if at = Application.get_env(:one_core, :address), do: {at, Edge.token()}
    {:ok, {endpoint, address}} = Iroh.start(key_file(), network, self())
    key = JSON.decode!(address)["id"]
    :persistent_term.put(__MODULE__, {key, endpoint})
    {:ok, machine} = :inet.gethostname()
    name = Application.get_env(:one_core, :name) || to_string(machine)
    Servers.started(key, address, name, Application.get_env(:one_core, :owner, "network"))
    # The clients that were watching before it stopped may have missed something meanwhile, a new build of its
    # plugins among them, so they're told to ask again.
    changed()
    {:ok, %{endpoint: endpoint}}
  end

  @impl true
  def handle_cast({:notify, notification}, server) do
    Iroh.notify(server.endpoint, notification)
    {:noreply, server}
  end

  # A server is reached as soon as it starts, and answers what it's asked once its plugins are there, since they
  # say whose a device is: so whoever asks while they start waits for them, and isn't told it can't be reached.
  @impl true
  def handle_info({:ask, ask, from, message}, server) do
    reply = fn ->
      Plugins.started()
      Iroh.reply(ask, Mcp.handle(message, from))
    end

    Task.Supervisor.start_child(OneCore.Asks, reply)
    {:noreply, server}
  end
end
