defmodule IphoneSimulator do
  @moduledoc """
  iPhone Simulator: iOS Simulators, Xcode's, on the Macs that run its service, `IphoneSimulator.Simulator`.

  A simulator is an artifact made for whoever created it (`OneCore.Call.make/2`), so who sees and uses one is what
  `reach` says: its creator, whoever they share it with, and the network's owner. A job stops any left running and
  unused for longer than two hours, whoever's they are: what it saves is the machine's.
  """
  @behaviour OneCore.App
  alias IphoneSimulator.Simulator
  alias OneCore.{Artifact, Call, Service}

  @idle_hours 2
  @simulator %{type: "string", description: "the simulator, by its name"}

  # One, as `list` answers each.
  @listed %{
    type: "object",
    properties: %{
      name: %{type: "string"},
      description: %{type: ["string", "null"], description: "What it's for"},
      state: %{type: "string", enum: ~w(stopped starting running stopping removing gone)},
      step: %{type: ["string", "null"], enum: ["booting", "connecting", nil], description: "While it's starting: booting iOS, or connecting to its screen"},
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
      "database" => "to keep your simulators",
      "webrtc" => "to show a simulator's screen live"
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
          "The simulators you can use, on every Mac: each one's name, model, iOS version, state (stopped, starting, running, stopping, removing, gone), machine, what it's for, who created it, and when it was created and last used (ms since 1970).",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{simulators: %{type: "array", items: @listed}}, required: ["simulators"]}
      },
      %{
        name: "machines",
        read_only: true,
        description: "The Macs that run simulators, each by the name `create` takes, whether it's on now, and if not when it last was (ms since 1970).",
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
          "Create an iOS Simulator, the newest iPhone Pro on the newest iOS the Mac's Xcode has, and boot it, which takes a minute or so; the first on a Mac installs idb first. Gives it as `list` does.",
        inputs: %{
          type: "object",
          required: ["name", "description"],
          properties: %{
            name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "its name, unique among yours"},
            description: %{type: "string", description: "what it's for, in a few words, for whoever sees it later"},
            machine: %{type: "string", description: "the Mac to run it on, as `machines` names it; default the first"}
          }
        },
        outputs: @listed
      },
      %{name: "start", description: "Boot a stopped simulator. Its apps and files are as they were; what was open isn't.", inputs: one(),
      outputs: %{type: "object", properties: %{simulator: %{type: "string"}}, required: ["simulator"]}},
      %{name: "stop", description: "Shut a simulator down. Its apps and files stay, and `start` boots it again.", inputs: one(),
      outputs: %{type: "object", properties: %{simulator: %{type: "string"}}, required: ["simulator"]}},
      %{name: "remove", description: "Remove a simulator and everything on it, for good.", inputs: one(),
      outputs: %{type: "object", properties: %{simulator: %{type: "string"}}, required: ["simulator"]}},
      %{
        name: "install",
        description: "Install an app built for the simulator on a running one: its .app, zipped (`ditto -c -k --keepParent My.app My.zip`), uploaded or by its url.",
        inputs:
          one(%{
            app: %{type: "string", format: "upload", description: "the zipped .app, uploaded"},
            url: %{type: "string", description: "or a URL to download the zipped .app from"}
          }),
        outputs: %{type: "object", properties: %{simulator: %{type: "string"}, said: %{type: "string", description: "What installing it said"}}, required: ["simulator", "said"]}
      },
      %{
        name: "open_url",
        description: "Open a URL on a running simulator, in the app for it or Safari.",
        inputs: one(%{url: %{type: "string", description: "the URL, like https://example.com or myapp://home"}}, ["url"]),
        outputs: %{type: "object", properties: %{simulator: %{type: "string"}, url: %{type: "string"}}, required: ["simulator", "url"]}
      },
      %{
        name: "screenshot",
        read_only: true,
        description: "A picture of a running simulator's screen, as a PNG, in its own pixels.",
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
          "Look at or act on a simulator's screen, for Computer Use, whose tools are how agents use a screen: they say what each action takes. A click is a tap, a right click a long press, a middle click the Home button, a drag a swipe, and keys are as xdotool names them (super is ⌘). `stream` is for a person's app to show it as live video over WebRTC: with no `offer`, the ICE servers to connect with; with the app's whole SDP `offer`, {answer}, the screen's.",
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
          "Shut down every running simulator, anyone's, that has gone unused for #{@idle_hours} hours, on each Mac. It's run every hour, and gives the ones it shut down.",
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
    %{type: "object", required: ["simulator" | required], properties: Map.put(properties, :simulator, @simulator)}
  end

  @impl true
  def call("list", _inputs, call), do: %{simulators: listed(call)}

  def call("machines", _inputs, call) do
    machines =
      for server <- Service.servers(call, Simulator) do
        case Service.seen(server) do
          at when is_integer(at) -> %{machine: Service.name(server), on: false, last_on: at}
          _ -> %{machine: Service.name(server), on: true}
        end
      end

    %{machines: machines}
  end

  def call("create", %{"name" => name} = inputs, call) do
    if Enum.any?(simulators(call), &(&1["name"] == name)), do: raise("You have a simulator called #{name} already")
    servers = Service.servers(call, Simulator)
    if servers == [], do: raise("No Mac runs simulators yet. Ask your network's admin to set one up.\n\n`install_service` runs IphoneSimulator.Simulator on one.")
    server = if inputs["machine"], do: machine(servers, inputs["machine"]), else: hd(servers)
    id = Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)

    spec = %{
      "name" => name,
      "description" => inputs["description"],
      "artifact" => Artifact.name(Call.make(call, id)),
      "by" => call.actor || call.subject
    }

    Service.call(call, Simulator, :create, [spec], server)
    Enum.find(listed(call), &(&1["name"] == name))
  end

  def call(tool, %{"simulator" => name} = inputs, call) when tool in ~w(start stop remove) do
    on(call, inputs, String.to_existing_atom(tool))
    %{simulator: name}
  end

  def call("open_url", %{"url" => url} = inputs, call) do
    on(call, inputs, :open_url, [url])
    %{simulator: inputs["simulator"], url: url}
  end

  def call("install", inputs, call) do
    app =
      case inputs do
        %{"app" => upload} ->
          {:ok, data, _tag} = Artifact.read(call.own, upload)
          Artifact.remove(call.own, upload)
          %{"data" => Base.encode64(data)}

        %{"url" => url} ->
          %{"url" => url}

        _ ->
          raise "Give the zipped .app, uploaded, or its url"
      end

    %{simulator: inputs["simulator"], said: on(call, inputs, :install, [app])}
  end

  def call("screenshot", inputs, call), do: {:blob, "image/png", Base.decode64!(on(call, inputs, :screenshot))}

  def call("screen", %{"action" => "screenshot"} = inputs, call), do: call("screenshot", inputs, call)
  def call("screen", %{"action" => "resize"}, _call), do: raise("An iPhone's screen keeps its size")

  # A stream's ICE servers, for the app, then, with its offer, the sender on the simulator's Mac started with its own,
  # and its answer. Both are given only to whoever reaches the simulator, as everything here is.
  def call("screen", %{"action" => "stream"} = inputs, call) do
    simulator!(call, inputs)
    ice_servers = Call.tool(call, "webrtc", "ice_servers", %{})

    case inputs["offer"] do
      nil -> ice_servers
      offer -> %{"answer" => on(call, inputs, :stream, [JSON.encode!(Map.put(ice_servers, "offer", offer))])}
    end
  end

  def call("screen", %{"action" => "move"}, _call), do: %{done: "An iPhone has no pointer to move: nothing to do"}

  def call("screen", %{"action" => action} = inputs, call) do
    on(call, inputs, :act, [action, Map.drop(inputs, ["simulator", "action"])])
    %{done: did(action, inputs)}
  end

  def call("stop_idle", _inputs, call) do
    stopped =
      for server <- Service.servers(call, Simulator),
          not is_integer(Service.seen(server)),
          name <- Service.call(call, Simulator, :stop_idle, [@idle_hours * 3_600_000], server),
          do: %{machine: Service.name(server), name: name}

    %{stopped: stopped}
  end

  # Computer Use asks for the caller's screens: each running simulator of theirs is one, called what it is, and acted
  # on with `screen`.
  @doc false
  # A simulator's state changed: every app watching is told. For `IphoneSimulator.Simulator`.
  def changed(call), do: Call.changed(call)

  @impl true
  def hook("computer-use.screens", _about, call) do
    for server <- Service.servers(call, Simulator), s <- each(call, server, :screens), Call.reaches?(call, s["artifact"]) do
      %{
        "name" => s["name"],
        "description" => "#{s["description"]}, an #{s["model"]} simulator on #{Service.name(server)}",
        "width" => s["width"],
        "height" => s["height"],
        "tool" => "screen",
        "inputs" => %{"simulator" => s["name"]}
      }
    end
  end

  defp did("click", %{"x" => x, "y" => y, "button" => "right"}), do: "Long-pressed at #{x}, #{y}"
  defp did("click", %{"button" => "middle"}), do: "Pressed Home"
  defp did("click", %{"x" => x, "y" => y}), do: "Tapped at #{x}, #{y}"
  defp did("double_click", %{"x" => x, "y" => y}), do: "Tapped twice at #{x}, #{y}"
  defp did("drag", %{"path" => path}), do: "Swiped through #{length(path)} points"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}"
  defp did("type", %{"text" => text}), do: "Typed #{String.length(text)} characters"
  defp did("key", %{"keys" => keys}), do: "Pressed #{keys}"

  # Runs a function of the service on the simulator called `inputs["simulator"]`, where it is.
  defp on(call, inputs, function, arguments \\ []) do
    s = simulator!(call, inputs)
    Service.call(call, Simulator, function, [s["udid"] | arguments], s["server"])
  end

  defp simulator!(call, %{"simulator" => name}),
    do: Enum.find(simulators(call), &(&1["name"] == name)) || raise("You have no simulator called #{name}")

  # The simulators this call may use, on every Mac that's up.
  defp listed(call), do: Enum.map(simulators(call), &Map.drop(&1, ["server", "artifact", "udid"]))

  defp simulators(call) do
    for server <- Service.servers(call, Simulator),
        e <- each(call, server, :list),
        Call.reaches?(call, e["artifact"]),
        do: Map.merge(e, %{"server" => server, "machine" => Service.name(server)})
  end

  # A machine that's known to be off has nothing to list now: `machines` says which.
  defp each(call, server, function) do
    if is_integer(Service.seen(server)), do: [], else: Service.call(call, Simulator, function, [], server)
  end

  defp machine(servers, name) do
    Enum.find(servers, &(String.downcase(Service.name(&1) || "") == String.downcase(name))) ||
      raise "No Mac called #{name} runs simulators. They're on #{Enum.map_join(servers, ", ", &Service.name/1)}."
  end
end
