defmodule OneCore.Server do
  @moduledoc """
  A server on its machine: it holds its key, in the folder that is its `:home` setting, answers what clients and
  other servers ask, each message apart from the others, and asks other servers. It tells the clients connected to
  it when something changed, and the other servers when there's something to look for in the bucket. Its network's
  address, if the network has one, is its `:address` setting.

  Everything a server keeps on its machine is in its home.
  """
  use GenServer
  alias OneCore.{Address, Hooks, Iroh, Mcp, Plugins, Servers}

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc "Tells every client that's watching that something changed, so that it asks again. With no server started, nobody is told."
  def changed do
    GenServer.cast(__MODULE__, {:tell, JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/changed"})})
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
  def passed_back, do: -32001

  @doc """
  Asks the server whose key is `server` for `method`. Gives `{:ok, result}`, or `{:error, code, why}` if it refused;
  `:unreachable` if it couldn't be reached, or is known to be off, so it was asked nothing; or `:stopped` if it
  stopped answering after it was asked, so it may have done what was asked.
  """
  def request(server, method, params) do
    with true <- Servers.seen(server) in [:on, nil] or :unreachable,
         {:ok, answer} <- ask(server, JSON.encode!(%{jsonrpc: "2.0", id: 1, method: method, params: params})) do
      case JSON.decode!(answer) do
        %{"result" => result} -> {:ok, result}
        %{"error" => %{"code" => code, "message" => why}} -> {:error, code, why}
      end
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
    do: "#{named(server)} stopped answering after it was asked, so this may or may not have been done"

  defp named(server), do: Servers.name(server) || "That server"

  @doc """
  Tells the other servers acting for `owners` that there's something new of theirs in the bucket to `look` for: a
  plugin's version, a server, a service installed. Or that what they kept of `hooks` was `decided` from what has
  since changed. Each has looked or forgotten, or couldn't be reached, when this returns.
  """
  def tell_servers(owners, what \\ "look", hooks \\ []),
    do: tell(Enum.uniq(Enum.flat_map(owners, &Servers.acting/1)), owners, what, hooks)

  @doc "Tells `servers`, by their keys, what `tell_servers/3` tells the servers acting for `owners`."
  def tell(servers, owners, what \\ "look", hooks \\ []) do
    params = %{for: owners, hooks: hooks}
    told = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/" <> what, params: params})
    Stream.run(Task.async_stream(servers -- [key()], &ask(&1, told), timeout: :infinity))
  end

  @doc """
  Looks in the bucket at what other servers put there for `owner`: the servers on their list, what each acts for
  and the services it runs, and what's added for them, running the newest version of each plugin. Looking at the
  network forgets whose each device is too, which the network's plugins say. Its clients are told if a plugin
  changed.

  A server looks at someone when it's told to, when it's asked for them and last looked more than five minutes
  ago (`OneCore.Plugins`), and when it comes back (`came_back/0`). Nothing else has it look, so someone nobody
  uses costs nothing.
  """
  def look(owner) do
    Servers.look(owner)
    if owner == "network", do: Hooks.forget(["device"])
    if Plugins.look(owner), do: changed()
    :ok
  end

  @doc """
  This server can reach the others again, its network's address or its bucket, after it couldn't, so it may have
  missed being told something: it forgets what it kept of servers and hooks, looks at once at who it acts for, and
  at everyone else it has looked at behind the next request for them (`OneCore.Plugins.look_again/0`).
  """
  def came_back do
    Hooks.forget()
    Servers.forget()
    Enum.each(Plugins.look_again(), &look/1)
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
    {:ok, machine} = :inet.gethostname()
    name = Application.get_env(:one_core, :name) || to_string(machine)
    Servers.started(key, address, name, Application.get_env(:one_core, :for, "network"))
    {:ok, %{endpoint: endpoint}}
  end

  @impl true
  def handle_cast({:tell, notification}, server) do
    Iroh.tell(server.endpoint, notification)
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
