defmodule OneCore.Server do
  @moduledoc """
  A server on its machine: it holds its key, in the folder that is its `:home` setting, and answers what clients
  ask, each message apart from the others.
  """
  use GenServer
  alias OneCore.{Iroh, Mcp}

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Where clients reach this server: its key and its addresses, as the JSON a client is told them in."
  def address, do: GenServer.call(__MODULE__, :address)

  @impl true
  def init(nil) do
    key = Path.join(Application.fetch_env!(:one_core, :home), "key")
    {:ok, {endpoint, address}} = Iroh.start(key, self())
    {:ok, %{endpoint: endpoint, address: address}}
  end

  @impl true
  def handle_call(:address, _from, server), do: {:reply, server.address, server}

  @impl true
  def handle_info({:ask, ask, message}, server) do
    Task.Supervisor.start_child(OneCore.Asks, fn -> Iroh.answer(ask, Mcp.answer(message)) end)
    {:noreply, server}
  end
end
