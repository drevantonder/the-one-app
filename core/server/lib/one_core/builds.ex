defmodule OneCore.Builds do
  @moduledoc """
  Which build each machine runs, and the latest, which a machine takes when it's asked to update.

  A build is named by the version it is, its commit's (`build/0`), and is made for a kind of machine (`target/0`):
  `server/<target>`, an installed server, `one/<target>`, the program `one`, and `one-cli/<target>`, `one` without its
  window. `tooling/release` makes them and publishes them (`publish/1`): each is a blob under the network's `builds/`, named
  by its SHA-256, and the blob `builds/latest` says which is latest for each name, by its build, its SHA-256 and its
  size. Every server reads them, whoever's it is (`core/edge/presigner/presigner.ts`), and a client downloads one by a link
  a server gives (`download/1`), checking it against what `latest` says before it runs it
  (`clients/cli/src/serving.rs`).

  What each machine runs is under whoever's it is: a server's in its own blobs, `servers/<key>/build`, which it
  writes as it starts and as an update of it moves on (`say/1`), and a device's under its user's prefix,
  `devices/<key>`, which it says as it renews its pass (`seen/3`).
  """
  alias OneCore.{Bucket, Server, Servers}
  alias OneCore.Plugins.{Installed, Store}

  @builds "network/builds/"
  @latest @builds <> "latest"
  @plugin ~r{^((?:network|(?:users|groups)/[^/]+)/plugins/[^/]+)/latest$}

  # The build this is, as `tooling/version` names the commit it's built from: compiled in, and again whenever the
  # commit changes, as git's HEAD or the branch it's on says, as a client's is (`core/client/build.rs`). Its `.dirty`
  # is what was uncommitted when the commit last changed.
  @root Path.expand("../../../..", __DIR__)
  git = fn args -> String.trim(elem(System.cmd("git", args, cd: @root, stderr_to_stdout: true), 0)) end
  branch = git.(["symbolic-ref", "-q", "HEAD"])

  for watched <- ["HEAD", "packed-refs", branch], watched != "",
      do: @external_resource(git.(["rev-parse", "--path-format=absolute", "--git-path", watched]))

  @build String.trim(elem(System.cmd("sh", [Path.join(@root, "tooling/version")]), 0))

  @doc "The build this server is, as `tooling/version` names its commit: `0.15.2+2974.febd37350abc`."
  def build, do: @build

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
  Whether the builds `a` and `b` talk: parts of the platform (servers, the edge, clients) talk only on the same
  0.minor, or from 1.0 the same major, as Cargo, npm and Hex read a version, where a breaking change takes the next.
  """
  def talk?(a, b), do: series(version(a)) == series(version(b))

  defp series(%Version{major: 0, minor: minor}), do: {0, minor}
  defp series(%Version{major: major}), do: major

  @doc "The versions the build `build` talks with, as a person reads them: `0.16`, or from 1.0 `1`."
  def series_name(build) do
    case series(version(build)) do
      {0, minor} -> "0.#{minor}"
      major -> "#{major}"
    end
  end

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
  What this server says of itself, as it starts and as what it is changes: its build, its kind of machine, the device
  it's on, if any, whether it was installed with `one serve on`, which is what takes the latest builds when it's
  asked to update, and what the program keeping it running says of it (`OneCore.Release.status/0`).
  """
  def say(key) do
    info = %{
      build: build(),
      target: target(),
      device: Application.get_env(:one_core, :device),
      installed: Application.get_env(:one_core, :installed, false)
    }

    Servers.say("servers/#{key}/build", JSON.encode!(Map.merge(info, OneCore.Release.status())))
  end

  @doc "The latest build of each name, as `latest` says: `%{name => %{\"build\", \"sha256\", \"size\"}}`."
  def latest do
    case Bucket.read(@latest) do
      {:ok, latest, _tag} -> JSON.decode!(latest)
      :absent -> %{}
    end
  end

  @doc """
  The latest builds for a kind of machine, each with a link to download it by: `%{\"one\" => ..., \"server\" => ...}`,
  with whichever there's one of.
  """
  def download(target) do
    for {name, build} <- latest(), [what, ^target] <- [String.split(name, "/")], into: %{} do
      {what, Map.put(build, "link", Bucket.link(@builds <> build["sha256"], :read))}
    end
  end

  @doc """
  Makes `builds` the latest of their names: each `%{\"name\", \"build\", \"sha256\", \"size\"}`. While the bucket lacks
  any of their files, gives a link to send each one it lacks by, `{:send, %{name => link}}`, and changes nothing:
  sending them and asking again makes them the latest, `{:ok, latest}`. The files no name is latest for any more
  are removed: a machine downloads only the latest, and one whose update doesn't take goes back to the build it
  kept of its own. So are the files of every plugin's versions before its last few (`OneCore.Plugins.Store.prune/1`),
  since each release deploys the plugins that changed again, each a new version with its apps and NIFs.
  """
  def publish(builds) do
    missing =
      for %{"name" => name, "sha256" => sha256, "size" => size} <- builds,
          Bucket.size(@builds <> sha256) != {:ok, size},
          into: %{},
          do: {name, Bucket.link(@builds <> sha256, :write)}

    if missing == %{}, do: {:ok, latest(builds)}, else: {:send, missing}
  end

  defp latest(builds) do
    {was, tag} =
      case Bucket.read(@latest) do
        {:ok, latest, tag} -> {JSON.decode!(latest), tag}
        :absent -> {%{}, nil}
      end

    now = Map.merge(was, Map.new(builds, &{&1["name"], Map.take(&1, ~w(build sha256 size))}))

    written =
      if tag, do: Bucket.replace(@latest, JSON.encode!(now), tag), else: Bucket.write(@latest, JSON.encode!(now))

    if written == :ok do
      named = for {_name, build} <- now, do: @builds <> build["sha256"]
      for blob <- Bucket.list(@builds), blob != @latest, blob not in named, do: Bucket.remove(blob)
      # Every plugin in the bucket, whether or not anyone still has it: each one's `latest`.
      plugins = for blob <- Bucket.list(""), [artifact] <- [Regex.run(@plugin, blob, capture: :all_but_first)], do: artifact
      Stream.run(Task.async_stream(plugins, &Store.prune/1, max_concurrency: 16, timeout: :infinity))
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

  # What the edge says of a server: whether it's ready, why not, and when it was last seen if it isn't connected.
  @presence ~w(server_membership server_daemon_status server_ready server_ready_reason last_seen)

  @doc """
  Which build each of `owner`'s servers runs, with what its machine is called and what it is (`say/1`), as it says
  it and, where the network's edge says it now, whether it's ready (`OneCore.Servers.presence/1`), and which each
  of `by`'s devices last said it is, and the latest for each kind of machine: what shows whether everything is on
  the same build; and `plugins`, whether what runs on the server that answers is what's installed, `synced`, or it's
  `syncing` (`OneCore.Plugins.sync/2`).
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
      edge: edge,
      latest: with({:ok, latest, _tag} <- latest, do: JSON.decode!(latest), else: (:absent -> %{})),
      servers: servers,
      devices: devices,
      plugins: OneCore.Plugins.synced(),
      waiting: waiting(owner)
    }
  end

  # The plugins installed for `owner` whose version doesn't run on this server's platform, and why
  # (`OneCore.Plugins.Version.platform/1`). Only one not running here is read from what it said of itself.
  defp waiting(owner) do
    running = OneCore.Plugins.running(owner)

    for {name, %{"plugin" => plugin}} <- Installed.read(owner),
        is_binary(plugin) and not is_map_key(running, name),
        %{state: state, why: why} when state != :runs <- [Store.platform(name, plugin)],
        do: %{name: name, state: state, why: why}
  end

  # Why `server` doesn't talk with this one or the edge, for being on a version that doesn't talk with theirs, or
  # `nil` where it does. The edge sends one on a version before its own nothing until it's updated.
  defp why(%{"build" => theirs, "name" => name}, edge) do
    cond do
      not talk?(theirs, build()) ->
        "It's on #{theirs} and #{Servers.name(Server.key()) || "this server"} on #{build()}, so they don't talk until both are on one"

      edge != nil and not talk?(theirs, edge) and older?(theirs, edge) ->
        "It's on #{theirs}, before the edge's #{edge}, so the edge sends #{name || "it"} nothing until it's updated"

      true ->
        nil
    end
  end

  defp why(_server, _edge), do: nil
end
