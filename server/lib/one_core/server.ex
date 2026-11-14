defmodule OneCore.Server do
  @moduledoc """
  A server on its machine: it holds its key, in the folder that is its `:home` setting, answers what clients and
  other servers ask, each message apart from the others, and asks other servers. It can also tell the clients
  connected to it something. Its network's address, if the network has one, is its `:address` setting.
  """
  use GenServer
  alias OneCore.{Address, Iroh, Mcp, Servers}

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Where clients reach this server: its key and its addresses, as the JSON a client is told them in."
  def address, do: GenServer.call(__MODULE__, :address)

  @doc "Tells every connected client `notification`, in MCP's JSON-RPC. With no server started, nobody is told."
  def tell(notification), do: GenServer.cast(__MODULE__, {:tell, JSON.encode!(notification)})

  @doc "The file this server's key is kept in."
  def key_file, do: Path.join(Application.fetch_env!(:one_core, :home), "key")

  @doc "This server's key, or `nil` where no server is running."
  def key, do: elem(:persistent_term.get(__MODULE__, {nil, nil}), 0)

  @doc "The JSON-RPC error a server answers a call that was passed on to it and that it would pass on in turn."
  def passed_back, do: -32001

  @doc """
  Asks the server whose key is `server` one message. Gives `{:ok, answer}`; `:unreachable` if it couldn't be
  reached, so it was asked nothing; or `:stopped` if it stopped answering after it was asked.
  """
  def ask(server, message) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)

    # A connection is opened when there's none, to where the server says it is now.
    with :closed <- told(&Iroh.ask_server(endpoint, server, message, self(), &1)),
         {:ok, address} <- Servers.address(server),
         :ok <- told(&Iroh.connect(endpoint, address, self(), &1)),
         :closed <- told(&Iroh.ask_server(endpoint, server, message, self(), &1)) do
      :unreachable
    else
      :absent -> :unreachable
      outcome -> outcome
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
