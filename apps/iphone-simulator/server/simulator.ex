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

  Each simulator is in one state, which changes only as `@steps` says, and every app watching is told as it does:
  `stopped`, `starting` (its `step`: `booting` iOS, then `connecting` idb to its screen), `running`, `stopping`,
  `removing`, and `gone` once Xcode no longer has it. Running, stopped and gone are what simctl says; starting,
  stopping and removing, and why a start failed, are this service's own while a call is under way. A step that
  isn't one, such as Start while it's stopping, is refused saying why.
  """
  use GenServer
  @behaviour OneCore.Service

  # A simulator's states, by the state it's in and what happens, as the moduledoc says. `fail` is a start that
  # couldn't finish, `stopped` a stop that's over, however it went.
  @steps %{
    {"stopped", "start"} => "starting",
    {"starting", "started"} => "running",
    {"starting", "fail"} => "stopped",
    {"starting", "stop"} => "stopping",
    {"running", "stop"} => "stopping",
    {"stopping", "stop"} => "stopping",
    {"stopped", "stop"} => "stopped",
    {"stopping", "started"} => "stopping",
    {"stopping", "fail"} => "stopping",
    {"stopped", "fail"} => "stopped",
    {"stopping", "stopped"} => "stopped",
    {"stopped", "stopped"} => "stopped",
    {"stopped", "remove"} => "removing",
    {"starting", "remove"} => "removing",
    {"running", "remove"} => "removing",
    {"stopping", "remove"} => "removing",
    {"gone", "remove"} => "removing",
    {"removing", "removed"} => "stopped",
    {"removing", "fail"} => "stopped"
  }
  # The states this service keeps, as simctl can't say them.
  @kept ~w(starting stopping removing)

  # Those in a state of `@kept`, or stopped with why a start failed, by udid.
  defstruct simulators: %{}

  @idb "1.7.8"
  @companion_sha256 "44369a2df0b62e3a1764de61bd09280999c7e9b27ea8f20bff1ea3a9db0302a2"
  # idb's CLI isn't on PyPI past 1.7.7: it's the wheel on the same release.
  @cli_sha256 "ed8557a4c54ffee53006c1fe63f070cda567949312ab24be56faf987c5a3d04b"
  @aiortc "1.16.0"
  @max_bytes 64 * 1024

  @impl OneCore.Service
  def called, do: "iPhone Simulator's simulators"

  @impl OneCore.Service
  def os, do: ["darwin"]

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
    {:ok, %__MODULE__{}}
  end

  # Each version's module has a name of its own, so its struct is taken by its fields. A version before this one
  # kept nothing (`nil`), and one started afresh has nothing under way.
  @impl GenServer
  def code_change(_old, %_{} = data, _extra), do: {:ok, struct(__MODULE__, Map.from_struct(data))}
  def code_change(_old, _data, _extra), do: {:error, "kept nothing to carry on with"}

  @impl GenServer
  def handle_call({:step, udid, event, changes}, _from, data) do
    kept = data.simulators[udid]
    was = (kept && kept.state) || state(simulators()[udid]["state"])

    case if(event, do: @steps[{was, event}], else: was) do
      nil ->
        {:reply, {:error, refused(was, event)}, data}

      to ->
        # Why a start failed is kept until the next step, and only for one that was starting.
        why = if event == "fail" and was == "starting", do: changes[:why]
        step = if to == "starting", do: changes[:step] || (kept && kept.step)

        simulators =
          if to in @kept or why,
            do: Map.put(data.simulators, udid, %{state: to, step: step, why: why}),
            else: Map.delete(data.simulators, udid)

        if simulators != data.simulators or to != was, do: tell()
        {:reply, {:ok, to}, %{data | simulators: simulators}}
    end
  end

  def handle_call(:simulators, _from, data), do: {:reply, data.simulators, data}

  defp refused("starting", "start"), do: "It's starting already."
  defp refused("running", "start"), do: "It's running already."
  defp refused("stopping", "start"), do: "It's still stopping. Start it once it's stopped.\n\n`list` says when it is."
  defp refused("removing", _), do: "It's being removed."
  defp refused("gone", _), do: "It's gone from Xcode, deleted outside the One App. `remove` takes it off the list."
  defp refused(state, event), do: "It can't #{event} while it's #{state}."

  # Tells every app watching that a simulator's state changed, as a call to the app does once it's done.
  defp tell do
    service = self()
    for who <- Enum.take(OneCore.Service.acts_for(), 1), do: Task.start(fn -> OneCore.Service.app(service, who, :changed) end)
  end

  # Takes the simulator `udid` one step (`event`, by `@steps`, or `nil` for none) with `changes`, and gives its state.
  defp step(service, udid, event, changes \\ %{}) do
    case GenServer.call(service, {:step, udid, event, changes}, 30_000) do
      {:ok, state} -> state
      {:error, why} -> raise why
    end
  end

  @doc "Every simulator the One App made here: its udid, name, whose it is, what it's for, its state and times."
  def list(service) do
    states = for {udid, s} <- simulators(), into: %{}, do: {udid, s["state"]}
    kept = GenServer.call(service, :simulators)
    used = used(service)
    folder = folder(service, "simulators")

    for file <- File.ls!(folder), String.ends_with?(file, ".json") do
      s = JSON.decode!(File.read!(Path.join(folder, file)))

      state = kept[s["udid"]] || %{state: state(states[s["udid"]]), step: nil, why: nil}

      Map.merge(s, %{
        "state" => state.state,
        "step" => state.step,
        "why" => state.why,
        "last_used" => Map.get(used, s["udid"], s["created"])
      })
    end
  end

  @doc """
  Makes a simulator from `spec` (its `name`, `description`, `artifact` and `by`) of the newest iPhone Pro this Xcode
  has, on its newest iOS, and boots it.
  """
  def create(service, spec) do
    %{"runtimes" => runtimes} = JSON.decode!(simctl!(service, ~w(list --json runtimes)))
    runtimes = Enum.filter(runtimes, &(&1["platform"] == "iOS" and &1["isAvailable"]))
    if runtimes == [], do: raise("Xcode here has no iOS runtime to make one with: Xcode › Settings › Components adds one")
    runtime = Enum.max_by(runtimes, &numbers(&1["version"]))
    pros = for t <- runtime["supportedDeviceTypes"], [_, n] <- [Regex.run(~r/^iPhone (\d+) Pro$/, t["name"])], do: {String.to_integer(n), t}
    if pros == [], do: raise("iOS #{runtime["version"]} here runs no iPhone Pro")
    {_, type} = Enum.max_by(pros, &elem(&1, 0))

    udid = String.trim(simctl!(service, ["create", "One: #{spec["name"]}", type["identifier"], runtime["identifier"]]))
    s = Map.merge(spec, %{"udid" => udid, "model" => type["name"], "ios" => runtime["version"], "created" => System.system_time(:millisecond)})
    File.write!(Path.join(folder(service, "simulators"), udid <> ".json"), JSON.encode!(s))
    start(service, udid)
  end

  defp numbers(version), do: version |> String.split(".") |> Enum.map(&String.to_integer/1)

  @doc """
  Boots a simulator and waits until it has, and until idb's companion for it has started, which takes seconds the
  first time: Computer Use's list of screens, which asks idb for each, gives an app no longer than that.
  """
  def start(service, udid) do
    step(service, udid, "start", %{step: "booting"})

    try do
      boot(service, udid)
      step(service, udid, "started")
    rescue
      e ->
        step(service, udid, "fail", %{why: Exception.message(e)})
        reraise e, __STACKTRACE__
    end

    :ok
  end

  defp boot(service, udid) do
    touch(service, udid)

    case cmd(["xcrun", "simctl", "boot", udid], 120) do
      %{exit: 0} -> :ok
      %{stderr: why} -> if why =~ "current state: Booted", do: :ok, else: raise(String.trim(why))
    end

    simctl!(service, ["bootstatus", udid, "-b"], 600)
    step(service, udid, nil, %{step: "connecting"})
    # idb remembers each simulator's companion, in a file of its own, and keeps asking one that's gone, as each is once
    # its simulator shuts down, rather than starting another: it's told to forget it.
    %{exit: 0} = cmd(idb(service) ++ ["disconnect", udid], 30)
    describe(service, udid) || raise "idb couldn't reach it"
  end

  @doc "Shuts a simulator down, and it's `stopping` until it has."
  def stop(service, udid) do
    if step(service, udid, "stop") == "stopping" do
      try do
        shutdown(udid)
      after
        step(service, udid, "stopped")
      end
    end

    :ok
  end

  defp shutdown(udid) do
    case cmd(["xcrun", "simctl", "shutdown", udid], 120) do
      %{exit: 0} -> :ok
      %{stderr: why} -> if why =~ "current state: Shutdown", do: :ok, else: raise(String.trim(why))
    end
  end

  @doc "Removes a simulator and everything on it, for good."
  def remove(service, udid) do
    gone = step(service, udid, nil) == "gone"
    step(service, udid, "remove")

    try do
      if not gone do
        shutdown(udid)
        simctl!(service, ["delete", udid])
      end

      File.rm(Path.join(folder(service, "simulators"), udid <> ".json"))
      File.rm(Path.join(folder(service, "used"), udid))
    rescue
      e ->
        step(service, udid, "fail")
        reraise e, __STACKTRACE__
    end

    step(service, udid, "removed")
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
  takes as whole points: a click is a tap, a right click a long press, a drag a swipe, a scroll a swipe the other way.
  """
  def act(service, udid, action, inputs) do
    touch(service, udid)
    %{"points" => points} = describe(service, udid) || raise "It isn't running.\n\n`start` starts it."
    at = fn x, y -> [round(x * points), round(y * points)] end

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
      case cmd(idb(service) ++ ["ui" | Enum.map(args, &to_string/1)] ++ ["--udid", udid], 60) do
        %{exit: 0} -> :ok
        %{stdout: out, stderr: why} -> raise "idb: #{String.trim(clip(why <> "\n" <> out))}"
      end
    end

    :ok
  end

  # A scroll is a swipe the other way, as a finger scrolls: 80 points for each step, at most 600.
  defp scroll([x, y], direction, amount) do
    far = min(80 * amount, 600)
    {dx, dy} = %{"up" => {0, far}, "down" => {0, -far}, "left" => {far, 0}, "right" => {-far, 0}}[direction]
    ["swipe", x, y, max(x + dx, 0), max(y + dy, 0)] ++ ~w(--duration 0.3)
  end

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
  The booted simulators the One App made here, each as `list/1` gives it, with its screen's `width` and `height` in
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
    [_, "--companion-path", companion] = idb(service)
    %{"points" => points} = describe(service, udid) || raise "It isn't running.\n\n`start` starts it."
    file = Path.join(OneCore.Service.folder(service), "stream-#{System.unique_integer([:positive])}.json")
    config = Map.merge(JSON.decode!(request), %{"companion" => companion, "udid" => udid, "points" => points, "log" => Path.join(OneCore.Service.folder(service), "stream.log")})
    File.write!(file, JSON.encode!(config))

    case cmd([python(service), Path.join(OneCore.Service.folder(service, "server/stream"), "stream.py"), file], 30) do
      %{exit: 0, stdout: output} -> output
      %{stderr: why} -> raise "Couldn't stream its screen: #{String.trim(why)}"
    end
  end

  # Its screen as idb describes it: its size in pixels, and its points for each pixel.
  defp describe(service, udid) do
    with %{exit: 0, stdout: output} <- cmd(idb(service) ++ ["describe", "--json", "--udid", udid], 30),
         %{"screen_dimensions" => %{"width" => width, "height" => height, "width_points" => across}} <- JSON.decode!(output) do
      %{"width" => width, "height" => height, "points" => across / width}
    else
      _ -> nil
    end
  end

  # The simulators in the Mac's device set, by udid.
  defp simulators do
    %{exit: 0, stdout: output} = cmd(~w(xcrun simctl list --json devices), 30)
    for {_runtime, devices} <- JSON.decode!(output)["devices"], d <- devices, into: %{}, do: {d["udid"], d}
  end

  defp state("Booted"), do: "running"
  defp state("Booting"), do: "starting"
  defp state("Shutting Down"), do: "stopping"
  defp state(nil), do: "gone"
  defp state(_), do: "stopped"

  # idb's command, with its companion, installed here the first time: the companion from idb's release on GitHub,
  # checked, and idb's CLI in this service's Python. idb starts a companion for a simulator the first time it's asked
  # about it, and runs it with no options of ours, so it's started through a script that adds one: it ends once its
  # simulator is shut down, rather than staying for good.
  defp idb(service) do
    python(service)
    companion = Path.join([OneCore.Service.folder(service), "idb-companion-#{@idb}", "idb_companion"])
    started = Path.join(Path.dirname(companion), "companion")

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
      File.write!(Path.join(unpacking, "companion"), ~s(#!/bin/sh\nexec "#{companion}" "$@" --terminate-offline true\n))
      File.chmod!(Path.join(unpacking, "companion"), 0o755)
      File.rename!(unpacking, Path.dirname(companion))
    end

    [Path.join([OneCore.Service.folder(service), "venv", "bin", "idb"]), "--companion-path", started]
  end

  # The Python idb's CLI and a stream's sender run with, made with uv the first time.
  defp python(service) do
    venv = Path.join(OneCore.Service.folder(service), "venv")
    marker = Path.join(venv, "idb-#{@idb}-aiortc-#{@aiortc}")
    cli = "https://github.com/facebook/idb/releases/download/v#{@idb}/fb_idb-#{@idb}-py3-none-any.whl#sha256=#{@cli_sha256}"

    if not File.exists?(marker) do
      File.rm_rf!(venv)
      %{exit: 0} = cmd(["uv", "venv", "--quiet", "--python", "3.12", venv], 300)
      %{exit: 0} = cmd(["uv", "pip", "install", "--quiet", "--python", Path.join(venv, "bin/python"), "fb-idb @ #{cli}", "aiortc==#{@aiortc}"], 300)
      File.touch!(marker)
    end

    Path.join(venv, "bin/python")
  end

  defp simctl!(_service, args, timeout \\ 60) do
    case cmd(["xcrun", "simctl" | args], timeout) do
      %{exit: 0, stdout: out} -> out
      %{stderr: why, stdout: out} -> raise "simctl #{hd(args)}: #{String.trim(clip(why <> "\n" <> out))}"
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

  # Runs `args` for at most `timeout` seconds (`OneCore.Command`). What it prints is kept whole, as simctl's JSON is
  # read; only what goes into an error is cut short.
  defp cmd(args, timeout) do
    ran = OneCore.Command.run(args, timeout)
    %{ran | stderr: clip(ran.stderr)}
  end

  defp clip(text), do: OneCore.Command.clip(text, @max_bytes)
end
