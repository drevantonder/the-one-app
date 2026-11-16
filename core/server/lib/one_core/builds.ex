defmodule OneCore.Builds do
  @moduledoc """
  Which build each machine runs, and the latest, which a machine takes when it's asked to update.

  A build is named by the commit it was built from (`build/0`), and is made for a kind of machine (`target/0`):
  `server/<target>`, an installed server, `one/<target>`, the program `one`, and `one-cli/<target>`, `one` without its
  window. `tooling/release` makes them and publishes them (`publish/1`): each is a blob under the network's `builds/`, named
  by its SHA-256, and the blob `builds/latest` says which is latest for each name, by its build, its SHA-256 and its
  size. Every server reads them, whoever's it is (`core/edge/presigner/presigner.ts`), and a client downloads one by a link
  a server gives (`download/1`), checking it against what `latest` says before it runs it
  (`clients/cli/src/serving.rs`).

  What each machine runs is under whoever's it is: a server's in its own blobs, `servers/<key>/build`, which it
  writes as it starts, and a device's under its user's prefix, `devices/<key>`, which it says as it renews its
  pass (`seen/3`).
  """
  alias OneCore.{Bucket, Server, Servers}
  alias OneCore.Plugins.Store

  @builds "network/builds/"
  @latest @builds <> "latest"
  @plugin ~r{^((?:network|(?:users|groups)/[^/]+)/plugins/[^/]+)/latest$}

  @doc "The build this server is: the commit it was built from."
  def build, do: Application.fetch_env!(:one_core, :build)

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
  What this server says of itself as it starts: its build, its kind of machine, the device it's on, if any, and
  whether it was installed with `one serve on`, which is what takes the latest builds when it's asked to update,
  and, if the last update didn't take, why, as that program said it.
  """
  def started(key) do
    said = %{
      build: build(),
      target: target(),
      device: Application.get_env(:one_core, :device),
      installed: Application.get_env(:one_core, :installed, false),
      went_back: Application.get_env(:one_core, :went_back)
    }

    Servers.say("servers/#{key}/build", JSON.encode!(said))
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
  def seen(by, device, %{} = said) do
    said = Map.take(said, ~w(build target name))
    blob = "#{by}/devices/#{device}"

    case Bucket.read(blob) do
      {:ok, was, tag} -> if JSON.decode!(was) != said, do: Bucket.replace(blob, JSON.encode!(said), tag)
      :absent -> Bucket.write(blob, JSON.encode!(said))
    end

    :ok
  end

  def seen(_by, _device, _said), do: :ok

  @doc """
  Which build each of `owner`'s servers runs, with what its machine is called and whether it's on, and which each
  of `by`'s devices last said it is, and the latest for each kind of machine: what shows whether everything is on
  the same build. `bundled` says which of the plugins it ships the server asked is still deploying for the
  network, or failed to, and why (`OneCore.Plugins.Deploy.deploying_bundled/0`).
  """
  def list(by, owner) do
    keys = Servers.acting(owner)
    devices = Bucket.list("#{by}/devices/")
    # Each blob is a trip across the internet where the bucket is far away, so they're all read at once.
    [latest | read] = Bucket.read_all([@latest | Enum.map(keys, &"servers/#{&1}/build")] ++ devices)
    {said, devices_said} = Enum.split(read, length(keys))

    servers =
      for {key, read} <- Enum.zip(keys, said) do
        said = with {:ok, said, _tag} <- read, do: JSON.decode!(said), else: (:absent -> %{})
        Map.merge(said, %{"server" => key, "name" => Servers.name(key), "seen" => seen(key)})
      end

    devices =
      for {blob, {:ok, said, _tag}} <- Enum.zip(devices, devices_said),
          do: Map.put(JSON.decode!(said), "device", Path.basename(blob))

    %{
      asked: Server.key(),
      build: build(),
      latest: with({:ok, latest, _tag} <- latest, do: JSON.decode!(latest), else: (:absent -> %{})),
      servers: servers,
      devices: devices,
      bundled: OneCore.Plugins.Deploy.deploying_bundled()
    }
  end

  # Whether a server is on, as `"on"`, or when it was last, in milliseconds since 1970, or `nil` if that isn't known.
  defp seen(key), do: with(:on <- Servers.seen(key), do: "on")
end
