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
  alias OneCore.Plugins.Store

  @builds "network/builds/"
  @latest @builds <> "latest"
  @plugin ~r{^((?:network|(?:users|groups)/[^/]+)/plugins/[^/]+)/latest$}

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
  Whether the builds `a` and `b` talk: parts of the platform (servers, the edge, clients) talk only on the same
  0.minor, or from 1.0 the same major, as Cargo, npm and Hex read a version, where a breaking change takes the next.
  """
  def talk?(a, b), do: series(version(a)) == series(version(b))

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
  What this server says of itself, as it starts and as an update of it moves on: its build, its kind of machine,
  the device it's on, if any, whether it was
  installed with `one serve on`, which is what takes the latest builds when it's asked to update, where an update
  of it is, if one is under way (`OneCore.Release`), and, if the last one didn't take, why, as that program said it.
  """
  def say(key) do
    info = %{
      build: build(),
      target: target(),
      device: Application.get_env(:one_core, :device),
      installed: Application.get_env(:one_core, :installed, false),
      update: update(),
      went_back: Application.get_env(:one_core, :went_back)
    }

    Servers.say("servers/#{key}/build", JSON.encode!(info))
  end

  @doc """
  Where an update of this server is: `"downloading"`, `"restarting"` or `"starting"`, or `nil` with none under way
  (`OneCore.Release`).
  """
  def update, do: Application.get_env(:one_core, :update)

  @doc """
  Keeps where an update of this server is, as the program keeping it running says it (`OneCore.Release`):
  `"update"`, its stage, and `"went_back"`, why the last one didn't take, where `said` has them. It's said in
  `builds`, and the clients watching are told. A server that's restarting says so at the network's edge first, for
  as long as it may take to be back, `"within"` seconds, as it's about to stop.
  """
  def updating(said) do
    for {name, setting} <- [{"update", :update}, {"went_back", :went_back}], Map.has_key?(said, name),
        do: Application.put_env(:one_core, setting, said[name])

    if update() == "restarting", do: Servers.restarting(said["within"])
    say(Server.key())
    told = fn waiting -> for {process, _} <- waiting, do: send(process, {:update, update()}) end
    Registry.dispatch(OneCore.Waiting, :update, told)
    Server.changed()
  end

  # How long the program keeping a server running takes to say it has begun an update, in milliseconds: it reads the
  # server's output as it comes, so this is only for a machine that's very busy.
  @begun 5_000

  @doc """
  Has `ask` ask the program keeping this server running to update it, and waits for that program to say the update's
  stage, `@begun` at most. Gives the stage, or `nil` if it said none: one from before it said stages.
  """
  def await_update(ask) do
    Registry.register(OneCore.Waiting, :update, nil)
    ask.()

    receive do
      {:update, stage} -> stage
    after
      @begun -> update()
    end
  after
    Registry.unregister(OneCore.Waiting, :update)
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
  since each server release deploys the plugins it ships with again, each a new version with its apps and NIFs.
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

  @doc """
  Which build each of `owner`'s servers runs, with what its machine is called, whether it's on and where an update
  of it is (`say/1`), and which each of `by`'s devices last said it is, and the latest for each kind of machine: what
  shows whether everything is on the same build. `bundled` says which of the plugins it ships the server asked is
  still deploying for the network, or failed to, and why (`OneCore.Plugins.Deploy.deploying_bundled/0`).
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
        server = Map.merge(info, %{"server" => key, "name" => Servers.name(key), "seen" => seen(key)})
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
      bundled: OneCore.Plugins.Deploy.deploying_bundled()
    }
  end

  # Why `server` doesn't talk with this one or the edge, for being on a version that doesn't talk with theirs, or
  # `nil` where it does. The edge takes one on a version before its own for restarting until it's updated.
  defp why(%{"build" => theirs, "name" => name}, edge) do
    cond do
      not talk?(theirs, build()) ->
        "It's on #{theirs} and #{Servers.name(Server.key()) || "this server"} on #{build()}, so they don't talk until both are on one"

      (edge && not talk?(theirs, edge)) and older?(theirs, edge) ->
        "It's on #{theirs}, before the edge's #{edge}, so the edge takes #{name || "it"} for restarting until it's updated"

      true ->
        nil
    end
  end

  defp why(_server, _edge), do: nil

  # Whether a server is on, as `"on"`, stopping to start again or starting and not up yet, as `"restarting"`, or
  # when it was last, in milliseconds since 1970, or `nil` if that isn't known.
  defp seen(key) do
    case Servers.seen(key) do
      :on -> "on"
      :restarting -> "restarting"
      at -> at
    end
  end
end
