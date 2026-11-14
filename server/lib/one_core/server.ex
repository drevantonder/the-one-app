defmodule OneCore.Server do
  @moduledoc """
  A server on its machine: it holds its key, in the folder that is its `:home` setting, answers what clients and
  other servers ask, each message apart from the others, and asks other servers. It tells the clients connected to
  it when something changed, and the other servers when there's something to look for in the bucket. Its network's
  address, if the network has one, is its `:address` setting.

  Everything a server keeps on its machine is in its home.
  """
  use GenServer
  alias OneCore.{Address, Iroh, Mcp, Plugins, Servers, Services}

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Where clients reach this server: its key and its addresses, as the JSON a client is told them in."
  def address, do: GenServer.call(__MODULE__, :address)

  @doc "Tells every connected client that something changed, so that it asks again. With no server started, nobody is told."
  def changed do
    GenServer.cast(__MODULE__, {:tell, JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/changed"})})
  end

  @doc "The folder this server keeps what's its own on this machine in."
  def home, do: Application.fetch_env!(:one_core, :home)

  @doc "The file this server's key is kept in."
  def key_file, do: Path.join(home(), "key")

  @doc "This server's key, or `nil` where no server is running."
  def key, do: elem(:persistent_term.get(__MODULE__, {nil, nil}), 0)

  @doc "The JSON-RPC error a server answers a call that was passed on to it and that it would pass on in turn."
  def passed_back, do: -32001

  @doc """
  Asks the server whose key is `server` for `method`. Gives `{:ok, result}`, or `{:error, code, why}` if it refused;
  `:unreachable` if it couldn't be reached, so it was asked nothing; or `:stopped` if it stopped answering after it
  was asked, so it may have done what was asked.
  """
  def request(server, method, params) do
    with {:ok, answer} <- ask(server, JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params})) do
      case JSON.decode!(answer) do
        %{"result" => result} -> {:ok, result}
        %{"error" => %{"code" => code, "message" => why}} -> {:error, code, why}
      end
    end
  end

  @doc "What whoever asked is told of a server that didn't answer `request/3`."
  def said(:unreachable), do: "That server isn't up"
  def said(:stopped), do: "That server stopped answering after it was asked, so this may or may not have been done"

  @doc """
  Tells the other servers acting that there's something new in the bucket to look for: a plugin's version, a
  server, a service installed. Each has looked, or couldn't be reached, when this returns.
  """
  def tell_servers do
    look = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/look"})
    Stream.run(Task.async_stream(Servers.acting() -- [key()], &ask(&1, look), timeout: :infinity))
  end

  @doc """
  Looks in the bucket for what other servers put there: plugins' newer versions, the servers acting and the
  services they run. Its clients are told if a plugin changed.
  """
  def look do
    Services.look()
    if Plugins.look(), do: changed()
    :ok
  end

  # A connection is opened when there's none, to where the server says it is now. A server whose connection no
  # longer answers has gone, unless it says it's somewhere else now: one that starts again listens somewhere new.
  defp ask(server, message) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)
    asked = fn -> told(&Iroh.ask_server(endpoint, server, message, self(), &1)) end

    case asked.() do
      :closed -> connect(endpoint, server, nil, asked)
      {:gone, was} -> connect(endpoint, server, was, asked)
      answered -> answered
    end
  end

  defp connect(endpoint, server, was, asked) do
    with {:ok, address} when address != was <- Servers.address(server),
         :ok <- told(&Iroh.connect(endpoint, address, self(), &1)) do
      case asked.() do
        :closed -> :unreachable
        {:gone, _address} -> :unreachable
        answered -> answered
      end
    else
      _ -> :unreachable
    end
  end

  defp told(ask) do
    reference = make_ref()
    ask.(reference)

    receive do
      {^reference, outcome} -> outcome
    end
  end

  @impl true
  def init(nil) do
    network = if at = Application.get_env(:one_core, :address), do: {at, Address.pass()}
    {:ok, {endpoint, address}} = Iroh.start(key_file(), network, self())
    key = JSON.decode!(address)["id"]
    :persistent_term.put(__MODULE__, {key, endpoint})
    Servers.started(key, address)
    {:ok, %{endpoint: endpoint, address: address}}
  end

  @impl true
  def handle_call(:address, _from, server), do: {:reply, server.address, server}

  @impl true
  def handle_cast({:tell, notification}, server) do
    Iroh.tell(server.endpoint, notification)
    {:noreply, server}
  end

  @impl true
  def handle_info({:ask, ask, from, message}, server) do
    Task.Supervisor.start_child(OneCore.Asks, fn -> Iroh.answer(ask, Mcp.answer(message, from)) end)
    {:noreply, server}
  end
end
