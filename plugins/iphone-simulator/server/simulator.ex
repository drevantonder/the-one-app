defmodule IphoneSimulator.Simulator do
  @moduledoc """
  Runs iOS Simulators on its Mac with Xcode's `simctl`, with no window: Simulator, the app, is never opened. What
  each is (its name, whose it is, what it's for) is a file in this service's folder, `simulators/<udid>.json`; the
  simulator itself is in the Mac's own device set, called `One: <name>`, as Xcode lists it. When it was last used is
  a file's time, `used/<udid>`.

  Its screen and touches are idb's (fbidb.io): `idb_companion`, from idb's own release, pinned here and checked, and
  its CLI, in this service's Python with aiortc, which a stream's sender runs with. Both are installed the first time.

  A simulator is the Mac's own processes, run as the Mac's user: it shares the Mac's network, its 127.0.0.1 among it,
  and what the Mac's user may read. Nothing here can keep an app in it from either.
  """
  use GenServer
  @behaviour OneCore.Service

  @idb "1.7.8"
  @companion_sha256 "44369a2df0b62e3a1764de61bd09280999c7e9b27ea8f20bff1ea3a9db0302a2"
  @aiortc "1.16.0"
  @kept 64 * 1024

  @impl OneCore.Service
  def called, do: "iPhone Simulator's simulators"

  @impl OneCore.Service
  def needs do
    "A Mac with Apple silicon, macOS 15 or later and Xcode 26 or later with an iOS runtime, and uv. It runs iOS " <>
      "Simulators as the Mac's user, with no window, about 2 GB of memory each while they run, and installs idb " <>
      "(Meta's iOS Simulator automation) in its own folder the first time. A simulator shares the Mac's network and " <>
      "files: an app in it reaches the Mac's 127.0.0.1 and reads what the Mac's user can."
  end

  @impl GenServer
  def init(_) do
    match?({:unix, :darwin}, :os.type()) || raise "iOS Simulators run on Macs"
    match?(%{exit: 0}, cmd(["xcrun", "simctl", "help"], 30)) || raise "This Mac has no Xcode's simctl"
    System.find_executable("uv") || raise "This Mac has no uv, which idb and a stream's sender are installed with"
    {:ok, nil}
  end

  @doc "Every simulator The One App made here: its udid, name, whose it is, what it's for, its state and times."
  def list(service) do
    states = for {udid, s} <- simulators(), into: %{}, do: {udid, s["state"]}
    used = used(service)
    folder = folder(service, "simulators")

    for file <- File.ls!(folder), String.ends_with?(file, ".json") do
      s = JSON.decode!(File.read!(Path.join(folder, file)))

      Map.merge(s, %{
        "state" => state(states[s["udid"]]),
        "last_used" => Map.get(used, s["udid"], s["created"])
      })
    end
  end

  @doc """
  Makes a simulator from `spec` (its `name`, `description`, `artifact` and `by`) of the newest iPhone Pro this Xcode
  has, on its newest iOS, and boots it.
  """
  def create(service, spec) do
    %{"runtimes" => runtimes, "devicetypes" => types} = JSON.decode!(simctl!(service, ~w(list --json runtimes devicetypes)))
    runtime = runtimes |> Enum.filter(&(&1["platform"] == "iOS" and &1["isAvailable"])) |> List.last()
    type = types |> Enum.filter(&(&1["name"] =~ ~r/^iPhone \d+ Pro$/)) |> List.last()
    if runtime == nil or type == nil, do: raise("Xcode here has no iOS runtime or iPhone to make one with: Xcode › Settings › Components adds one")

    udid = String.trim(simctl!(service, ["create", "One: #{spec["name"]}", type["identifier"], runtime["identifier"]]))
    s = Map.merge(spec, %{"udid" => udid, "model" => type["name"], "ios" => runtime["version"], "created" => System.system_time(:millisecond)})
    File.write!(Path.join(folder(service, "simulators"), udid <> ".json"), JSON.encode!(s))
    start(service, udid)
  end

  @doc "Boots a simulator and waits until it has."
  def start(service, udid) do
    touch(service, udid)

    case cmd(["xcrun", "simctl", "boot", udid], 120) do
      %{exit: 0} -> :ok
      %{stderr: why} -> if why =~ "current state: Booted", do: :ok, else: raise(String.trim(why))
    end

    simctl!(service, ["bootstatus", udid, "-b"], 600)
    :ok
  end

  def stop(_service, udid) do
    case cmd(["xcrun", "simctl", "shutdown", udid], 120) do
      %{exit: 0} -> :ok
      %{stderr: why} -> if why =~ "current state: Shutdown", do: :ok, else: raise(String.trim(why))
    end
  end

  @doc "Removes a simulator and everything on it, for good."
  def remove(service, udid) do
    stop(service, udid)
    simctl!(service, ["delete", udid])
    File.rm(Path.join(folder(service, "simulators"), udid <> ".json"))
    File.rm(Path.join(folder(service, "used"), udid))
    :ok
  end

  @doc "Shuts down every booted simulator that's gone unused for `idle` ms, and gives their names."
  def stop_idle(service, idle) do
    now = System.system_time(:millisecond)

    for s <- list(service), s["state"] == "running", now - s["last_used"] > idle do
      stop(service, s["udid"])
      s["name"]
    end
  end

  @doc "A PNG of the simulator's screen, as base64, in its own pixels, by `simctl io screenshot`."
  def screenshot(service, udid) do
    touch(service, udid)
    file = Path.join(folder(service, "shots"), "#{System.unique_integer([:positive])}.png")

    try do
      simctl!(service, ["io", udid, "screenshot", "--type=png", file], 30)
      Base.encode64(File.read!(file))
    after
      File.rm(file)
    end
  end

  @doc """
  Plays Computer Use's `action` on the simulator with idb's `ui`, its x and y in the screenshot's pixels, which idb
  takes as points: a click is a tap, a right click a long press, a drag a swipe, a scroll a swipe the other way.
  """
  def act(service, udid, action, inputs) do
    touch(service, udid)
    %{"points" => points} = describe(service, udid) || raise "It isn't running: `start` starts it"
    at = fn x, y -> [fmt(x * points), fmt(y * points)] end

    plays =
      case {action, inputs} do
        {"click", %{"button" => "middle"}} -> [~w(button HOME)]
        {"click", %{"x" => x, "y" => y, "button" => "right"}} -> [["tap" | at.(x, y)] ++ ~w(--duration 1)]
        {"click", %{"x" => x, "y" => y}} -> [["tap" | at.(x, y)]]
        {"double_click", %{"x" => x, "y" => y}} -> [["tap" | at.(x, y)], ["tap" | at.(x, y)]]
        {"drag", %{"path" => [first | rest]}} -> [["swipe" | at.(first["x"], first["y"])] ++ at.(List.last(rest)["x"], List.last(rest)["y"]) ++ ~w(--duration 0.5)]
        {"scroll", %{"x" => x, "y" => y, "direction" => direction} = inputs} -> [scroll(at.(x, y), direction, inputs["amount"] || 5)]
        {"type", %{"text" => text}} -> [["text", text]]
        {"key", %{"keys" => keys}} -> Enum.map(String.split(keys), &key/1)
        _ -> raise "#{action} needs more: Computer Use's tools say what each action takes"
      end

    for args <- plays do
      case cmd(idb(service) ++ ["ui" | args] ++ ["--udid", udid], 60) do
        %{exit: 0} -> :ok
        %{stdout: out, stderr: why} -> raise "idb: #{String.trim(why <> "\n" <> out)}"
      end
    end

    :ok
  end

  # A scroll is a swipe the other way, as a finger scrolls: 80 points for each step, at most 600.
  defp scroll([x, y], direction, amount) do
    far = min(80 * amount, 600)
    {x, y} = {String.to_float(x), String.to_float(y)}
    {dx, dy} = %{"up" => {0, far}, "down" => {0, -far}, "left" => {far, 0}, "right" => {-far, 0}}[direction]
    ["swipe", fmt(x), fmt(y), fmt(max(x + dx, 0)), fmt(max(y + dy, 0))] ++ ~w(--duration 0.3)
  end

  defp fmt(n), do: :erlang.float_to_binary(n * 1.0, decimals: 1)

  # A key or a combination, as xdotool names them (ctrl+a, Return), as idb presses it: the key's USB HID usage,
  # with its modifiers. A Mac's ⌘ is idb's --command.
  @usages %{
    "return" => 40, "enter" => 40, "escape" => 41, "backspace" => 42, "tab" => 43, "space" => 44, "delete" => 76,
    "right" => 79, "left" => 80, "down" => 81, "up" => 82, "home" => 74, "end" => 77, "prior" => 75, "next" => 78,
    "page_up" => 75, "page_down" => 78
  }
  @modifiers %{"ctrl" => "--control", "control" => "--control", "shift" => "--shift", "alt" => "--option", "super" => "--command", "cmd" => "--command"}

  defp key(combination) do
    {mods, [name]} = Enum.split(String.split(String.downcase(combination), "+"), -1)

    usage =
      cond do
        @usages[name] -> @usages[name]
        name =~ ~r/^[a-z]$/ -> 4 + :binary.first(name) - ?a
        name =~ ~r/^[1-9]$/ -> 30 + :binary.first(name) - ?1
        name == "0" -> 39
        true -> raise "idb has no key #{name}"
      end

    ["key" | Enum.map(mods, &(@modifiers[&1] || raise("idb has no modifier #{&1}")))] ++ ["#{usage}"]
  end

  @doc "Opens `url` in the simulator, in the app for it or Safari."
  def open_url(service, udid, url) do
    touch(service, udid)
    simctl!(service, ["openurl", udid, url])
    :ok
  end

  @doc "Installs an app built for the simulator: its .app, zipped, as base64 or by its URL."
  def install(service, udid, zip) do
    touch(service, udid)
    folder = Path.join(folder(service, "install"), "#{System.unique_integer([:positive])}")
    File.mkdir_p!(folder)

    try do
      file = Path.join(folder, "app.zip")

      case zip do
        %{"url" => url} -> Req.get!(url, into: File.stream!(file), retry: false, receive_timeout: 300_000)
        %{"data" => data} -> File.write!(file, Base.decode64!(data))
      end

      # ditto is how a Mac unzips an app, keeping what a bundle needs (its links, its signature's attributes).
      %{exit: 0} = cmd(["ditto", "-x", "-k", file, Path.join(folder, "app")], 120)
      app =
        case Enum.sort_by(Path.wildcard(Path.join(folder, "app/**/*.app")), &String.length/1) do
          [app | _] -> app
          [] -> raise "There's no .app in it: zip the .app built for the simulator"
        end

      simctl!(service, ["install", udid, app], 300)
      "Installed #{Path.basename(app)}"
    after
      File.rm_rf!(folder)
    end
  end

  @doc """
  The booted simulators The One App made here, each as `list/1` gives it, with its screen's `width` and `height` in
  pixels and how many of its points one pixel is (`points`), as idb describes it.
  """
  def screens(service) do
    for s <- list(service), s["state"] == "running", screen = describe(service, s["udid"]), do: Map.merge(s, screen)
  end

  @doc """
  Streams the simulator's screen as WebRTC video to the app whose SDP offer `request` has, as JSON, with the ICE
  servers to use: `server/stream/stream.py` answers it and carries on by itself until the app goes. Gives the SDP
  answer.
  """
  def stream(service, udid, request) do
    touch(service, udid)
    idb = idb(service)
    %{"points" => points} = describe(service, udid) || raise "It isn't running: `start` starts it"
    file = Path.join(OneCore.Service.folder(service), "stream-#{System.unique_integer([:positive])}.json")
    asked = Map.merge(JSON.decode!(request), %{"idb" => idb, "udid" => udid, "points" => points, "log" => Path.join(OneCore.Service.folder(service), "stream.log")})
    File.write!(file, JSON.encode!(asked))

    case cmd([python(service), Path.join(OneCore.Service.folder(service, "server/stream"), "stream.py"), file], 30) do
      %{exit: 0, stdout: answer} -> answer
      %{stderr: why} -> raise "Couldn't stream its screen: #{String.trim(why)}"
    end
  end

  # Its screen as idb describes it: its size in pixels, and its points for each pixel.
  defp describe(service, udid) do
    with %{exit: 0, stdout: said} <- cmd(idb(service) ++ ["describe", "--json", "--udid", udid], 30),
         %{"screen_dimensions" => %{"width" => width, "height" => height, "width_points" => across}} <- JSON.decode!(said) do
      %{"width" => width, "height" => height, "points" => across / width}
    else
      _ -> nil
    end
  end

  # The simulators in the Mac's device set, by udid.
  defp simulators do
    %{exit: 0, stdout: said} = cmd(~w(xcrun simctl list --json devices), 30)
    for {_runtime, devices} <- JSON.decode!(said)["devices"], d <- devices, into: %{}, do: {d["udid"], d}
  end

  defp state("Booted"), do: "running"
  defp state("Booting"), do: "starting"
  defp state("Shutting Down"), do: "stopping"
  defp state(nil), do: "gone"
  defp state(_), do: "stopped"

  # idb's command, with its companion, installed here the first time: the companion from idb's release on GitHub,
  # checked, and idb's CLI in this service's Python.
  defp idb(service) do
    python(service)
    companion = Path.join([OneCore.Service.folder(service), "idb-companion-#{@idb}", "idb_companion"])

    if not File.exists?(companion) do
      url = "https://github.com/facebook/idb/releases/download/v#{@idb}/idb-companion.macos-arm64.tar.gz"
      %{status: 200, body: tarball} = Req.get!(url, decode_body: false, retry: false, receive_timeout: 300_000)

      if Base.encode16(:crypto.hash(:sha256, tarball), case: :lower) != @companion_sha256,
        do: raise("idb's companion from GitHub isn't the one expected")

      unpacking = Path.dirname(companion) <> ".unpacking"
      File.rm_rf!(unpacking)
      File.mkdir_p!(unpacking)
      File.write!(Path.join(unpacking, "companion.tar.gz"), tarball)
      %{exit: 0} = cmd(["tar", "-xzf", Path.join(unpacking, "companion.tar.gz"), "-C", unpacking], 120)
      File.rm!(Path.join(unpacking, "companion.tar.gz"))
      File.rename!(unpacking, Path.dirname(companion))
    end

    [Path.join([OneCore.Service.folder(service), "venv", "bin", "idb"]), "--companion-path", companion]
  end

  # The Python idb's CLI and a stream's sender run with, made with uv the first time.
  defp python(service) do
    venv = Path.join(OneCore.Service.folder(service), "venv")
    made = Path.join(venv, "idb-#{@idb}-aiortc-#{@aiortc}")

    if not File.exists?(made) do
      File.rm_rf!(venv)
      %{exit: 0} = cmd(["uv", "venv", "--quiet", "--python", "3.12", venv], 300)
      %{exit: 0} = cmd(["uv", "pip", "install", "--quiet", "--python", Path.join(venv, "bin/python"), "fb-idb==#{@idb}", "aiortc==#{@aiortc}"], 300)
      File.touch!(made)
    end

    Path.join(venv, "bin/python")
  end

  defp simctl!(_service, args, timeout \\ 60) do
    case cmd(["xcrun", "simctl" | args], timeout) do
      %{exit: 0, stdout: out} -> out
      %{stderr: why, stdout: out} -> raise "simctl #{hd(args)}: #{String.trim(why <> "\n" <> out)}"
    end
  end

  defp folder(service, name) do
    folder = Path.join(OneCore.Service.folder(service), name)
    File.mkdir_p!(folder)
    folder
  end

  defp touch(service, udid), do: File.touch!(Path.join(folder(service, "used"), udid))

  defp used(service) do
    folder = folder(service, "used")
    for file <- File.ls!(folder), into: %{}, do: {file, File.stat!(Path.join(folder, file), time: :posix).mtime * 1000}
  end

  # Runs `args` for at most `timeout` seconds, as Containers' Podman runs podman: its own process group, stopped
  # whole once it takes too long, with what it writes as errors in a file beside while it runs.
  defp cmd([program | args], timeout) do
    errors = Path.join(System.tmp_dir!(), "iphone-simulator-#{System.unique_integer([:positive])}")

    port =
      Port.open({:spawn_executable, System.find_executable("sh")}, [
        :binary,
        :exit_status,
        args: ["-c", ~s(exec </dev/null 2>"$0"; exec "$@"), errors, program | args]
      ])

    {:os_pid, pid} = Port.info(port, :os_pid)
    {exit, stdout} = read(port, pid, System.monotonic_time(:millisecond) + timeout * 1000, [])
    stderr = with {:ok, text} <- File.read(errors), do: text, else: (_ -> "")
    File.rm(errors)
    %{exit: exit, stdout: clip(stdout), stderr: clip(stderr)}
  end

  defp read(port, pid, until, printed) do
    receive do
      {^port, {:data, data}} -> read(port, pid, until, [printed | data])
      {^port, {:exit_status, status}} -> {status, IO.iodata_to_binary(printed)}
    after
      max(0, until - System.monotonic_time(:millisecond)) ->
        System.cmd("kill", ["-KILL", "--", "-#{pid}"])
        Port.close(port)
        {-1, IO.iodata_to_binary(printed)}
    end
  end

  defp clip(text) do
    text = String.replace_invalid(text)
    if byte_size(text) <= @kept, do: text, else: "… " <> String.replace_invalid(binary_part(text, byte_size(text) - @kept, @kept), "")
  end
end
