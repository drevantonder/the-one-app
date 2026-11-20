defmodule AndroidEmulator.Emulator do
  @moduledoc """
  Runs Android emulators on its machine: the Android SDK's `emulator`, with no window, each an Android Virtual Device
  (AVD) of this service's own, in its folder's `avd/` (`ANDROID_AVD_HOME`), never one of the machine's own. adb
  reaches each at its console's port (`emulator-<port>`). What each is, its name, whose it is and what it's for, is a
  file beside its AVD, `<id>.json`, and when it was last used is a file's time, `used/<id>`.

  The emulator's own network gives a phone's apps this machine's 127.0.0.1 at 10.0.2.2, with no way not to. So on
  Linux each runs in a network of its own (pasta), reaching the internet as this machine does but nothing on its
  127.0.0.1, which takes in only the phone's console and adb ports. A Mac has nothing like pasta: there a phone's
  apps reach the Mac's 127.0.0.1.

  Each emulator is in one state, which changes only as `@steps` says, and every app watching is told as it does:
  `stopped`, `starting` (its `step`: `preparing` the SDK, then `booting` Android), `running`, `stopping` (it saves
  where it was, which it does alone once asked) and `removing`. Running and stopping are what the machine says (an
  emulator's processes, and the `.stopping` file a stop leaves beside its AVD), so they outlast this service's
  restarts; starting and removing, and why a start failed, are this service's own while a call is under way. A
  step that isn't one, such as Start while it's stopping, is refused saying why.
  """
  use GenServer
  @behaviour OneCore.Service

  # An emulator's states, by the state it's in and what happens, as the moduledoc says. `exited` is its emulator
  # having ended, `fail` a start that couldn't finish.
  @steps %{
    {"stopped", "start"} => "starting",
    {"starting", "started"} => "running",
    {"starting", "fail"} => "stopped",
    {"starting", "stop"} => "stopping",
    {"running", "stop"} => "stopping",
    {"stopping", "stop"} => "stopping",
    {"stopped", "stop"} => "stopped",
    {"stopping", "fail"} => "stopping",
    {"stopped", "fail"} => "stopped",
    {"running", "exited"} => "stopped",
    {"stopping", "exited"} => "stopped",
    {"starting", "exited"} => "starting",
    {"removing", "exited"} => "removing",
    {"stopped", "exited"} => "stopped",
    {"stopped", "remove"} => "removing",
    {"starting", "remove"} => "removing",
    {"running", "remove"} => "removing",
    {"stopping", "remove"} => "removing",
    {"removing", "removed"} => "stopped",
    {"removing", "fail"} => "stopped"
  }
  # The states this service keeps, as the machine can't say them.
  @kept ~w(starting removing)

  # `emulators` has those in a state of `@kept`, or stopped with why a start failed, by id; `watched` the emulator
  # each port watches, by port.
  defstruct emulators: %{}, watched: %{}

  # Android 16, with Google Play, as Google's emulator images name it, for this machine's processor.
  @android "android-36"
  @tag "google_apis_playstore"
  @memory_mb 2048
  # The command-line tools Google publishes, to install the rest with, where the machine has no SDK.
  @cmdline_tools "13114758"
  # scrcpy's server, which a stream runs in the phone for its H.264 and its input, and the file's SHA-256.
  @scrcpy "5.0.1"
  @scrcpy_sha256 "764eb6f79811d5211fe9df341120882ba9994c7a61b897d7bf3fb662e53bc536"
  @aiortc "1.16.0"
  @max_bytes 64 * 1024

  @impl OneCore.Service
  def called, do: "Android Emulator's emulators"

  @impl OneCore.Service
  def os, do: ["linux", "darwin"]

  @impl OneCore.Service
  def needs do
    "A Linux x86 machine with KVM (/dev/kvm) and pasta (the passt package), or a Mac, with uv and Java. It runs " <>
      "Android emulators as the user this server runs as, about 2 GB of memory and a CPU or two each while they " <>
      "run. It uses the Android SDK where ANDROID_HOME or ~/Android/Sdk has one, and otherwise installs Google's " <>
      "command-line tools, emulator and an Android 16 system image in its own folder, accepting Google's SDK " <>
      "licences, the first time it's used (a few GB). On Linux each phone has a network of its own, reaching the " <>
      "internet but nothing on this machine; on a Mac its apps reach the Mac's 127.0.0.1."
  end

  @impl GenServer
  def init(_) do
    case :os.type() do
      {:unix, :darwin} ->
        :ok

      {:unix, :linux} ->
        File.exists?("/dev/kvm") || raise "Android's emulator needs KVM here (/dev/kvm)"
        System.find_executable("pasta") || raise "Android's emulator needs pasta here (the passt package), for a network of its own"

      _ ->
        raise "Android's emulator runs on Linux and Macs"
    end

    System.find_executable("uv") || raise "This machine has no uv, which a stream's sender is installed with"
    {:ok, %__MODULE__{}, {:continue, :watch}}
  end

  # Emulators outlive this service: those running as it starts are watched, so it can say when each stops.
  @impl GenServer
  def handle_continue(:watch, data) do
    data = Enum.reduce(processes(), data, fn {id, p}, data -> if p.emulator, do: watched(data, id, p.emulator), else: data end)
    {:noreply, data}
  end

  # Each version's module has a name of its own, so its struct is taken by its fields. A version before this one
  # kept nothing (`nil`), and one started afresh watches what's running.
  @impl GenServer
  def code_change(_old, %_{} = data, _extra), do: {:ok, struct(__MODULE__, Map.from_struct(data))}
  def code_change(_old, _data, _extra), do: {:error, "kept nothing to carry on with"}

  @impl GenServer
  def handle_call({:step, id, event, changes}, _from, data) do
    kept = data.emulators[id]
    was = (kept && kept.state) || derived(self(), id, processes()[id])

    case if(event, do: @steps[{was, event}], else: was) do
      nil ->
        {:reply, {:error, refused(was, event)}, data}

      to ->
        # Why a start failed is kept until the next step, and only for one that was starting.
        why = if event == "fail" and was == "starting", do: changes[:why]
        step = if to == "starting", do: changes[:step] || (kept && kept.step)

        emulators =
          if to in @kept or why,
            do: Map.put(data.emulators, id, %{state: to, step: step, why: why}),
            else: Map.delete(data.emulators, id)

        # An emulator that ended was running a moment ago, though the machine says it's stopped already.
        if emulators != data.emulators or to != was or event == "exited", do: OneCore.Service.changed(self())
        {:reply, {:ok, to}, %{data | emulators: emulators}}
    end
  end

  def handle_call(:emulators, _from, data), do: {:reply, data.emulators, data}

  @impl GenServer
  def handle_cast({:watch, id, pid}, data), do: {:noreply, watched(data, id, pid)}

  # An emulator that was watched has ended: stopped, or stopped by itself.
  @impl GenServer
  def handle_info({port, {:exit_status, _}}, %{watched: watched} = data) when is_map_key(watched, port) do
    {id, watched} = Map.pop(watched, port)
    {:reply, _, data} = handle_call({:step, id, "exited", %{}}, nil, %{data | watched: watched})
    {:noreply, data}
  end

  def handle_info(_message, data), do: {:noreply, data}

  # Watches the emulator `id`, whose process is `pid`, until it ends. It isn't this server's child (it's in a scope
  # of its own, to outlive the server's restarts), so nothing tells the server when it ends: a shell asks the
  # machine every second, and ends with it, or with this service.
  defp watched(data, id, pid) do
    if id in Map.values(data.watched) do
      data
    else
      wait = ~s(while kill -0 "$0" 2>/dev/null; do read -t 1 _; [ $? = 1 ] && exit; done)
      port = Port.open({:spawn_executable, System.find_executable("bash")}, [:exit_status, args: ["-c", wait, "#{pid}"]])
      put_in(data.watched[port], id)
    end
  end

  defp refused("starting", "start"), do: "It's starting already."
  defp refused("running", "start"), do: "It's running already."
  defp refused("stopping", "start"), do: "It's still stopping, as it saves where it was. Start it once it's stopped.\n\n`list` says when it is."
  defp refused("removing", _), do: "It's being removed."
  defp refused(state, event), do: "It can't #{event} while it's #{state}."

  # Takes the emulator `id` one step (`event`, by `@steps`, or `nil` for none) with `changes`, and gives its state.
  defp step(service, id, event, changes \\ %{}) do
    case GenServer.call(service, {:step, id, event, changes}) do
      {:ok, state} -> state
      {:error, why} -> raise why
    end
  end

  @doc "Every emulator here: its id, name, whose it is, what it's for, its state and when it was created and last used."
  def list(service) do
    avd = avd_home(service)
    used = OneCore.Service.used(service)
    running = processes()
    kept = GenServer.call(service, :emulators)

    for file <- if(File.dir?(avd), do: File.ls!(avd), else: []), String.ends_with?(file, ".json") do
      device = JSON.decode!(File.read!(Path.join(avd, file)))
      id = device["id"]
      state = kept[id] || %{state: derived(service, id, running[id]), step: nil, why: nil}

      Map.merge(device, %{
        "state" => state.state,
        "step" => state.step,
        "why" => state.why,
        "last_used" => Map.get(used, id, device["created"])
      })
    end
  end

  @doc """
  Makes an AVD from `spec` (its `id`, `name`, `description`, `artifact` and `by`) with Google's `avdmanager`, from
  Android 16's system image with Google Play, installing what the SDK lacks first, and starts it, with Chrome's first
  run done.
  """
  def create(service, spec) do
    sdk = sdk!(service)
    id = spec["id"]

    avdmanager =
      ["sh", "-c", ~s(echo no | "$0" "$@"), Path.join(sdk, "cmdline-tools/latest/bin/avdmanager")] ++
        ["create", "avd", "--name", id, "--package", image(), "--device", "pixel_8", "--path", Path.join(avd_home(service), id <> ".avd")]

    run!(service, avdmanager, 120)
    device = Map.merge(spec, %{"created" => System.system_time(:millisecond)})
    File.write!(Path.join(avd_home(service), id <> ".json"), JSON.encode!(device))
    :ok = start(service, id)
    chrome_first_run(service, id)
  end

  # Chrome asks to sign in, then about notifications, the first time it opens, and so would every page opened in it. Its
  # command-line file (`--disable-fre`) is read only on a phone that's debuggable, which this image isn't, so the two
  # prompts are answered as a person would, by the text on their buttons. What it answers is kept in the phone's disk.
  defp chrome_first_run(service, id) do
    adb!(service, id, ["shell", "am start -a android.intent.action.VIEW -d about:blank"], 30)

    for button <- ["Use without an account", "No thanks"] do
      script = ~s|for i in $(seq 60); do uiautomator dump /sdcard/ui.xml >/dev/null 2>&1; set -- $(sed -n 's/.*text="#{button}"[^>]*bounds="\\[\\([0-9]*\\),\\([0-9]*\\)\\]\\[\\([0-9]*\\),\\([0-9]*\\)\\]".*/\\1 \\2 \\3 \\4/p' /sdcard/ui.xml); [ $# = 4 ] && exec input tap $((($1 + $3) / 2)) $((($2 + $4) / 2)); sleep 1; done; echo "no \"#{button}\" button in 60 s"; exit 1|
      adb!(service, id, ["shell", "sh -c " <> word(script)], 90)
    end

    adb!(service, id, ["shell", "input keyevent KEYCODE_HOME"], 10)
    :ok
  end

  @doc """
  Starts an emulator and waits for Android to have started. It carries on from where it was when it last stopped,
  if that stop saved it (the emulator's Quick Boot snapshot), and otherwise boots: a snapshot taken before a stop
  that didn't save would take its disk back to then.
  """
  def start(service, id) do
    step(service, id, "start", %{step: "preparing"})

    try do
      boot(service, id)
      step(service, id, "started")
    rescue
      e ->
        step(service, id, "fail", %{why: Exception.message(e)})
        reraise e, __STACKTRACE__
    end

    :ok
  end

  defp boot(service, id) do
    sdk = sdk!(service)
    step(service, id, nil, %{step: "booting"})
    port = port()
    log = log(service, id)
    saved = String.contains?(read(log), "Saving snapshot 'default_boot' took")
    File.rm(Path.join(avd_home(service), id <> ".stopping"))
    OneCore.Service.touch(service, id)

    emulator =
      [Path.join(sdk, "emulator/emulator"), "-avd", id, "-port", "#{port}", "-no-window", "-no-audio", "-no-boot-anim"] ++
        ["-gpu", "swiftshader_indirect", "-accel", "on", "-memory", "#{@memory_mb}"] ++
        if(saved, do: [], else: ["-no-snapshot-load"])

    # On Linux: in a network of its own, and in a scope of the user's systemd, so it outlives this server's
    # restarts, which stop everything in the server's own, and what it starts in its network (its own adb server,
    # netsimd) is stopped with it. Its runtime folder is its own, for its own netsimd (its
    # radios): one an emulator in another network started is unreachable from this one, and a phone that waits for
    # it never starts. It asks this machine's resolvers, not a stub on its 127.0.0.53.
    command =
      case :os.type() do
        {:unix, :linux} ->
          runtime = Path.join([OneCore.Service.folder(service), "run", id])
          File.mkdir_p!(runtime)

          ["systemd-run", "--user", "--scope", "--unit", scope(id), "--quiet", "--collect", "--", "env", "XDG_RUNTIME_DIR=#{runtime}"] ++
            ["pasta", "--config-net", "--no-map-gw", "-T", "none", "-U", "none", "-u", "none", "-t", "127.0.0.1/#{port}-#{port + 1}", "--quiet", "--"] ++
            emulator ++ dns()

        _ ->
          emulator
      end

    # Started in the background with what it says in its log; it's in a session of its own already, as everything
    # Erlang starts is.
    %{exit: 0} = run(service, ["sh", "-c", ~s("$@" >"$0" 2>&1 </dev/null &), log | command], 10)
    booted(service, id, port, System.monotonic_time(:millisecond), false)
    with %{emulator: pid} when is_integer(pid) <- processes()[id], do: GenServer.cast(service, {:watch, id, pid})

    for args <- [~w(svc power stayon true), ~w(input keyevent KEYCODE_WAKEUP), ~w(wm dismiss-keyguard)],
        do: adb(service, id, ["shell" | args], 10)
  end

  @doc """
  Stops an emulator, as its own SIGTERM does: it saves what it was doing (its Quick Boot snapshot) and ends, a few
  seconds' work it carries on with alone, and it's `stopping` until it has.
  """
  def stop(service, id) do
    if step(service, id, "stop") == "stopping" do
      File.touch!(Path.join(avd_home(service), id <> ".stopping"))

      case processes()[id] do
        %{emulator: pid} when is_integer(pid) ->
          GenServer.cast(service, {:watch, id, pid})
          System.cmd("kill", ["-TERM", "#{pid}"])

        _ ->
          step(service, id, "exited")
      end
    end

    :ok
  end

  @doc "Removes an emulator and its AVD, for good, stopping it at once if it's running."
  def remove(service, id) do
    step(service, id, "remove")

    try do
      kill(id)
      avd = avd_home(service)
      File.rm_rf!(Path.join(avd, id <> ".avd"))
      for file <- [id <> ".ini", id <> ".json", id <> ".stopping", id <> ".log"], do: File.rm(Path.join(avd, file))
      File.rm_rf!(Path.join([OneCore.Service.folder(service), "run", id]))
      File.rm(Path.join([OneCore.Service.folder(service), "used", id]))
    rescue
      e ->
        step(service, id, "fail")
        reraise e, __STACKTRACE__
    end

    step(service, id, "removed")
    :ok
  end

  @doc "Stops every running emulator that's gone unused for `idle` ms, and gives their names."
  def stop_idle(service, idle) do
    now = System.system_time(:millisecond)

    for d <- list(service), d["state"] == "running", now - d["last_used"] > idle do
      stop(service, d["id"])
      d["name"]
    end
  end

  @doc "A PNG of the phone's screen, as base64, by Android's `screencap`."
  def screenshot(service, id) do
    OneCore.Service.touch(service, id)

    case adb(service, id, ~w(exec-out screencap -p), 15, :raw) do
      %{exit: 0, stdout: png} -> Base.encode64(png)
      %{stderr: why} -> raise "No screenshot: #{String.trim(why)}"
    end
  end

  @doc "Runs Android's `input` with `args`, in the phone's shell: a tap, a swipe, text, a key."
  def input(service, id, args) do
    OneCore.Service.touch(service, id)
    %{exit: 0} = adb!(service, id, ["shell", Enum.join(["input" | Enum.map(args, &word/1)], " ")], 30)
    :ok
  end

  @doc "Opens `url` in the phone, as its own app for it or its browser does."
  def open_url(service, id, url) do
    OneCore.Service.touch(service, id)
    adb!(service, id, ["shell", "am start -a android.intent.action.VIEW -d #{word(url)}"], 30)
    :ok
  end

  @doc "Installs an APK, given as base64 or by its URL, granting what it asks for."
  def install(service, id, apk) do
    OneCore.Service.touch(service, id)
    file = Path.join(OneCore.Service.folder(service), "install-#{System.unique_integer([:positive])}.apk")

    try do
      case apk do
        %{"url" => url} -> Req.get!(url, into: File.stream!(file), retry: false, receive_timeout: 300_000)
        %{"data" => data} -> File.write!(file, Base.decode64!(data))
      end

      %{stdout: output} = adb!(service, id, ["install", "-r", "-g", file], 600)
      String.trim(output)
    after
      File.rm(file)
    end
  end

  @doc "Runs `command` in the phone's shell, as adb's shell user, for at most `timeout` seconds."
  def shell(service, id, command, timeout) do
    OneCore.Service.touch(service, id)
    adb(service, id, ["shell", command], timeout)
  end

  @doc """
  The running emulators here, each as `list/1` gives it, with its screen's `width` and `height` in pixels as
  Android says them now.
  """
  def screens(service) do
    for d <- list(service), d["state"] == "running",
        %{exit: 0, stdout: output} <- [adb(service, d["id"], ~w(shell wm size), 10)],
        [_, width, height] <- [Regex.run(~r/(\d+)x(\d+)\s*$/, String.trim(output))],
        do: Map.merge(d, %{"width" => String.to_integer(width), "height" => String.to_integer(height)})
  end

  @doc """
  Streams the phone's screen as WebRTC video to the app whose SDP offer `request` has, as JSON, with the ICE servers
  to use: `server/stream/stream.py` answers it and carries on by itself until the app goes. Gives the SDP answer.
  """
  def stream(service, id, request) do
    OneCore.Service.touch(service, id)
    python = sender(service)
    adb!(service, id, ["push", scrcpy(service), "/data/local/tmp/scrcpy-server.jar"], 30)
    folder = OneCore.Service.folder(service)
    file = Path.join(folder, "stream-#{System.unique_integer([:positive])}.json")

    config =
      Map.merge(JSON.decode!(request), %{
        "adb" => adb_path(service),
        "serial" => serial(id),
        "version" => @scrcpy,
        "log" => Path.join(folder, "stream.log")
      })

    File.write!(file, JSON.encode!(config))

    case run(service, [python, Path.join(OneCore.Service.folder(service, "server/stream"), "stream.py"), file], 30) do
      %{exit: 0, stdout: output} -> output
      %{stderr: why} -> raise "Couldn't stream its screen: #{String.trim(why)}"
    end
  end

  # The SDK: the machine's own, where it has one with the emulator, or this service's, installed here the first
  # time. Either way with what an emulator needs: the emulator, adb and the system image.
  defp sdk(service) do
    Enum.find(
      [System.get_env("ANDROID_HOME"), System.get_env("ANDROID_SDK_ROOT"), Path.expand("~/Android/Sdk"), Path.expand("~/Library/Android/sdk")],
      &(&1 && File.exists?(Path.join(&1, "emulator/emulator")))
    ) || Path.join(OneCore.Service.folder(service), "sdk")
  end

  defp sdk!(service) do
    sdk = sdk(service)

    image = Path.join([sdk, "system-images", @android, @tag, abi(), "system.img"])
    sdkmanager = Path.join(sdk, "cmdline-tools/latest/bin/sdkmanager")

    if not (File.exists?(image) and File.exists?(Path.join(sdk, "platform-tools/adb")) and File.exists?(sdkmanager)) do
      if not File.exists?(sdkmanager), do: cmdline_tools(service, sdk)
      run!(service, ["sh", "-c", ~s(yes | "$0" --sdk_root="$1" --licenses >/dev/null), sdkmanager, sdk], 120)
      run!(service, [sdkmanager, "--sdk_root=#{sdk}", "platform-tools", "emulator", image()], 3600)
    end

    sdk
  end

  # Google's command-line tools, unpacked as the SDK keeps them, in cmdline-tools/latest.
  defp cmdline_tools(service, sdk) do
    os = if match?({:unix, :darwin}, :os.type()), do: "mac", else: "linux"
    zip = Path.join(OneCore.Service.folder(service), "cmdline-tools.zip")
    url = "https://dl.google.com/android/repository/commandlinetools-#{os}-#{@cmdline_tools}_latest.zip"
    Req.get!(url, into: File.stream!(zip), receive_timeout: 600_000)
    unpacked = Path.join(sdk, "cmdline-tools.unpacking")
    File.rm_rf!(unpacked)
    {:ok, _} = :zip.unzip(String.to_charlist(zip), cwd: String.to_charlist(unpacked))
    for bin <- File.ls!(Path.join(unpacked, "cmdline-tools/bin")), do: File.chmod!(Path.join([unpacked, "cmdline-tools/bin", bin]), 0o755)
    File.mkdir_p!(Path.join(sdk, "cmdline-tools"))
    File.rename!(Path.join(unpacked, "cmdline-tools"), Path.join(sdk, "cmdline-tools/latest"))
    File.rm_rf!(unpacked)
    File.rm!(zip)
  end

  defp image, do: "system-images;#{@android};#{@tag};#{abi()}"

  defp abi do
    if String.starts_with?(to_string(:erlang.system_info(:system_architecture)), "aarch64"), do: "arm64-v8a", else: "x86_64"
  end

  # The Python a stream's sender runs with, aiortc in it, made with uv the first time.
  defp sender(service) do
    venv = Path.join(OneCore.Service.folder(service), "venv")
    python = Path.join(venv, "bin/python")
    marker = Path.join(venv, "aiortc-#{@aiortc}")

    if not File.exists?(marker) do
      File.rm_rf!(venv)
      run!(service, ["uv", "venv", "--quiet", "--python", "3.12", venv], 300)
      run!(service, ["uv", "pip", "install", "--quiet", "--python", python, "aiortc==#{@aiortc}"], 300)
      File.touch!(marker)
    end

    python
  end

  # scrcpy's server, downloaded from its releases the first time, and checked.
  defp scrcpy(service) do
    file = Path.join(OneCore.Service.folder(service), "scrcpy-server-v#{@scrcpy}")

    if not File.exists?(file) do
      url = "https://github.com/Genymobile/scrcpy/releases/download/v#{@scrcpy}/scrcpy-server-v#{@scrcpy}"
      %{status: 200, body: body} = Req.get!(url, decode_body: false, retry: false)

      if Base.encode16(:crypto.hash(:sha256, body), case: :lower) != @scrcpy_sha256,
        do: raise("scrcpy's server from GitHub isn't the one expected")

      File.write!(file, body)
    end

    file
  end

  # Each running emulator here, by its AVD: its processes (the emulator's, and on Linux its pasta and scope), which
  # of them is the emulator itself (QEMU, which the SDK's `emulator` becomes), and its console port.
  defp processes do
    {ps, 0} = System.cmd("ps", ["-axo", "pid=,args="])

    # By its words, as a pattern of the whole line backtracks past its limit on the long ones some programs have.
    for line <- String.split(ps, "\n", trim: true),
        [pid, program | args] <- [String.split(line)],
        id = after_flag(args, "-avd"),
        id != nil,
        reduce: %{} do
      running ->
        pid = String.to_integer(pid)
        port = with port when port != nil <- after_flag(args, "-port"), do: String.to_integer(port)
        emulator = if Path.basename(program) =~ ~r/^(qemu-system|emulator)/, do: pid
        was = Map.get(running, id, %{pids: [], port: nil, emulator: nil})
        Map.put(running, id, %{pids: [pid | was.pids], port: was.port || port, emulator: was.emulator || emulator})
    end
  end

  defp after_flag(args, flag), do: args |> Enum.drop_while(&(&1 != flag)) |> Enum.at(1)

  # An emulator's state as the machine says it, by its processes: none is stopped, and one a stop left its file
  # beside is stopping.
  defp derived(_service, _id, nil), do: "stopped"

  defp derived(service, id, _processes),
    do: if(File.exists?(Path.join(avd_home(service), id <> ".stopping")), do: "stopping", else: "running")

  # Ends an emulator at once, keeping nothing: on Linux its scope, with everything it started; on a Mac its
  # processes, by their PIDs.
  defp kill(id) do
    case :os.type() do
      {:unix, :linux} -> System.cmd("systemctl", ["--user", "kill", "--signal=KILL", scope(id) <> ".scope"], stderr_to_stdout: true)
      _ -> with %{pids: pids} <- processes()[id], do: System.cmd("kill", ["-KILL" | Enum.map(pids, &to_string/1)])
    end
  end

  defp scope(id), do: "android-emulator-#{id}"

  # A phone takes a while to start, and nothing says when it has: Android is asked every second, for at most five
  # minutes. The emulator tells adb's server it's there, unless it's in a network of its own, where it can't reach
  # it: then it's told here, as the emulator would have. Its process shows a moment after it's started, and one
  # that's gone after that has stopped.
  defp booted(service, id, port, since, seen) do
    announce(service, port + 1)
    running = processes()[id] != nil
    waited = System.monotonic_time(:millisecond) - since

    cond do
      running and match?(%{exit: 0, stdout: "1" <> _}, adb(service, id, ~w(shell getprop sys.boot_completed), 5)) ->
        :ok

      not running and (seen or waited > 10_000) ->
        raise "Its emulator stopped as it started.\n\n#{String.slice(read(log(service, id)), -2000..-1//1)}"

      waited > 300_000 ->
        kill(id)
        raise "It didn't start in 5 minutes, so it was stopped.\n\n#{String.slice(read(log(service, id)), -2000..-1//1)}"

      true ->
        Process.sleep(1000)
        booted(service, id, port, since, seen or running)
    end
  end

  # Tells adb's server of the emulator whose adb is at `port`, in adb's own words: `host:emulator:<port>`.
  defp announce(service, port) do
    case :gen_tcp.connect(~c"127.0.0.1", 5037, [:binary, active: false], 1000) do
      {:ok, socket} ->
        message = "host:emulator:#{port}"
        :gen_tcp.send(socket, String.pad_leading(Integer.to_string(byte_size(message), 16), 4, "0") <> message)
        :gen_tcp.close(socket)

      _ ->
        run(service, [adb_path(service), "start-server"], 10)
    end
  end

  # A console port that no emulator here uses, nor anything else on the machine: from 5560, leaving the SDK's own
  # first, 5554, to whoever runs one by hand. Its adb port is the one after it.
  defp port do
    taken =
      for line <- String.split(elem(System.cmd("ps", ["-axo", "args="]), 0), "\n"),
          [_, port] <- [Regex.run(~r/\s-port\s(\d+)/, line)],
          do: String.to_integer(port)

    Enum.find(5560..5680//2, fn port ->
      port not in taken and Enum.all?([port, port + 1], &free?/1)
    end) || raise "Every emulator port here is taken"
  end

  defp free?(port) do
    case :gen_tcp.connect(~c"127.0.0.1", port, [], 200) do
      {:ok, socket} -> :gen_tcp.close(socket) && false
      _ -> true
    end
  end

  # The resolvers this machine asks, not a stub on its 127.0.0.53, which a phone in its own network can't reach.
  defp dns do
    found =
      for file <- ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"],
          line <- String.split(read(file), "\n"),
          [_, ip] <- [Regex.run(~r/^nameserver\s+(\d+\.\d+\.\d+\.\d+)\s*$/, line)],
          not String.starts_with?(ip, "127."),
          uniq: true,
          do: ip

    if found == [], do: [], else: ["-dns-server", Enum.join(found, ",")]
  end

  defp avd_home(service) do
    folder = Path.join(OneCore.Service.folder(service), "avd")
    File.mkdir_p!(folder)
    folder
  end

  defp env(service, sdk), do: [{"ANDROID_HOME", sdk}, {"ANDROID_SDK_ROOT", sdk}, {"ANDROID_AVD_HOME", avd_home(service)}]

  defp log(service, id), do: Path.join(avd_home(service), id <> ".log")

  defp adb_path(service), do: Path.join(sdk(service), "platform-tools/adb")

  # adb's name for a running emulator: `emulator-` and its console port.
  defp serial(id) do
    case processes()[id] do
      %{port: port} when is_integer(port) -> "emulator-#{port}"
      _ -> raise "It isn't running.\n\n`start` starts it."
    end
  end

  # Runs adb for the emulator `id`, for at most `timeout` seconds: a phone whose Android has hung leaves adb waiting
  # for as long as it's let.
  defp adb(service, id, args, timeout, output \\ :text), do: run(service, [adb_path(service), "-s", serial(id) | args], timeout, output)

  defp adb!(service, id, args, timeout) do
    case adb(service, id, args, timeout) do
      %{exit: 0} = done -> done
      %{exit: -1} -> raise "It stopped answering.\n\nadb had no answer in #{timeout} s. `stop` and `start` it if it stays so."
      %{stdout: stdout, stderr: stderr} -> raise String.trim(stderr <> "\n" <> stdout)
    end
  end

  defp run!(service, args, timeout) do
    case run(service, args, timeout) do
      %{exit: 0} = done -> done
      %{stdout: stdout, stderr: stderr} -> raise "#{Path.basename(hd(args))} failed: #{String.trim(stderr <> "\n" <> stdout)}"
    end
  end

  # Runs `args` for at most `timeout` seconds (`OneCore.Command`), with the SDK's variables.
  defp run(service, args, timeout, output \\ :text) do
    ran = OneCore.Command.run(args, timeout, env: env(service, sdk(service)))
    if output == :raw, do: ran, else: %{ran | stdout: clip(ran.stdout), stderr: clip(ran.stderr)}
  end

  defp clip(text), do: OneCore.Command.clip(text, @max_bytes)

  defp read(file) do
    case File.read(file) do
      {:ok, text} -> text
      _ -> ""
    end
  end

  # A word for the phone's shell, which reads what adb sends it as a command line.
  defp word(text), do: "'" <> String.replace(text, "'", ~S('\'')) <> "'"
end
