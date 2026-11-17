defmodule Containers.Podman do
  @moduledoc """
  Runs containers on its machine with Podman, through its own CLI, as the user this server runs as: rootless where
  that's how Podman is set up. Each container The One App made carries labels saying whose it is (`one.artifact`),
  what it's called (`one.name`), what it's for (`one.description`), its size, its restart policy and who asked for it, and each image
  made with `commit` its own `one.artifact` and `one.name`. When a container was last used is a file's time in the
  service's folder, `used/<its name in Podman>`.

  Another network's server on the same machine shares its Podman, so each container and image carries the key of
  the server that made it too (`one.server`), as Compose labels a container with its project, and a server lists,
  stops and creates from only its own.
  """
  use GenServer
  @behaviour OneCore.Service
  alias Containers.Sealed

  # The most of each of what a command prints that's given back: the end of it, where errors usually are.
  @max_bytes 64 * 1024
  # The most of a file, a screenshot or a response's body that's given back.
  @largest 16 * 1024 * 1024
  @sizes %{"small" => {"1", "1g"}, "medium" => {"2", "4g"}, "large" => {"4", "8g"}}

  @impl OneCore.Service
  def called, do: "Containers' Podman"

  @impl OneCore.Service
  def needs do
    "A Linux machine with Podman (rootless is best), which pulls images and runs containers as the user this " <>
      "server runs as, for everyone this server acts for. Containers get no host files, no host network and no " <>
      "privileges unless someone asks for a folder of this machine by name. A container that starts again after a " <>
      "reboot (restart: always) needs Podman's podman-restart user service enabled and the user lingering, which " <>
      "creating one enables: systemctl --user enable podman-restart.service and loginctl enable-linger."
  end

  @impl GenServer
  def init(_) do
    System.find_executable("podman") || raise "This machine has no podman to run containers with"
    send(self(), :build)
    {:ok, nil}
  end

  # Builds the new week's desktop image once the week turns, and once its files change, on a machine that has one,
  # so `create` uses the newest there is and never waits for it. Checked as the service starts and then each hour.
  @impl GenServer
  def handle_info(:build, state) do
    service = self()
    Task.start(fn -> if image(service, "desktop"), do: build(service) end)
    Process.send_after(service, :build, 3_600_000)
    {:noreply, state}
  end

  @doc """
  Every container The One App made on this machine, with its labels, state, ports and when it was last used, and for
  one that exited, its exit code and when it did, as Podman says them.
  """
  def list(service) do
    used = used(service)

    for c <- podman_json!(service, ["ps", "--all", "--filter", "label=one.artifact", "--filter", "label=#{made_here()}", "--format", "json"]) do
      [podman_name | _] = c["Names"]
      labels = c["Labels"] || %{}

      %{
        id: podman_name,
        name: labels["one.name"],
        artifact: labels["one.artifact"],
        description: labels["one.description"],
        size: labels["one.size"],
        restart: labels["one.restart"] || "no",
        by: labels["one.by"],
        image: c["Image"],
        state: c["State"],
        created: c["Created"] * 1000,
        last_used: Map.get(used, podman_name, c["Created"] * 1000),
        ports: for(p <- c["Ports"] || [], do: %{port: p["container_port"], host_port: p["host_port"]})
      }
      |> Map.merge(if c["State"] == "exited", do: %{exit_code: c["ExitCode"], finished: c["ExitedAt"] * 1000}, else: %{})
    end
  end

  @doc """
  The images on this machine: what `commit` made, with whose each is and whether this server made it (`ours`), and
  the rest as Podman names them.
  """
  def images(service) do
    for i <- podman_json!(service, ["images", "--format", "json"]) do
      labels = i["Labels"] || %{}

      %{
        id: i["Id"],
        name: labels["one.name"] || List.first(i["Names"] || []) || i["Id"],
        artifact: labels["one.artifact"],
        ours: labels["one.server"] == OneCore.Service.server(),
        tags: i["Names"] || [],
        size: i["Size"],
        created: i["Created"] * 1000
      }
    end
  end

  @doc """
  The image to create from, if it's on this machine: for `desktop`, the newest built here; or `image` as it's named.
  `nil` where there's none, for `build/1` or `pull/2` to bring.
  """
  def image(service, "desktop"), do: List.first(desktops(service))

  def image(service, image), do: if(run(service, ["image", "exists", image], 10).exit == 0, do: image)

  def pull(service, image), do: podman!(service, ["pull", "--quiet", image], 900) && image

  @doc """
  Builds the app's own image, the desktop, from its Containerfile, for the week and the files it's built from,
  unless it's here, and gives its name. It's built again from scratch each week for Google Chrome's security fixes,
  which it downloads as it's built. It removes the ones this server built before it, but those a container still
  uses, which Podman refuses to remove: a later build does.
  """
  def build(service) do
    folder = OneCore.Service.folder(service, "server/desktop")
    {year, week} = :calendar.iso_week_number()
    image = "localhost/containers-desktop:#{Path.basename(folder)}-#{year}-#{week}"

    if run(service, ["image", "exists", image], 10).exit != 0 do
      podman!(service, ["build", "--pull=newer", "--no-cache", "--label", made_here(), "-t", image, folder], 900)
      for before <- desktops(service), before != image, do: run(service, ["rmi", before], 60)
    end

    image
  end

  @doc """
  Creates a container from an image on this machine and starts it. `spec` has its `id` and labels, the `image`,
  `size`, its `restart` policy, `ports` to publish on this machine's loopback, a `command`, `env` and `mounts`.

  Podman starts a container again by its policy as its command ends; after a reboot, only Podman's `podman-restart`
  user service starts one, those whose policy is `always`, so creating one enables that service first. Stopping one
  by hand keeps it stopped until the next reboot.

  Podman stops a container by sending its command SIGTERM and killing what's left once its stop timeout passes, which
  is long enough here for a desktop's Chrome to write its profile down and quit.
  """
  def create(service, spec) do
    {cpus, memory} = Map.fetch!(@sizes, spec["size"])
    if spec["restart"] == "always", do: start_after_reboot!()

    labels =
      for {key, value} <- spec["labels"], value != nil, do: ["--label", "one.#{key}=#{value}"]

    labels = [["--label", made_here()] | labels]

    args =
      ["create", "--name", spec["id"], "--init", "--tty", "--interactive", "--cpus", cpus, "--memory", memory] ++
        ["--pids-limit", "4096", "--shm-size", "1g", "--stop-timeout", "30", "--restart", spec["restart"]] ++
        List.flatten(labels) ++
        Enum.flat_map(spec["ports"] || [], &["--publish", "127.0.0.1::#{&1}"]) ++
        Enum.flat_map(spec["env"] || %{}, fn {key, value} -> ["--env", "#{key}=#{value}"] end) ++
        Enum.flat_map(spec["mounts"] || [], &["--volume", mount(&1)]) ++
        [spec["image"] | spec["command"] || []]

    podman!(service, args)
    podman!(service, ["start", spec["id"]])
    touch(service, spec["id"])
    :ok
  end

  def start(service, id) do
    podman!(service, ["start", id])
    touch(service, id)
    :ok
  end

  def stop(service, id), do: podman!(service, ["stop", id]) && :ok

  def remove(service, id) do
    podman!(service, ["rm", "--force", "--time", "5", id])
    File.rm(Path.join([OneCore.Service.folder(service), "used", id]))
    :ok
  end

  @doc "Stops every running container The One App made here that's gone unused for `idle` ms, and gives their names."
  def stop_idle(service, idle) do
    now = System.system_time(:millisecond)

    for c <- list(service), c.state == "running", now - c.last_used > idle do
      stop(service, c.id)
      c.name
    end
  end

  def logs(service, id, tail), do: run(service, ["logs", "--tail", "#{tail}", id], 30)

  @doc "Runs `command` with `sh -c` in the container, in `dir`, for at most `timeout` seconds."
  def exec(service, id, command, dir, timeout) do
    touch(service, id)
    run(service, ["exec"] ++ if(dir, do: ["--workdir", dir], else: []) ++ [id, "sh", "-c", command], timeout)
  end

  @doc "A file of the container, as base64, by `podman cp`, which reads a stopped container's too."
  def read_file(service, id, path) do
    touch(service, id)

    within(service, fn folder ->
      podman!(service, ["cp", "#{id}:#{path}", folder])
      file = Path.join(folder, Path.basename(path))
      if File.dir?(file), do: raise("#{path} is a folder.\n\n`exec` with ls lists it.")
      if File.stat!(file).size > @largest, do: raise("#{path} is larger than 16 MB.\n\n`exec` reads part of it.")
      Base.encode64(File.read!(file))
    end)
  end

  @doc "Writes a file into the container, from base64, by `podman cp`. Its folder must be there."
  def write_file(service, id, path, data) do
    touch(service, id)

    within(service, fn folder ->
      file = Path.join(folder, "file")
      File.write!(file, Base.decode64!(data))
      podman!(service, ["cp", file, "#{id}:#{path}"])
      :ok
    end)
  end

  @doc """
  Sends a file of the container to the bucket, sealed with `key` (base64), by `link`, which writes one blob: copied out
  by `podman cp`, which reads a stopped container's too, and sent a chunk at a time (`Containers.Sealed`), so a file
  of any size is sent and none is ever whole in memory. Gives its size in bytes.
  """
  def upload(service, id, path, link, key) do
    touch(service, id)

    within(service, fn folder ->
      podman!(service, ["cp", "#{id}:#{path}", folder], 600)
      file = Path.join(folder, Path.basename(path))
      if File.dir?(file), do: raise("#{path} is a folder.\n\n`exec` with tar packs one into a file.")
      Sealed.put(file, Base.decode64!(key), link)
    end)
  end

  @doc """
  Writes a file into the container from a blob `upload/5` sent, by `link`, which reads it, opened with `key`
  (base64), a chunk at a time. Its folder must be there. Gives its size in bytes.
  """
  def download(service, id, path, link, key) do
    touch(service, id)

    within(service, fn folder ->
      file = Path.join(folder, "file")
      size = Sealed.get(link, Base.decode64!(key), file)
      podman!(service, ["cp", file, "#{id}:#{path}"], 600)
      size
    end)
  end

  @doc "Makes an image of the container as it is now, labelled whose it is and what it's called."
  def commit(service, id, image, labels) do
    changes = Enum.flat_map(labels, fn {key, value} -> ["--change", "LABEL one.#{key}=#{value}"] end)
    changes = ["--change", "LABEL #{made_here()}" | changes]
    podman!(service, ["commit", "--quiet"] ++ changes ++ [id, image])
    :ok
  end

  def remove_image(service, image), do: podman!(service, ["rmi", image]) && :ok

  @doc "A PNG of the X display the container's `DISPLAY` names, as base64, taken by ImageMagick's `import` in it."
  def screenshot(service, id) do
    touch(service, id)

    case run(service, ["exec", id, "import", "-window", "root", "png:-"], 30, :raw) do
      %{exit: 0, stdout: png} -> Base.encode64(png)
      %{stderr: why} -> raise "No screenshot: #{String.trim(why)}. It needs a display, as the image desktop has"
    end
  end

  @doc """
  The running containers The One App made here that have a screen: a `DISPLAY` in their environment, as the image
  desktop has. Each as `list/1` gives it, with its display's `width` and `height` as they are now.
  """
  def screens(service) do
    running = for c <- list(service), c.state == "running", do: c
    inspected = if running == [], do: [], else: podman_json!(service, ["container", "inspect" | Enum.map(running, & &1.id)])

    for {c, i} <- Enum.zip(running, inspected),
        Enum.any?(i["Config"]["Env"] || [], &String.starts_with?(&1, "DISPLAY=")),
        %{exit: 0, stdout: size} <- [run(service, ["exec", c.id, "xdotool", "getdisplaygeometry"], 10)],
        [width, height] = String.split(size),
        do: Map.merge(c, %{width: String.to_integer(width), height: String.to_integer(height)})
  end

  @doc """
  Makes the container's display `width` × `height`, as RandR does a monitor's mode: the image desktop's X server,
  Xvnc, takes any size, and its output is `VNC-0`. A mode already made is made again in vain, and that's let be.
  """
  def resize(service, id, width, height) do
    touch(service, id)
    mode = "#{width}x#{height}"

    script =
      "xrandr --newmode #{mode} 60 #{width} #{width} #{width} #{width} #{height} #{height} #{height} #{height} 2>/dev/null; " <>
        "xrandr --addmode VNC-0 #{mode} 2>/dev/null; xrandr --fb #{mode} --output VNC-0 --mode #{mode}"

    podman!(service, ["exec", id, "sh", "-c", script], 30)
    :ok
  end

  @doc """
  Streams the container's display as WebRTC video, to the app whose SDP offer `request` has, as JSON, with the ICE
  servers to use: the image desktop's `webrtc-stream` answers it and carries on by itself until the app goes. Gives
  the SDP answer.
  """
  def stream(service, id, request) do
    touch(service, id)
    file = "/tmp/webrtc-stream-#{System.unique_integer([:positive])}.json"

    within(service, fn folder ->
      File.write!(Path.join(folder, "request"), request)
      podman!(service, ["cp", Path.join(folder, "request"), "#{id}:#{file}"])
    end)

    case run(service, ["exec", id, "webrtc-stream", file], 30) do
      %{exit: 0, stdout: output} -> output
      %{stderr: why} -> raise "Couldn't stream its screen: #{String.trim(why)}"
    end
  end

  @doc "Runs xdotool in the container, on its display, with `args` as its words, and gives what it printed."
  def xdotool(service, id, args) do
    touch(service, id)
    podman!(service, ["exec", id, "xdotool" | args], 30)
  end

  @doc "Makes an HTTP request to a port the container published, from this machine, and gives the response."
  def request(service, id, port, method, path, headers, body) do
    touch(service, id)

    host_port =
      case run(service, ["port", id, "#{port}/tcp"], 10) do
        %{exit: 0, stdout: "127.0.0.1:" <> port} -> String.trim(port)
        _ -> raise "#{port} isn't one of its ports: a container's ports are the ones it was created with"
      end

    url = "http://127.0.0.1:#{host_port}" <> if(String.starts_with?(path, "/"), do: path, else: "/" <> path)

    response =
      Req.request!(
        method: method,
        url: url,
        headers: headers,
        body: body,
        retry: false,
        redirect: false,
        decode_body: false,
        receive_timeout: 60_000
      )

    %{
      status: response.status,
      headers: Map.new(response.headers, fn {key, values} -> {key, Enum.join(values, ", ")} end),
      body: clip(response.body, @largest)
    }
  end

  # Enables Podman's `podman-restart` user service, which starts containers whose policy is `always` as the machine
  # starts, and lingering, so the user's services start with the machine rather than once they sign in.
  defp start_after_reboot! do
    with {_, 0} <- System.cmd("systemctl", ["--user", "enable", "podman-restart.service"], stderr_to_stdout: true),
         {_, 0} <- System.cmd("loginctl", ["enable-linger"], stderr_to_stdout: true) do
      :ok
    else
      {why, _} ->
        raise "This machine can't start a container again after a reboot: #{String.trim(why)}\n\n" <>
                "It needs Podman's podman-restart user service enabled (systemctl --user enable podman-restart.service) " <>
                "and the user lingering (loginctl enable-linger)."
    end
  end

  # The desktop images this server built, newest first.
  defp desktops(service) do
    filters = ["--filter", "reference=localhost/containers-desktop", "--filter", "label=#{made_here()}"]
    String.split(podman!(service, ["images", "--format", "{{.Repository}}:{{.Tag}}" | filters]), "\n", trim: true)
  end

  # The label of what this server made.
  defp made_here, do: "one.server=#{OneCore.Service.server()}"

  defp mount(%{"source" => source, "target" => target} = mount),
    do: "#{source}:#{target}" <> if(mount["read_only"], do: ":ro", else: "")

  defp touch(service, id) do
    folder = Path.join(OneCore.Service.folder(service), "used")
    File.mkdir_p!(folder)
    File.touch!(Path.join(folder, id))
  end

  defp used(service) do
    folder = Path.join(OneCore.Service.folder(service), "used")

    for file <- if(File.dir?(folder), do: File.ls!(folder), else: []), into: %{} do
      {file, File.stat!(Path.join(folder, file), time: :posix).mtime * 1000}
    end
  end

  # Runs `fun` with a folder of its own, which is gone after.
  defp within(service, fun) do
    folder = Path.join(OneCore.Service.folder(service), "cp-#{System.unique_integer([:positive])}")
    File.mkdir_p!(folder)

    try do
      fun.(folder)
    after
      File.rm_rf!(folder)
    end
  end

  defp podman_json!(service, args), do: JSON.decode!(podman!(service, args))

  defp podman!(service, args, timeout \\ 60) do
    case run(service, args, timeout) do
      %{exit: 0, stdout: stdout} -> stdout
      %{stderr: stderr} -> raise String.trim(String.replace(stderr, ~r/^Error: /m, ""))
    end
  end

  # Runs podman with `args` for at most `timeout` seconds, as Shell's runner runs a command: its own process group,
  # stopped whole once it takes too long, with what it writes as errors in a file beside while it runs.
  defp run(service, args, timeout, output \\ :text) do
    errors = Path.join(OneCore.Service.folder(service), "errors-#{System.unique_integer([:positive])}")

    program =
      Port.open({:spawn_executable, System.find_executable("sh")}, [
        :binary,
        :exit_status,
        args: ["-c", ~s(exec </dev/null 2>"$0"; exec podman "$@"), errors | args]
      ])

    {:os_pid, pid} = Port.info(program, :os_pid)
    {exit, stdout} = read(program, pid, System.monotonic_time(:millisecond) + timeout * 1000, [])
    stderr = File.read!(errors)
    File.rm!(errors)
    stderr = if exit == -1, do: stderr <> "\n(stopped after #{timeout}s)", else: stderr

    case output do
      :raw -> %{exit: exit, stdout: stdout, stderr: stderr}
      :text -> %{exit: exit, stdout: clip(stdout, @max_bytes), stderr: clip(stderr, @max_bytes)}
    end
  end

  defp read(program, pid, until, printed) do
    receive do
      {^program, {:data, data}} -> read(program, pid, until, [printed | data])
      {^program, {:exit_status, status}} -> {status, IO.iodata_to_binary(printed)}
    after
      max(0, until - System.monotonic_time(:millisecond)) ->
        System.cmd("kill", ["-KILL", "--", "-#{pid}"])
        Port.close(program)
        {-1, IO.iodata_to_binary(printed)}
    end
  end

  # Keeps the end of long output, and says how much was cut.
  defp clip(text, max) do
    text = String.replace_invalid(text)

    if byte_size(text) <= max do
      text
    else
      end_of = String.replace_invalid(binary_part(text, byte_size(text) - max, max), "")
      "(#{byte_size(text) - byte_size(end_of)} earlier bytes cut) …" <> end_of
    end
  end
end
