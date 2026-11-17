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
  alias OneCore.{Builds, Edge, Hooks, Iroh, Mcp, Apps, Servers, Trace}
  alias OneCore.Call.State

  def start_link(_), do: GenServer.start_link(__MODULE__, nil, name: __MODULE__)

  @doc """
  Tells every client that's watching that something changed, so that it asks again. It says the UI library's build
  (`library`), and for each of `owners` whose apps moved, the version each of theirs runs at (`apps`): an app's
  screen loaded against another of either is stale, and its client loads it again. With no server started, nobody is
  told. The versions are looked up apart from whoever says it, which may be what keeps them.
  """
  def changed(owners \\ []) do
    Task.Supervisor.start_child(OneCore.Asks, fn ->
      apps =
        Map.new(owners, fn owner ->
          {owner, for({name, version} <- Apps.versions([owner]), into: %{}, do: {name, version.code})}
        end)

      params = %{library: OneCore.Entrypoint.library().code, apps: apps}
      notification = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/changed", params: params})
      GenServer.cast(__MODULE__, {:notify, notification})
    end)

    :ok
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
  `:unreachable` if it couldn't be reached, isn't ready, or is on a version before this
  server's that doesn't talk with it (`OneCore.Builds.talk?/2`), so it was asked nothing; or `:stopped` if it stopped
  answering after it was asked, so it may have done what was asked. One on a newer version that doesn't talk with this
  server refuses, saying this one is behind.

  Every request says this server's build, as `_meta.version`, and one of a version that doesn't talk with the server
  asked is refused before anything of it is read (`OneCore.Mcp`). So while a release with a breaking change puts it on
  each server in turn, a server asks only those that talk with it: one behind is skipped, as if it were off, and one
  ahead refuses what this one asks.

  Only a server that's ready (`OneCore.Servers.presence/1`), or isn't known not to be, is asked. With `ready: :any`
  one that isn't is asked too, unless it's stopped, for what only it can answer: a server that's starting answers
  once its apps have, and is ready only once it has answered by its key.
  """
  def request(server, method, params, opts \\ []) do
    # A call that's passed on is one trace on both servers. It asks for MCP's progress notifications, which are how
    # this one knows the other is still running it, however long it takes (`client::Reached::ask`).
    # Its id is its own, so cancelling the call it's for tells that server which of its calls to cancel.
    id = System.unique_integer([:positive])
    meta = %{progressToken: id, version: Builds.build()}
    meta = if parent = Trace.traceparent(), do: Map.put(meta, :traceparent, parent), else: meta
    params = Map.update(params, :_meta, meta, &Map.merge(&1, meta))
    message = JSON.encode!(%{jsonrpc: "2.0", id: id, method: method, params: params})

    asked? = fn presence ->
      presence["server_ready"] != false or (opts[:ready] == :any and presence["server_daemon_status"] != "stopped")
    end

    with true <- asked?.(Servers.presence(server)) or asked?.(Servers.presence_again(server)) or :unreachable,
         {:ok, response} <- passed(method, server, id, fn -> ask(server, message, State.answering()) end) do
      case JSON.decode(response) do
        {:ok, %{"result" => result}} -> {:ok, result}
        {:ok, %{"error" => %{"code" => -32022, "data" => %{"version" => theirs}}}} -> refused(server, theirs)
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

  # A server on a version that doesn't talk with this one was asked nothing. One that's behind is updated in its
  # turn, which a release does next: it's as if it were off, so whoever asked asks another. One that's ahead was
  # updated first, and this one is the one behind.
  defp refused(server, theirs) do
    Servers.build_again(server)

    why =
      "#{named(server)} is on #{theirs} and #{named(key())} on #{Builds.build()}, so they don't talk until both are on one"

    Logger.warning(why)

    if Builds.older?(theirs, Builds.build()),
      do: :unreachable,
      else: {:error, -32022, "#{why}: #{named(key())} is the one behind, and is asked nothing until it's updated"}
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
    presence = Servers.presence(server) || %{}

    cond do
      not Builds.talk?(Servers.build(server), Builds.build()) ->
        "#{named(server)} is on #{Servers.build(server)} and #{named(key())} on #{Builds.build()}, so it's asked nothing until it's updated"

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
  Tells the other servers acting for `owners` that there's something new of theirs in the bucket to `look` for: a
  app's version, a server, a service installed. Or that what they kept of `hooks` was `decided` from what has
  since changed. Each has looked or forgotten, or couldn't be reached, when this returns.
  """
  def notify_servers(owners, what \\ "reload", hooks \\ []),
    do: notify(Enum.uniq(Enum.flat_map(owners, &Servers.acting/1)), owners, what, hooks)

  @doc "Tells `server` to cancel the call it's running for what this server asked it as `request` (`request/3`)."
  def cancel(server, request) do
    notification = %{jsonrpc: "2.0", method: "notifications/cancelled", params: %{requestId: request}}
    Task.Supervisor.start_child(OneCore.Asks, fn -> ask(server, JSON.encode!(notification)) end)
    :ok
  end

  @doc "Tells `servers`, by their keys, what `notify_servers/3` tells the servers acting for `owners`."
  def notify(servers, owners, what \\ "reload", hooks \\ []) do
    params = %{owners: owners, hooks: hooks}
    notification = JSON.encode!(%{jsonrpc: "2.0", method: "notifications/one/" <> what, params: params})
    Stream.run(Task.async_stream(servers -- [key()], &ask(&1, notification), timeout: :infinity))
  end

  @doc """
  Looks in the bucket at what other servers put there for `owner`: the servers on their list, what each acts for
  and the services it runs, and what's installed for them, running the latest version of each app. Looking at the
  network invalidates whose each device is too, which the network's apps say. Its clients are told if an app
  changed.

  A server looks at someone when it's told to, when it's asked for them and last looked more than five minutes
  ago (`OneCore.Apps`), and when it comes back (`reconnected/0`). Nothing else has it look, so someone nobody
  uses costs nothing.
  """
  def reload(owner) do
    Servers.reload(owner)
    OneCore.Apps.Deploy.forget()
    if owner == "network", do: Hooks.invalidate(["device"])
    if Apps.sync(owner), do: changed([owner])
    # An app updated by a server elsewhere may require what isn't installed for them yet.
    if Servers.here?(owner), do: Task.start(fn -> OneCore.Apps.Lifecycle.required(owner) end)
    :ok
  end

  @doc """
  This server can reach the others again, its network's address or its bucket, after it couldn't, so it may have
  missed being told something, and the edge what it is: it says its status again (`OneCore.Servers.report/0`),
  invalidates what it cached of servers and hooks, looks at once at who it acts for, and at everyone else it has
  looked at behind the next request for them (`OneCore.Apps.refresh/0`). The program keeping it running is told, to
  take a release it may have missed (`OneCore.Release.reconnected/0`).
  """
  def reconnected do
    OneCore.Release.reconnected()
    Servers.report()
    Hooks.invalidate()
    Servers.invalidate()
    Enum.each(Apps.refresh(), &reload/1)
  end

  # A connection is opened when there's none, to where the server says it is now. One whose connection no longer
  # answered may only have lost what it was sent, as a network now and then does, so it's connected to again, which
  # one that's there answers at once (`Iroh.connect/4`), one that started again too.
  defp ask(server, message, relay \\ nil) do
    {_key, endpoint} = :persistent_term.get(__MODULE__)
    request = fn -> await(&Iroh.ask_server(endpoint, server, message, self(), &1, relay)) end

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
    OneCore.Release.holds(Iroh)
    key = JSON.decode!(address)["id"]
    :persistent_term.put(__MODULE__, {key, endpoint})
    {:ok, machine} = :inet.gethostname()
    name = Application.get_env(:one_core, :name) || to_string(machine)
    Servers.started(key, address, name, Application.get_env(:one_core, :owner, "network"))
    # The clients that were watching before it stopped may have missed something meanwhile, a new build of its
    # apps among them, so they're told to ask again.
    changed()
    {:ok, %{endpoint: endpoint}}
  end

  @impl true
  def handle_cast({:notify, notification}, server) do
    Iroh.notify(server.endpoint, notification)
    {:noreply, server}
  end

  # A server is reached as soon as it starts, and answers what it's asked once its apps are there, since they
  # say whose a device is: so whoever asks while they start waits for them, and isn't told it can't be reached.
  @impl true
  def handle_info({:ask, ask, from, message}, server) do
    reply = fn ->
      Apps.started()
      Iroh.reply(ask, Mcp.handle(message, from, ask))
    end

    Task.Supervisor.start_child(OneCore.Asks, reply)
    {:noreply, server}
  end
end
