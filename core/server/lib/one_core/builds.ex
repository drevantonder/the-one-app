defmodule OneCore.Builds do
  @moduledoc """
  Which build each machine runs, and the latest, which a machine takes when it's asked to update.

  A build is named by the version it is, its commit's (`build/0`), and is made for a kind of machine (`target/0`):
  `one/<target>`, the program `one`, and `one-cli/<target>`, `one` without its window, and an installed server, in two
  parts, which a machine puts together (`clients/cli/src/serving.rs`):
  `server-runtime/<target>`, the VM, Elixir, the deps, One Core's NIFs and its config, named by the build that last
  changed them and made only then, and `server-code`, One Core's own code and the UI library, the same for every kind
  of machine and made every time. `tooling/release` makes them and publishes them (`publish/1`): each is a blob under the network's `builds/`, named
  by its SHA-256, and the blob `builds/latest` says which is latest for each name, by its build, its SHA-256 and its
  size. Every server reads them, whoever's it is (`core/edge/presigner/presigner.ts`), and a client downloads one by a link
  a server gives (`download/1`), checking it against what `latest` says before it runs it
  (`clients/cli/src/serving.rs`).

  What each machine runs is under whoever's it is: a server's in its own blobs, `servers/<key>/build`, which it
  writes as it starts and as an update of it moves on (`say/1`), and a device's under its user's prefix,
  `devices/<key>`, which it says as it renews its pass (`seen/3`).
  """
  alias OneCore.{Bucket, Server, Servers}
  alias OneCore.Apps.{Installed, Store}

  @builds "network/builds/"
  @latest @builds <> "latest"

  @doc "The build this server is, as `tooling/version` names its commit: `0.15.2+2974.febd37350abc`."
  def build, do: Application.fetch_env!(:one_core, :build)

  @doc """
  The version a build is, Semantic Versioning's. A build from before builds had versions, named `<count>-<commit>`,
  is 0.15.0, whose servers, clients and edge it talked with as they were.
  """
  def version(build) do
    case Version.parse(to_string(build)) do
      {:ok, version} -> version
      :error -> Version.parse!("0.15.0")
    end
  end

  @doc """
  Whether the builds `a` and `b` talk: parts of the platform (servers, the edge, clients) talk across one breaking
  release, a 0.minor or from 1.0 a major, as Cargo, npm and Hex read a version, and refuse two or more, as Kubernetes'
  version skew policy does. So a release reaches each part in turn with nothing split, and a change that would break
  talking across one stays compatible for one release.
  """
  def talk?(a, b), do: apart(a, b) <= 1

  @doc "How many breaking releases apart the builds `a` and `b` are: 0.19 and 0.21 are 2, and 0.21 and 1.0 any number."
  def apart(a, b) do
    case {series(version(a)), series(version(b))} do
      {{0, a}, {0, b}} -> abs(a - b)
      {a, b} when is_integer(a) and is_integer(b) -> abs(a - b)
      _ -> :infinity
    end
  end

  defp series(%Version{major: 0, minor: minor}), do: {0, minor}
  defp series(%Version{major: major}), do: major

  @doc "Whether the build `a` is a version before `b`'s."
  def older?(a, b), do: Version.compare(version(a), version(b)) == :lt

  @doc "The kind of machine this is, as builds are named, the same as a client names it: `linux-x86_64`, `macos-aarch64`."
  def target do
    [arch | _] = String.split(to_string(:erlang.system_info(:system_architecture)), "-")

    case :os.type() do
      {:unix, :darwin} -> "macos-" <> arch
      {:unix, os} -> "#{os}-#{arch}"
      # Windows' VM says only `win32`, and Erlang/OTP's one Windows build is x64's, which an ARM PC runs emulated.
      {:win32, _} -> "windows-x86_64"
    end
  end

  @doc """
  What this server says of itself, as it starts and as what it is changes: its build, the runtime it runs on, its kind
  of machine, the device it's on, if any, whether it was installed with `one serve on`, which is what takes the latest builds when it's
  asked to update, and what the program keeping it running says of it (`OneCore.Release.status/0`).
  """
  def say(key) do
    info = %{
      build: build(),
      target: target(),
      device: Application.get_env(:one_core, :device),
      installed: Application.get_env(:one_core, :installed, false),
      runtime: runtime()
    }

    Servers.say("servers/#{key}/build", JSON.encode!(Map.merge(info, OneCore.Release.status())))
  end

  @doc """
  The runtime this server runs on, `server-runtime/<target>`'s build, which a hot load of its code keeps, or `nil`
  where it was built whole, before there were runtimes.
  """
  def runtime do
    case File.read(Path.join(to_string(:code.root_dir()), "runtime")) do
      {:ok, runtime} -> String.trim(runtime)
      {:error, _} -> nil
    end
  end

  @doc "The latest build of each name, as `latest` says: `%{name => %{\"build\", \"sha256\", \"size\"}}`."
  def latest do
    case Bucket.read(@latest) do
      {:ok, latest, _tag} -> JSON.decode!(latest)
      :absent -> %{}
    end
  end

  @doc """
  The latest builds for a kind of machine, each with a link to download it by: `%{\"one\" => ...,
  \"server-runtime\" => ..., \"server-code\" => ...}`, with whichever there's one of.
  """
  def download(target) do
    for {name, build} <- latest(), what <- what(String.split(name, "/"), target), into: %{} do
      {what, Map.put(build, "link", Bucket.link(@builds <> build["sha256"], :read))}
    end
  end

  # What a build is for a kind of machine: one for that kind, or one for every kind, as `server-code` is.
  defp what([what, target], target), do: [what]
  defp what([what], _target), do: [what]
  defp what(_name, _target), do: []

  @doc """
  Makes `builds` the latest of their names: each `%{\"name\", \"build\", \"sha256\", \"size\"}`, and a runtime's
  `\"inputs\"`, the SHA-256 of what it's built from, which `tooling/release` tells whether it changed by. While the bucket lacks
  any of their files, gives a link to send each one it lacks by, `{:send, %{name => link}}`, and changes nothing:
  sending them and asking again makes them the latest, `{:ok, latest}`. The files no name is latest for any more,
  or was just before, are removed: a machine downloads only the latest, and one whose update doesn't take goes back
  to the build it kept of its own. The one before stays, as a machine may be downloading it as a newer is published,
  as a Mac's `one` is published after Linux's builds of the same release (18 Nov: the Mac's 404 held an update).
  """
  def publish(builds) do
    missing =
      for %{"name" => name, "sha256" => sha256, "size" => size} <- builds,
          Bucket.size(@builds <> sha256) != {:ok, size},
          into: %{},
          do: {name, Bucket.link(@builds <> sha256, :write)}

    if missing == %{}, do: published(latest(builds)), else: {:send, missing}
  end

  # The network's updater is told the latest of what servers take, the server build and each `one`, and takes the
  # network's servers to it (`core/edge/updater/updater.ts`): `published`, the newest of them, and the `latest` of
  # each name, so a new `one` alone reaches every server it runs.
  defp published(latest) do
    taken = for {name, %{"build" => build}} <- latest, taken?(String.split(name, "/")), into: %{}, do: {name, build}
    newest = taken |> Map.values() |> Enum.max_by(&count/1, fn -> nil end)

    case newest && Servers.network(%{published: newest, latest: taken}) do
      {:error, why} ->
        raise "The builds are published, and the network's updater wasn't told: #{why}. Publish them again."

      _told ->
        {:ok, latest}
    end
  end

  defp taken?(["server-code"]), do: true
  defp taken?([program, _target]), do: program in ["one", "one-cli"]
  defp taken?(_name), do: false

  @doc """
  The count of commits in a build's metadata (`tooling/version`), which only grows on main: `0.22.4+3100.abcdef` is
  3100, and one from before builds had versions 0.
  """
  def count(build) do
    with %Version{build: metadata} when is_binary(metadata) <- version(build),
         {count, _commit} <- Integer.parse(metadata),
         do: count,
         else: (_ -> 0)
  end

  defp latest(builds) do
    {was, tag} =
      case Bucket.read(@latest) do
        {:ok, latest, tag} -> {JSON.decode!(latest), tag}
        :absent -> {%{}, nil}
      end

    now = Map.merge(was, Map.new(builds, &{&1["name"], Map.take(&1, ~w(build sha256 size inputs))}))

    written =
      if tag, do: Bucket.replace(@latest, JSON.encode!(now), tag), else: Bucket.write(@latest, JSON.encode!(now))

    if written == :ok do
      named = for {_name, build} <- Map.to_list(now) ++ Map.to_list(was), do: @builds <> build["sha256"]
      for blob <- Bucket.list(@builds), blob != @latest, blob not in named, do: Bucket.remove(blob)
      now
    else
      latest(builds)
    end
  end

  @doc "Keeps which build the device whose key is `device`, `by`'s, says it is, when that's changed."
  def seen(by, device, %{} = info) do
    info = Map.take(info, ~w(build target name))
    blob = "#{by}/devices/#{device}"

    case Bucket.read(blob) do
      {:ok, was, tag} -> if JSON.decode!(was) != info, do: Bucket.replace(blob, JSON.encode!(info), tag)
      :absent -> Bucket.write(blob, JSON.encode!(info))
    end

    :ok
  end

  def seen(_by, _device, _said), do: :ok

  @doc """
  What each of `by`'s `devices` calls itself, as it said when it last connected (`seen/3`): `%{device => name}`, with
  none for a device that hasn't said. It's what its person has called it since, where the name it signed in under
  isn't.
  """
  def names(by, devices) do
    reads = Bucket.read_all(Enum.map(devices, &"#{by}/devices/#{&1}"))

    for {device, {:ok, info, _tag}} <- Enum.zip(devices, reads),
        name = JSON.decode!(info)["name"],
        is_binary(name) and name != "",
        into: %{},
        do: {device, name}
  end

  # What the edge says of a server: whether it's ready, why not, and when it was last seen if it isn't connected.
  @presence ~w(server_membership server_daemon_status server_ready server_ready_reason last_seen)

  @doc """
  Which build each of `owner`'s servers runs, with what its machine is called and what it is (`say/1`), as it says
  it and, where the network's edge says it now, whether it's ready (`OneCore.Servers.presence/1`), and which each
  of `by`'s devices last said it is, and the latest for each kind of machine: what shows whether everything is on
  the same build; and `apps`, whether what runs on the server that answers is what's installed, `synced`, or it's
  `syncing` (`OneCore.Apps.sync/2`).
  """
  def list(by, owner) do
    keys = Servers.acting(owner)
    devices = Bucket.list("#{by}/devices/")
    # Each blob is a trip across the internet where the bucket is far away, so they're all read at once.
    [latest | read] = Bucket.read_all([@latest | Enum.map(keys, &"servers/#{&1}/build")] ++ devices)
    {server_reads, device_reads} = Enum.split(read, length(keys))

    edge = Servers.edge()

    servers =
      for {key, read} <- Enum.zip(keys, server_reads) do
        info = with {:ok, info, _tag} <- read, do: JSON.decode!(info), else: (:absent -> %{})
        presence = Map.take(Servers.presence(key) || %{}, @presence)
        server = Map.merge(info, Map.merge(presence, %{"server" => key, "name" => Servers.name(key)}))
        if why = why(server, edge), do: Map.put(server, "why", why), else: server
      end

    devices =
      for {blob, {:ok, info, _tag}} <- Enum.zip(devices, device_reads),
          do: Map.put(JSON.decode!(info), "device", Path.basename(blob))

    %{
      server: Server.key(),
      build: build(),
      network: network(servers),
      edge: edge,
      latest: with({:ok, latest, _tag} <- latest, do: JSON.decode!(latest), else: (:absent -> %{})),
      servers: servers,
      devices: devices,
      apps: OneCore.Apps.synced(),
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
            not OneCore.Release.on?(OneCore.Release.target(network, server), server),
            do: server["name"]

      Map.put(network, "behind", behind)
    else
      _ -> nil
    end
  end

  # The apps installed for `owner` whose version doesn't run on this server's platform, and why
  # (`OneCore.Apps.Version.platform/1`). Only one not running here is read from what it said of itself.
  defp waiting(owner) do
    running = OneCore.Apps.running(owner)

    for {name, %{"app" => app}} <- Installed.read(owner),
        is_binary(app) and not is_map_key(running, name),
        %{state: state, why: why} when state != :runs <- [Store.platform(name, app)],
        do: %{name: name, state: state, why: why}
  end

  # Why `server` doesn't talk with this one or the edge, for being two or more breaking releases from theirs, or
  # `nil` where it does. The edge sends one that far behind its own nothing until it's updated.
  defp why(%{"build" => theirs, "name" => name}, edge) do
    cond do
      not talk?(theirs, build()) ->
        "It's on #{theirs} and #{Servers.name(Server.key()) || "this server"} on #{build()}, more than one breaking release apart, so they don't talk until one is updated"

      edge != nil and not talk?(theirs, edge) and older?(theirs, edge) ->
        "It's on #{theirs}, more than one breaking release before the edge's #{edge}, so the edge sends #{name || "it"} nothing until it's updated"

      true ->
        nil
    end
  end

  defp why(_server, _edge), do: nil
end
