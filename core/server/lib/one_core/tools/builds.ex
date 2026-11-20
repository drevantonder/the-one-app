defmodule OneCore.Tools.Builds do
  @moduledoc "One Core's own tools for the platform's builds: which there are, downloading them, and the network's update, as `OneCore.Tools` calls them, with their definitions in `OneCore.Tools.Definitions`."
  import OneCore.Tools.Helpers
  alias OneCore.{Apps, Bucket, Builds, Protocol, Release, Server, Servers, User}
  alias OneCore.Apps.{Installed, Store}

  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  def run("builds", inputs, {actor, subject} = principal, _pass, _request) do
    on(inputs["server"], "builds", inputs, principal, fn ->
      list(actor, User.owner(subject) || "network")
    end)
  end

  def run("download", %{"target" => target}, _principal, _pass, _request), do: Builds.download(target)

  # What takes the latest builds is the program that keeps an installed server running, which hears this said.
  def run("update", _inputs, principal, _pass, _request) do
    reaches!(principal, "network", "owner")
    network = Release.update_now()
    {:content, %{network: network}, [%{type: "text", text: Release.software_update(network)}]}
  end

  # What the edge says of a server: whether it's ready, why not, and when it was last seen if it isn't connected.
  @presence ~w(server_membership server_daemon_status server_ready server_ready_reason last_seen)

  # Which build each of `owner`'s servers runs, with what its machine is called and what it is (`OneCore.Builds.say/1`), as it says
  # it and, where the network's edge says it now, whether it's ready (`OneCore.Servers.presence/1`), and which each
  # of `by`'s devices last said it is, with when it was last seen, and the latest for each kind of machine: what shows
  # whether everything is on the same build; the protocol each part speaks (`OneCore.Protocol.of/1`), which says which
  # talk; and `apps`, whether what runs on the server that answers is what's installed, `synced`, or it's `syncing`
  # (`OneCore.Apps.sync/2`).
  defp list(by, owner) do
    keys = OneCore.Apps.covering(owner)
    devices = Bucket.list("#{by}/devices/")
    # Each blob is a trip across the internet where the bucket is far away, so they're all read at once.
    latest = Task.async(&Builds.latest/0)
    read = Bucket.read_all(Enum.map(keys, &"servers/#{&1}/build") ++ devices)
    {server_reads, device_reads} = Enum.split(read, length(keys))

    edge = Servers.edge(Enum.map(devices, &Path.basename/1)) || %{}

    servers =
      for {key, read} <- Enum.zip(keys, server_reads) do
        info = with {:ok, info, _tag} <- read, do: JSON.decode!(info), else: (:absent -> %{})
        presence = Map.take(Servers.presence(key) || %{}, @presence)
        info = Map.put(info, "protocol", Protocol.of(info["protocol"]))
        server = Map.merge(info, Map.merge(presence, %{"server" => key, "name" => Servers.name(key)}))
        if why = why(server, edge), do: Map.put(server, "why", why), else: server
      end

    devices =
      for {blob, {:ok, info, _tag}} <- Enum.zip(devices, device_reads) do
        info = JSON.decode!(info)
        device = Path.basename(blob)
        seen = Map.take(edge["presence"][device] || %{}, ~w(connected last_seen))

        Map.merge(info, Map.put(seen, "device", device))
        |> Map.put("protocol", Protocol.of(info["protocol"]))
      end

    %{
      server: Server.key(),
      build: Builds.build(),
      protocol: Protocol.range(),
      network: network(servers),
      edge: edge["build"],
      edge_protocol: edge["protocol"],
      latest: Task.await(latest, :infinity),
      servers: servers,
      devices: devices,
      apps: Apps.synced(),
      waiting: waiting(owner)
    }
  end

  # The network's update, as its updater says it, with the servers that take builds and aren't on its build and `one`
  # yet.
  defp network(servers) do
    with {:ok, %{} = network} <- Servers.network() do
      behind =
        for server <- servers,
            server["installed"],
            not Release.on?(Release.target(network, server), server),
            do: server["name"]

      Map.put(network, "behind", behind)
    else
      _ -> nil
    end
  end

  # The apps installed for `owner` whose version doesn't run on this server's platform, and why
  # (`OneCore.Apps.Version.platform/1`). Only one not running here is read from what it said of itself.
  defp waiting(owner) do
    running = Apps.running(owner)

    for {name, %{"app" => app}} <- Installed.read(owner),
        is_binary(app) and not is_map_key(running, name),
        %{state: state, why: why} when state != :runs <- [Store.platform(name, app)],
        do: %{name: name, state: state, why: why}
  end

  # Why `server` doesn't talk with this one or the edge, for speaking none of the protocols theirs does, or `nil`
  # where it does. The edge sends one behind its own nothing until it's updated.
  defp why(%{"protocol" => theirs, "name" => name}, edge) do
    cond do
      not Protocol.speaks?(theirs) ->
        "It speaks protocol #{Protocol.name(theirs)} and #{Servers.name(Server.key()) || "this server"} #{Protocol.name(Protocol.range())}, so they don't talk until one is updated"

      edge["protocol"] != nil and not Protocol.overlap?(theirs, edge["protocol"]) ->
        "It speaks protocol #{Protocol.name(theirs)} and the edge #{Protocol.name(edge["protocol"])}, so the edge sends #{name || "it"} nothing until one is updated"

      true ->
        nil
    end
  end

  defp why(_server, _edge), do: nil
end
