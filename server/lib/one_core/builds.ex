defmodule OneCore.Builds do
  @moduledoc """
  Which build each machine runs, and the newest, which a machine takes when it's asked to update.

  A build is named by the commit it was built from (`build/0`), and is made for a kind of machine (`target/0`):
  `server/<target>`, an installed server, and `one/<target>`, the program `one`. `./release` makes them and
  publishes them (`publish/1`): each is a blob under the network's `builds/`, named by its SHA-256, and the blob
  `builds/newest` says which is newest for each name, by its build, its SHA-256 and its size. Every server reads
  them, whoever's it is (`address/signer/signer.js`), and a client downloads one by a link a server gives
  (`download/1`), checking it against what `newest` says before it runs it (`client/src/serving.rs`).

  What each machine runs is under whoever's it is: a server's in its own blobs, `servers/<key>/build`, which it
  writes as it starts, and a device's under its user's prefix, `devices/<key>`, which it says as it renews its
  pass (`seen/3`).
  """
  alias OneCore.{Bucket, Server, Servers}

  @builds "network/builds/"
  @newest @builds <> "newest"

  @doc "The build this server is: the commit it was built from."
  def build, do: Application.fetch_env!(:one_core, :build)

  @doc "The kind of machine this is, as builds are named, the same as a client names it: `linux-x86_64`, `macos-aarch64`."
  def target do
    [arch | _] = String.split(to_string(:erlang.system_info(:system_architecture)), "-")

    case :os.type() do
      {:unix, :darwin} -> "macos-" <> arch
      {:unix, os} -> "#{os}-#{arch}"
      {:win32, _} -> "windows-" <> arch
    end
  end

  @doc """
  What this server says of itself as it starts: its build, its kind of machine, the device it's on, if any, and
  whether it was installed with `one serve on`, which is what takes the newest builds when it's asked to update.
  """
  def started(key) do
    said = %{
      build: build(),
      target: target(),
      device: Application.get_env(:one_core, :device),
      installed: Application.get_env(:one_core, :installed, false)
    }

    Servers.say("servers/#{key}/build", JSON.encode!(said))
  end

  @doc "The newest build of each name, as `newest` says: `%{name => %{\"build\", \"sha256\", \"size\"}}`."
  def newest do
    case Bucket.read(@newest) do
      {:ok, newest, _tag} -> JSON.decode!(newest)
      :absent -> %{}
    end
  end

  @doc """
  The newest builds for a kind of machine, each with a link to download it by: `%{\"one\" => ..., \"server\" => ...}`,
  with whichever there's one of.
  """
  def download(target) do
    for {name, build} <- newest(), [what, ^target] <- [String.split(name, "/")], into: %{} do
      {what, Map.put(build, "link", Bucket.link(@builds <> build["sha256"], :read))}
    end
  end

  @doc """
  Makes `builds` the newest of their names: each `%{\"name\", \"build\", \"sha256\", \"size\"}`. While the bucket lacks
  any of their files, gives a link to send each one it lacks by, `{:send, %{name => link}}`, and changes nothing:
  sending them and asking again makes them the newest, `{:ok, newest}`. The files no name is newest for any more
  are removed.
  """
  def publish(builds) do
    missing =
      for %{"name" => name, "sha256" => sha256, "size" => size} <- builds,
          Bucket.size(@builds <> sha256) != {:ok, size},
          into: %{},
          do: {name, Bucket.link(@builds <> sha256, :write)}

    if missing == %{}, do: {:ok, newest(builds)}, else: {:send, missing}
  end

  defp newest(builds) do
    {was, tag} =
      case Bucket.read(@newest) do
        {:ok, newest, tag} -> {JSON.decode!(newest), tag}
        :absent -> {%{}, nil}
      end

    now = Map.merge(was, Map.new(builds, &{&1["name"], Map.take(&1, ~w(build sha256 size))}))

    written =
      if tag, do: Bucket.replace(@newest, JSON.encode!(now), tag), else: Bucket.write(@newest, JSON.encode!(now))

    if written == :ok do
      named = for {_name, build} <- now, do: @builds <> build["sha256"]
      for blob <- Bucket.list(@builds), blob != @newest, blob not in named, do: Bucket.remove(blob)
      now
    else
      newest(builds)
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
  of `by`'s devices last said it is, and the newest for each kind of machine: what shows whether everything is on
  the same build. `shipping` says which of the plugins it ships the server asked is still deploying for the
  network, or failed to, and why (`OneCore.Plugins.Deploy.shipping/0`).
  """
  def list(by, owner) do
    servers =
      for key <- Servers.acting(owner) do
        said =
          case Bucket.read("servers/#{key}/build") do
            {:ok, said, _tag} -> JSON.decode!(said)
            :absent -> %{}
          end

        Map.merge(said, %{"server" => key, "name" => Servers.name(key), "seen" => seen(key)})
      end

    devices =
      for blob <- Bucket.list("#{by}/devices/"), {:ok, said, _tag} <- [Bucket.read(blob)] do
        Map.put(JSON.decode!(said), "device", Path.basename(blob))
      end

    %{
      asked: Server.key(),
      build: build(),
      newest: newest(),
      servers: servers,
      devices: devices,
      shipping: OneCore.Plugins.Deploy.shipping()
    }
  end

  # Whether a server is on, as `"on"`, or when it was last, in milliseconds since 1970, or `nil` if that isn't known.
  defp seen(key), do: with(:on <- Servers.seen(key), do: "on")
end
