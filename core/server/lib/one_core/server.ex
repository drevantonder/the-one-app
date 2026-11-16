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

  @doc "The JSON-RPC error a server answers a call that was passed on to it and that it would pass on in turn."
  def forwarding_loop, do: -32001

  @doc """
  Asks the server whose key is `server` for `method`. Gives `{:ok, result}`, or `{:error, code, why}` if it refused;
  `:unreachable` if it couldn't be reached, or is known to be off, so it was asked nothing; or `:stopped` if it
  stopped answering after it was asked, so it may have done what was asked.
  """
  def request(server, method, params) do
    # A call that's passed on is one trace on both servers. It asks for MCP's progress notifications, which are how
    # this one knows the other is still running it, however long it takes (`client::Reached::ask`).
    said = %{progressToken: System.unique_integer([:positive])}
    said = if parent = Trace.traceparent(), do: Map.put(said, :traceparent, parent), else: said
    params = Map.update(params, :_meta, said, &Map.merge(&1, said))

    with true <- Servers.seen(server) in [:on, nil] or Servers.seen_again(server) in [:on, nil] or :unreachable,
         {:ok, answer} <- ask(server, JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params})) do
      case JSON.decode(answer) do
        {:ok, %{"result" => result}} -> {:ok, result}
        {:ok, %{"error" => %{"code" => code, "message" => why}}} -> {:error, code, why}
        # An answer cut short, or none, is from a server that went away after it was asked.
        _ -> :stopped
      end
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
  def said(:unreachable, server) do
    case Servers.seen(server) do
      at when is_integer(at) -> "#{named(server)} was last seen #{Servers.ago(at)}"
      _ -> "#{named(server)} isn't answering"
    end
  end

  def said(:stopped, server),
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
    params = %{for: owners, hooks: hooks}
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
  # one that's there answers at once (`Iroh.connect/4`); one that started again is somewhere new.
  defp ask(server, message) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)
    request = fn -> await(&Iroh.ask_server(endpoint, server, message, self(), &1)) end

    case request.() do
      :closed -> connect(endpoint, server, request)
      {:gone, _address} -> connect(endpoint, server, request)
      answered -> answered
    end
  end

  defp connect(endpoint, server, request) do
    with {:ok, address} <- Servers.address(server),
         :ok <- await(&Iroh.connect(endpoint, address, self(), &1)) do
      case request.() do
        :closed -> :unreachable
        {:gone, _address} -> :unreachable
        answered -> answered
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
    Servers.started(key, address, name, Application.get_env(:one_core, :for, "network"))
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
    answer = fn ->
      Plugins.started()
      Iroh.answer(ask, Mcp.answer(message, from))
    end

    Task.Supervisor.start_child(OneCore.Asks, answer)
    {:noreply, server}
  end
end
