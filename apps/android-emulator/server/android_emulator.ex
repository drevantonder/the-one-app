defmodule AndroidEmulator do
  @moduledoc """
  Android Emulator: Android phones in the Android SDK's emulator, on the machines that run its service,
  `AndroidEmulator.Emulator`.

  An emulator is an artifact made for whoever created it (`OneCore.Call.make/2`), so who sees and uses one is what
  `reach` says: its creator, whoever they share it with, and the network's owner. A job stops any left running and
  unused for longer than two hours, whoever's they are: what it saves is the machine's.
  """
  @behaviour OneCore.App
  alias AndroidEmulator.Emulator
  alias OneCore.{Artifact, Call, Service}

  @idle_hours 2
  @emulator %{type: "string", description: "the emulator, by its name"}

  # One, as `list` answers each.
  @listed %{
    type: "object",
    properties: %{
      name: %{type: "string"},
      description: %{type: ["string", "null"], description: "What it's for"},
      state: %{type: "string", enum: ~w(stopped starting running stopping removing)},
      step: %{type: ["string", "null"], enum: ["preparing", "booting", nil], description: "While it's starting: preparing the SDK, or booting Android"},
      why: %{type: ["string", "null"], description: "Why it couldn't start, when it's stopped for that"},
      machine: %{type: ["string", "null"]},
      by: %{type: ["string", "null"], description: "Who created it"},
      created: %{type: "integer", description: "In milliseconds since 1970"},
      last_used: %{type: "integer", description: "In milliseconds since 1970"}
    },
    required: ["name", "description", "state", "step", "why", "machine", "by", "created", "last_used"]
  }

  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def requires do
    %{
      "core" => ">= 0.32.47",
      "database" => "to keep your emulators",
      "webrtc" => "to show an emulator's screen live"
    }
  end

  @impl true
  def jobs, do: [%{tool: "stop_idle", every: 3600}]

  @impl true
  def hooks, do: ["computer-use.screens"]

  @impl true
  def entrypoints,
    do: [
      %{
        page: "client/index.html",
        subject: "user",
        runtime_version: 4
      }
    ]

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "The emulators you can use, on every machine: each one's name, state (stopped, starting, running, stopping, removing), machine, what it's for, who created it, and when it was created and last used (ms since 1970).",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{emulators: %{type: "array", items: @listed}}, required: ["emulators"]}
      },
      %{
        name: "machines",
        read_only: true,
        description: "The machines that run emulators, each by the name `create` takes, whether it's on now, and if not when it last was (ms since 1970).",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            machines: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  machine: %{type: ["string", "null"], description: "What create's machine takes"},
                  on: %{type: "boolean"},
                  last_on: %{type: "integer", description: "When it last was, if it's off, in milliseconds since 1970"}
                },
                required: ["machine", "on"]
              }
            }
          },
          required: ["machines"]
        }
      },
      %{
        name: "create",
        description:
          "Create an Android emulator (Android 16 with Google Play, a Pixel 8's screen) and start it, which takes a minute or two; the first on a machine may install the Android SDK first, which takes longer. Gives it as `list` does.",
        inputs: %{
          type: "object",
          required: ["name", "description"],
          properties: %{
            name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "its name, unique among yours"},
            description: %{type: "string", description: "what it's for, in a few words, for whoever sees it later"},
            machine: %{type: "string", description: "the machine to run it on, as `machines` names it; default the first"}
          }
        },
        outputs: @listed
      },
      %{name: "start", description: "Start a stopped emulator, carrying on from where it was when it stopped. Takes up to a minute or two.", inputs: one(),
      outputs: %{type: "object", properties: %{emulator: %{type: "string"}}, required: ["emulator"]}},
      %{name: "stop", description: "Stop an emulator. It saves what it was doing, and `start` carries on from there.", inputs: one(),
      outputs: %{type: "object", properties: %{emulator: %{type: "string"}}, required: ["emulator"]}},
      %{name: "remove", description: "Remove an emulator and everything on it, for good.", inputs: one(),
      outputs: %{type: "object", properties: %{emulator: %{type: "string"}}, required: ["emulator"]}},
      %{
        name: "install",
        description: "Install an app on a running emulator from its APK, granting what it asks for. Give the APK as an upload, or its url.",
        inputs:
          one(%{
            apk: %{type: "string", format: "upload", description: "the APK, uploaded"},
            url: %{type: "string", description: "or a URL to download the APK from"}
          }),
        outputs: %{type: "object", properties: %{emulator: %{type: "string"}, said: %{type: "string", description: "What installing it said"}}, required: ["emulator", "said"]}
      },
      %{
        name: "open_url",
        description: "Open a URL on a running emulator, in the app for it or the browser (Chrome).",
        inputs: one(%{url: %{type: "string", description: "the URL, like https://example.com or myapp://home"}}, ["url"]),
        outputs: %{type: "object", properties: %{emulator: %{type: "string"}, url: %{type: "string"}}, required: ["emulator", "url"]}
      },
      %{
        name: "shell",
        description:
          "Run a command in a running emulator's shell (adb shell), as its shell user, and wait for it: `logcat -d -t 200` for its log, `pm list packages -3` for the apps installed, `am force-stop <package>`. Gives { exit, stdout, stderr }, the last 64 KB of each.",
        inputs:
          one(
            %{
              command: %{type: "string", description: "the command line"},
              timeout: %{type: "integer", minimum: 1, maximum: 600, description: "seconds to wait, 1 to 600; default 60"}
            },
            ["command"]
          ),
        outputs: %{
          type: "object",
          properties: %{
            exit: %{type: "integer", description: "0 when it worked, -1 once its timeout passed"},
            stdout: %{type: "string"},
            stderr: %{type: "string"}
          },
          required: ["exit", "stdout", "stderr"]
        }
      },
      %{
        name: "screenshot",
        read_only: true,
        description: "A picture of a running emulator's screen, as a PNG, in the phone's own pixels.",
        inputs: one(),
        outputs: %{
          type: "object",
          description: "The screen, a PNG: an agent sees it",
          properties: %{mimeType: %{type: "string"}, size: %{type: "integer"}},
          required: ["mimeType"]
        }
      },
      %{
        name: "screen",
        description:
          "Look at or act on an emulator's screen, for Computer Use, whose tools are how agents use a screen: they say what each action takes. A click is a tap, a right click a long press, a drag a swipe, and keys are as xdotool names them (Escape is Back). `stream` is for a person's app to show it as live video over WebRTC: with no `offer`, the ICE servers to connect with; with the app's whole SDP `offer`, {answer}, the screen's.",
        inputs:
          one(
            %{
              action: %{type: "string", enum: ~w(screenshot click double_click move drag scroll type key resize stream)},
              offer: %{type: "string", description: "for stream: the app's SDP offer, with its candidates"},
              x: %{type: "integer", minimum: 0},
              y: %{type: "integer", minimum: 0},
              button: %{type: "string", enum: ~w(left right middle)},
              path: %{type: "array", minItems: 2, items: %{type: "object", required: ["x", "y"], properties: %{x: %{type: "integer"}, y: %{type: "integer"}}}},
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50},
              text: %{type: "string"},
              keys: %{type: "string"},
              width: %{type: "integer"},
              height: %{type: "integer"}
            },
            ["action"]
          ),
        outputs: %{
          type: "object",
          description: "As Computer Use says: a screenshot, what it did, or for stream, the ICE servers or the screen's answer",
          properties: %{
            mimeType: %{type: "string"},
            size: %{type: "integer"},
            done: %{type: "string", description: "What it did, in a few words"},
            iceServers: %{type: "array", items: %{type: "object"}},
            answer: %{type: "string", description: "The screen's SDP answer"}
          }
        }
      },
      %{
        name: "stop_idle",
        description:
          "Stop every running emulator, anyone's, that has gone unused for #{@idle_hours} hours, on each machine. It's run every hour, and gives the ones it stopped.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            stopped: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{machine: %{type: ["string", "null"]}, name: %{type: "string"}},
                required: ["machine", "name"]
              }
            }
          },
          required: ["stopped"]
        }
      }
    ]
  end

  defp one(properties \\ %{}, required \\ []) do
    %{type: "object", required: ["emulator" | required], properties: Map.put(properties, :emulator, @emulator)}
  end

  @impl true
  def call("list", _inputs, call), do: %{emulators: listed(call)}

  def call("machines", _inputs, call) do
    machines =
      for server <- Service.servers(call, Emulator) do
        case Service.seen(server) do
          at when is_integer(at) -> %{machine: Service.name(server), on: false, last_on: at}
          _ -> %{machine: Service.name(server), on: true}
        end
      end

    %{machines: machines}
  end

  def call("create", %{"name" => name} = inputs, call) do
    if Enum.any?(emulators(call), &(&1["name"] == name)), do: raise("You have an emulator called #{name} already")
    servers = Service.servers(call, Emulator)
    if servers == [], do: raise("No computer runs Android emulators yet. Ask your network's admin to set one up.\n\n`install_service` runs AndroidEmulator.Emulator on one.")
    server = if inputs["machine"], do: machine(servers, inputs["machine"]), else: hd(servers)
    id = "#{name}-#{Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)}"

    spec = %{
      "id" => id,
      "name" => name,
      "description" => inputs["description"],
      "artifact" => Artifact.name(Call.make(call, id)),
      "by" => call.actor || call.subject
    }

    Service.call(call, Emulator, :create, [spec], server)
    Enum.find(listed(call), &(&1["name"] == name))
  end

  def call(tool, %{"emulator" => name} = inputs, call) when tool in ~w(start stop remove) do
    on(call, inputs, String.to_existing_atom(tool))
    %{emulator: name}
  end

  def call("open_url", %{"url" => url} = inputs, call) do
    on(call, inputs, :open_url, [url])
    %{emulator: inputs["emulator"], url: url}
  end

  def call("install", inputs, call) do
    apk =
      case inputs do
        %{"apk" => upload} ->
          {:ok, data, _tag} = Artifact.read(call.own, upload)
          Artifact.remove(call.own, upload)
          %{"data" => Base.encode64(data)}

        %{"url" => url} ->
          %{"url" => url}

        _ ->
          raise "Give the APK, uploaded, or its url"
      end

    %{emulator: inputs["emulator"], said: on(call, inputs, :install, [apk])}
  end

  def call("shell", %{"command" => command} = inputs, call), do: on(call, inputs, :shell, [command, Map.get(inputs, "timeout", 60)])

  def call("screenshot", inputs, call), do: {:blob, "image/png", Base.decode64!(on(call, inputs, :screenshot))}

  def call("screen", %{"action" => "screenshot"} = inputs, call), do: call("screenshot", inputs, call)
  def call("screen", %{"action" => "resize"}, _call), do: raise("A phone's screen keeps its size")

  # A stream's ICE servers, for the app, then, with its offer, the sender on the emulator's machine started with its
  # own, and its answer. Both are given only to whoever reaches the emulator, as everything here is.
  def call("screen", %{"action" => "stream"} = inputs, call) do
    emulator!(call, inputs)
    ice_servers = Call.tool(call, "webrtc", "ice_servers", %{})

    case inputs["offer"] do
      nil -> ice_servers
      offer -> %{"answer" => on(call, inputs, :stream, [JSON.encode!(Map.put(ice_servers, "offer", offer))])}
    end
  end

  def call("screen", %{"action" => "move"}, _call), do: %{done: "A phone has no pointer to move: nothing to do"}

  def call("screen", %{"action" => action} = inputs, call) do
    for args <- input(action, inputs), do: on(call, inputs, :input, [args])
    %{done: did(action, inputs)}
  end

  def call("stop_idle", _inputs, call) do
    stopped =
      for server <- Service.servers(call, Emulator),
          not is_integer(Service.seen(server)),
          name <- Service.call(call, Emulator, :stop_idle, [@idle_hours * 3_600_000], server),
          do: %{machine: Service.name(server), name: name}

    %{stopped: stopped}
  end

  @doc false
  # An emulator's state changed: every app watching is told. For `AndroidEmulator.Emulator`.
  def changed(call), do: Call.changed(call)

  # Computer Use asks for the caller's screens: each running emulator of theirs is one, called what it is, and acted
  # on with `screen`.
  @impl true
  def hook("computer-use.screens", _about, call) do
    for server <- Service.servers(call, Emulator), s <- each(call, server, :screens), Call.reaches?(call, s["artifact"]) do
      %{
        "name" => s["name"],
        "description" => "#{s["description"]}, an Android emulator on #{Service.name(server)}",
        "width" => s["width"],
        "height" => s["height"],
        "tool" => "screen",
        "inputs" => %{"emulator" => s["name"]}
      }
    end
  end

  # Computer Use's actions, as Android's `input` takes them: each a list of its words, run one after another.
  defp input("click", %{"x" => x, "y" => y, "button" => "right"}), do: [~w(swipe #{x} #{y} #{x} #{y} 800)]
  defp input("click", %{"button" => "middle"}), do: [~w(keyevent KEYCODE_HOME)]
  defp input("click", %{"x" => x, "y" => y}), do: [~w(tap #{x} #{y})]
  defp input("double_click", %{"x" => x, "y" => y}), do: [~w(tap #{x} #{y}), ~w(tap #{x} #{y})]
  defp input("drag", %{"path" => [%{"x" => x, "y" => y}, %{"x" => x2, "y" => y2}]}), do: [~w(swipe #{x} #{y} #{x2} #{y2} 400)]

  defp input("drag", %{"path" => [first | rest]}) do
    [~w(motionevent DOWN #{first["x"]} #{first["y"]})] ++
      Enum.map(rest, &~w(motionevent MOVE #{&1["x"]} #{&1["y"]})) ++ [~w(motionevent UP #{List.last(rest)["x"]} #{List.last(rest)["y"]})]
  end

  # A scroll is a swipe the other way, as a finger scrolls, a fifth of a phone's height for each step.
  defp input("scroll", %{"x" => x, "y" => y, "direction" => direction} = inputs) do
    far = 160 * (inputs["amount"] || 5)
    {dx, dy} = %{"up" => {0, far}, "down" => {0, -far}, "left" => {far, 0}, "right" => {-far, 0}}[direction]
    [~w(swipe #{x} #{y} #{max(x + dx, 0)} #{max(y + dy, 0)} 500)]
  end

  # Text as Android's `input text` takes it, which reads %s as a space.
  defp input("type", %{"text" => text}), do: [["text", String.replace(text, " ", "%s")]]
  defp input("key", %{"keys" => keys}), do: Enum.map(String.split(keys), &key/1)
  defp input(action, _inputs), do: raise("#{action} needs more: Computer Use's tools say what each action takes")

  # A key or a combination, as xdotool names them (ctrl+a, Return), as Android's key codes: one alone is a key
  # event, more a combination, held together.
  @keycodes %{
    "return" => "ENTER", "enter" => "ENTER", "backspace" => "DEL", "delete" => "FORWARD_DEL", "escape" => "BACK",
    "tab" => "TAB", "space" => "SPACE", "up" => "DPAD_UP", "down" => "DPAD_DOWN", "left" => "DPAD_LEFT",
    "right" => "DPAD_RIGHT", "home" => "MOVE_HOME", "end" => "MOVE_END", "prior" => "PAGE_UP", "page_up" => "PAGE_UP",
    "next" => "PAGE_DOWN", "page_down" => "PAGE_DOWN", "ctrl" => "CTRL_LEFT", "control" => "CTRL_LEFT",
    "shift" => "SHIFT_LEFT", "alt" => "ALT_LEFT", "super" => "META_LEFT", "back" => "BACK", "menu" => "MENU",
    "app_switch" => "APP_SWITCH"
  }

  defp key(combination) do
    codes =
      for name <- String.split(combination, "+") do
        lower = String.downcase(name)
        "KEYCODE_" <> (@keycodes[lower] || @keycodes[String.replace(lower, ~r/_[lr]$/, "")] || String.upcase(String.replace_prefix(lower, "keycode_", "")))
      end

    if length(codes) == 1, do: ["keyevent" | codes], else: ["keycombination" | codes]
  end

  defp did("click", %{"x" => x, "y" => y, "button" => "right"}), do: "Long-pressed at #{x}, #{y}"
  defp did("click", %{"button" => "middle"}), do: "Pressed Home"
  defp did("click", %{"x" => x, "y" => y}), do: "Tapped at #{x}, #{y}"
  defp did("double_click", %{"x" => x, "y" => y}), do: "Tapped twice at #{x}, #{y}"
  defp did("drag", %{"path" => path}), do: "Swiped through #{length(path)} points"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}"
  defp did("type", %{"text" => text}), do: "Typed #{String.length(text)} characters"
  defp did("key", %{"keys" => keys}), do: "Pressed #{keys}"

  # Runs a function of the service on the emulator called `inputs["emulator"]`, where it is.
  defp on(call, inputs, function, arguments \\ []) do
    e = emulator!(call, inputs)
    Service.call(call, Emulator, function, [e["id"] | arguments], e["server"])
  end

  defp emulator!(call, %{"emulator" => name}),
    do: Enum.find(emulators(call), &(&1["name"] == name)) || raise("You have no emulator called #{name}")

  # The emulators this call may use, on every machine that's up.
  defp listed(call), do: Enum.map(emulators(call), &Map.drop(&1, ["server", "artifact", "id"]))

  defp emulators(call) do
    for server <- Service.servers(call, Emulator),
        e <- each(call, server, :list),
        Call.reaches?(call, e["artifact"]),
        do: Map.merge(e, %{"server" => server, "machine" => Service.name(server)})
  end

  # A machine that's known to be off has nothing to list now: `machines` says which.
  defp each(call, server, function) do
    if is_integer(Service.seen(server)), do: [], else: Service.call(call, Emulator, function, [], server)
  end

  defp machine(servers, name) do
    Enum.find(servers, &(String.downcase(Service.name(&1) || "") == String.downcase(name))) ||
      raise "No machine called #{name} runs emulators. They're on #{Enum.map_join(servers, ", ", &Service.name/1)}."
  end
end
