defmodule Containers do
  @moduledoc """
  Containers: Linux containers on the machines that run its service, `Containers.Podman`, through Podman.

  A container is an artifact made for whoever created it (`OneCore.Call.make/2`), and its name is a label on the
  container, so who sees and uses one is what `reach` says: its creator, whoever they share it with, and the
  network's owner. An image made with `commit` is one too. A job stops any left running and unused for longer than
  the setting `idle_hours`, whoever's they are: what it saves is the machine's.
  """
  @behaviour OneCore.Plugin
  alias Containers.Podman
  alias OneCore.{Artifact, Call, Service}

  @idle_hours 4
  @container %{type: "string", description: "the container, by its name"}

  @impl true
  def uses, do: ["database"]

  @impl true
  def jobs, do: [%{tool: "stop_idle", every: 3600}]

  @impl true
  def hooks, do: ["computer-use.screens"]

  @impl true
  def apps, do: [%{name: "Containers", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "The containers you can use, on every machine: each one's name, image, state (running, exited, created), machine, size, what it's for, who created it, when it was created and last used (ms since 1970), and its ports, each published on the machine's own loopback.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "machines",
        read_only: true,
        description: "The machines that run containers, each by the name `create` takes, whether it's on now, and if not when it last was (ms since 1970).",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "images",
        read_only: true,
        description:
          "The images on each machine to create a container from: the ones you made with `commit`, by their names, and the ones pulled there. Any other image is pulled from its registry when a container is created from it.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "create",
        description:
          "Create a container and start it. It gets no files or network of its machine's, and no privileges: only its own, with the limits of its size, and the ports asked for. Creating from an image not on the machine pulls it first, and `desktop` is built there the first time (about 2 minutes); after that it's a second. Gives the container as `list` does.",
        inputs: %{
          type: "object",
          required: ["image", "name", "description"],
          properties: %{
            image: %{
              type: "string",
              description:
                ~s(the image: "desktop" for a Linux desktop with Chromium on a display you can screenshot, one you made with `commit`, or any from a registry, like "docker.io/library/ubuntu:24.04")
            },
            name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "its name, unique among yours"},
            description: %{type: "string", description: "what it's for, in a few words, for whoever sees it later"},
            size: %{
              type: "string",
              enum: ["small", "medium", "large"],
              description: "small is 1 CPU and 1 GB of memory (the default), medium 2 and 4 GB, large 4 and 8 GB"
            },
            machine: %{type: "string", description: "the machine to run it on, as `machines` names it; default the first"},
            ports: %{
              type: "array",
              items: %{type: "integer", minimum: 1, maximum: 65_535},
              description: "ports of the container to reach with `request`"
            },
            command: %{
              type: "array",
              items: %{type: "string"},
              description: "the command to run in place of the image's own, as its words; a shell stays running"
            },
            env: %{type: "object", additionalProperties: %{type: "string"}, description: "environment variables"},
            mounts: %{
              type: "array",
              description:
                "folders of the machine itself to put in the container. They're the files of the machine's owner, which the container can then change: only when asked for",
              items: %{
                type: "object",
                required: ["source", "target"],
                properties: %{
                  source: %{type: "string", description: "the folder on the machine, an absolute path"},
                  target: %{type: "string", description: "where it is in the container"},
                  read_only: %{type: "boolean"}
                }
              }
            }
          }
        }
      },
      %{name: "start", description: "Start a stopped container.", inputs: one()},
      %{
        name: "stop",
        description: "Stop a container. Its files stay, and `start` starts it again.",
        inputs: one()
      },
      %{name: "remove", description: "Remove a container and its files, for good.", inputs: one()},
      %{
        name: "exec",
        description:
          "Run a command in a running container with sh -c, as its user (root, in most images), and wait for it. Gives { exit, stdout, stderr }, the last 64 KB of each. Once its timeout passes it's stopped waiting for and exit is -1, but what it started may carry on in the container.",
        inputs:
          one(%{
            command: %{type: "string", description: ~s(the command line, like "apt-get install -y git"; it reads no input)},
            dir: %{type: "string", description: "the folder to run it in; default the image's"},
            timeout: %{type: "integer", minimum: 1, maximum: 3600, description: "seconds to wait, 1 to 3600; default 60"}
          }, ["command"])
      },
      %{
        name: "logs",
        read_only: true,
        description: "What the container's own command printed: its last lines.",
        inputs: one(%{tail: %{type: "integer", minimum: 1, description: "how many lines; default 200"}})
      },
      %{
        name: "read_file",
        read_only: true,
        description:
          "Read a file of a container, running or stopped: text as text, anything else as the file (a picture is seen). Up to 16 MB.",
        inputs: one(%{path: %{type: "string", description: "its absolute path in the container"}}, ["path"])
      },
      %{
        name: "write_file",
        description: "Write a file into a container, replacing it if it's there. Its folder must be there already.",
        inputs:
          one(
            %{
              path: %{type: "string", description: "its absolute path in the container"},
              text: %{type: "string", description: "what's in it, as text"},
              data: %{type: "string", contentEncoding: "base64", description: "or what's in it, as base64"}
            },
            ["path"]
          )
      },
      %{
        name: "commit",
        description:
          "Make an image of a container as it is now: its files, not what's running. `create` makes throwaway copies from it, on the same machine.",
        inputs:
          one(%{name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the image's name"}}, ["name"])
      },
      %{
        name: "remove_image",
        description: "Remove an image you made with `commit`, once no container uses it.",
        inputs: %{
          type: "object",
          required: ["name"],
          properties: %{name: %{type: "string", description: "the image's name"}}
        }
      },
      %{
        name: "request",
        description:
          "Make an HTTP request to a port the container was created with, from its machine, and give the response: { status, headers, body }, the body as text, up to 16 MB.",
        inputs:
          one(
            %{
              port: %{type: "integer", description: "the container's port"},
              path: %{type: "string", description: ~s(the path and query, like "/api/health"; default "/")},
              method: %{type: "string", enum: ~w(GET HEAD POST PUT PATCH DELETE OPTIONS), description: "default GET"},
              headers: %{type: "object", additionalProperties: %{type: "string"}},
              body: %{type: "string"}
            },
            ["port"]
          )
      },
      %{
        name: "screenshot",
        read_only: true,
        description:
          "A picture of a container's screen: the X display its DISPLAY names, as a PNG. A container of the image desktop has one, 1280×800, with Chromium on it; `exec` with xdotool moves, clicks and types there.",
        inputs: one()
      },
      %{
        name: "screen",
        description:
          "Look at or act on a container's screen, for Computer Use, whose tools are how agents use a screen: they say what each action takes.",
        inputs:
          one(
            %{
              action: %{type: "string", enum: ~w(screenshot click double_click move drag scroll type key)},
              x: %{type: "integer", minimum: 0},
              y: %{type: "integer", minimum: 0},
              button: %{type: "string", enum: ~w(left right middle)},
              path: %{type: "array", minItems: 2, items: %{type: "object", required: ["x", "y"], properties: %{x: %{type: "integer"}, y: %{type: "integer"}}}},
              direction: %{type: "string", enum: ~w(up down left right)},
              amount: %{type: "integer", minimum: 1, maximum: 50},
              text: %{type: "string"},
              keys: %{type: "string"}
            },
            ["action"]
          )
      },
      %{
        name: "settings",
        description:
          "The settings: idle_hours, how long a running container goes unused before it's stopped (default #{@idle_hours}). Gives them; with one given, its owner changes it.",
        inputs: %{
          type: "object",
          properties: %{idle_hours: %{type: "integer", minimum: 1, description: "hours a container may go unused"}}
        }
      },
      %{
        name: "stop_idle",
        description:
          "Stop every running container, anyone's, that has gone unused longer than idle_hours, on each machine. It's run every hour, and gives the ones it stopped.",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  defp one(properties \\ %{}, required \\ []) do
    %{type: "object", required: ["container" | required], properties: Map.put(properties, :container, @container)}
  end

  @impl true
  def call("list", _inputs, call), do: Enum.map(containers(call), &Map.drop(&1, [:server, :artifact, :id]))

  def call("machines", _inputs, call) do
    for server <- Service.servers(call, Podman) do
      case Service.seen(server) do
        at when is_integer(at) -> %{machine: Service.name(server), on: false, last_on: at}
        _ -> %{machine: Service.name(server), on: true}
      end
    end
  end

  def call("images", _inputs, call), do: Enum.map(images(call), &Map.drop(&1, [:server, :artifact, :id]))

  def call("create", inputs, call) do
    name = inputs["name"]
    if Enum.any?(containers(call), &(&1.name == name)), do: raise("You have a container called #{name} already")
    servers = Service.servers(call, Podman)
    if servers == [], do: raise("No machine runs containers yet: `install_service` runs Containers.Podman on one.")
    {image, server} = image(call, inputs["image"], inputs["machine"] && machine(servers, inputs["machine"]))
    server = server || hd(servers)

    id = Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)
    artifact = Artifact.name(Call.make(call, id))

    spec = %{
      "id" => "#{name}-#{id}",
      "image" => image,
      "size" => inputs["size"] || "small",
      "ports" => inputs["ports"],
      "command" => inputs["command"],
      "env" => inputs["env"],
      "mounts" => inputs["mounts"],
      "labels" => %{
        "artifact" => artifact,
        "name" => name,
        "description" => inputs["description"],
        "size" => inputs["size"] || "small",
        "by" => call.actor || call.subject
      }
    }

    Service.call(call, Podman, :create, [spec], server)
    call("list", %{}, call) |> Enum.find(&(&1.name == name))
  end

  def call("start", inputs, call), do: on(call, inputs, :start)
  def call("stop", inputs, call), do: on(call, inputs, :stop)
  def call("remove", inputs, call), do: on(call, inputs, :remove)
  def call("logs", inputs, call), do: on(call, inputs, :logs, [Map.get(inputs, "tail", 200)])

  def call("exec", inputs, call) do
    if String.trim(inputs["command"]) == "", do: raise("Give a command")
    on(call, inputs, :exec, [inputs["command"], inputs["dir"], Map.get(inputs, "timeout", 60)])
  end

  def call("read_file", %{"path" => path} = inputs, call) do
    data = Base.decode64!(on(call, inputs, :read_file, [path]))

    if String.valid?(data) and not String.contains?(data, <<0>>),
      do: data,
      else: {:blob, MIME.from_path(path), data}
  end

  def call("write_file", %{"path" => path} = inputs, call) do
    data =
      case inputs do
        %{"text" => text} -> Base.encode64(text)
        %{"data" => data} -> data
        _ -> raise "Give its text, or its data as base64"
      end

    on(call, inputs, :write_file, [path, data])
    "Wrote #{path}"
  end

  def call("commit", %{"name" => name} = inputs, call) do
    if Enum.any?(images(call), &(&1.artifact && &1.name == name)), do: raise("You have an image called #{name} already")
    id = Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)
    labels = %{"artifact" => Artifact.name(Call.make(call, "image-" <> id)), "name" => name}
    on(call, inputs, :commit, ["localhost/#{name}-#{id}", labels])
    "Made the image #{name}: `create` with image #{name} makes a copy"
  end

  def call("remove_image", %{"name" => name}, call) do
    image = Enum.find(images(call), &(&1.artifact && &1.name == name)) || raise "You have no image called #{name}"
    Service.call(call, Podman, :remove_image, [image.id], image.server)
    "Removed the image #{name}"
  end

  def call("request", inputs, call) do
    request = [
      inputs["port"],
      Map.get(inputs, "method", "GET"),
      Map.get(inputs, "path", "/"),
      Map.get(inputs, "headers", %{}),
      Map.get(inputs, "body", "")
    ]

    on(call, inputs, :request, request)
  end

  def call("screenshot", inputs, call), do: {:blob, "image/png", Base.decode64!(on(call, inputs, :screenshot))}

  def call("screen", %{"action" => "screenshot"} = inputs, call), do: call("screenshot", inputs, call)

  def call("screen", %{"action" => action} = inputs, call) do
    on(call, inputs, :xdotool, [xdotool(action, inputs)])
    did(action, inputs)
  end

  def call("settings", inputs, call) do
    settings = settings(call)

    case inputs do
      %{"idle_hours" => hours} ->
        # The settings are whoever's it was installed for, so it's they who must be owned, not whoever it runs for.
        if not Call.owns?(%{call | subject: call.installed}), do: raise("Only its owner changes Containers' settings")
        changed = Map.put(settings, "idle_hours", hours)

        case Artifact.read(call.own, "settings") do
          {:ok, _data, tag} -> Artifact.replace(call.own, "settings", JSON.encode!(changed), tag)
          :absent -> Artifact.write(call.own, "settings", JSON.encode!(changed))
        end

        changed

      _ ->
        settings
    end
  end

  def call("stop_idle", _inputs, call) do
    idle = settings(call)["idle_hours"] * 3_600_000

    for server <- Service.servers(call, Podman),
        not is_integer(Service.seen(server)),
        name <- Service.call(call, Podman, :stop_idle, [idle], server),
        do: %{machine: Service.name(server), name: name}
  end

  # Computer Use asks for the caller's screens: each running container of theirs with a display is one, called what
  # the container is, and acted on with `screen`.
  @impl true
  def hook("computer-use.screens", _about, call) do
    screens =
      for server <- Service.servers(call, Podman), s <- each(call, server, :screens), reaches?(call, s) do
        %{
          "name" => s["name"],
          "description" => "#{s["description"]}, a container on #{Service.name(server)}",
          "width" => s["width"],
          "height" => s["height"],
          "tool" => "screen",
          "inputs" => %{"container" => s["name"]}
        }
      end

    %{call.plugin => screens}
  end

  # Computer Use's actions, as xdotool's words.
  @buttons %{"left" => "1", "middle" => "2", "right" => "3"}
  @wheel %{"up" => "4", "down" => "5", "left" => "6", "right" => "7"}

  defp xdotool("click", %{"x" => x, "y" => y} = inputs),
    do: ~w(mousemove #{x} #{y} click #{@buttons[inputs["button"] || "left"]})

  defp xdotool("double_click", %{"x" => x, "y" => y}), do: ~w(mousemove #{x} #{y} click --repeat 2 --delay 80 1)
  defp xdotool("move", %{"x" => x, "y" => y}), do: ~w(mousemove #{x} #{y})

  defp xdotool("drag", %{"path" => [%{"x" => x, "y" => y} | rest]}),
    do: ~w(mousemove #{x} #{y} mousedown 1) ++ Enum.flat_map(rest, &~w(mousemove #{&1["x"]} #{&1["y"]})) ++ ~w(mouseup 1)

  defp xdotool("scroll", %{"x" => x, "y" => y, "direction" => direction} = inputs),
    do: ~w(mousemove #{x} #{y} click --repeat #{inputs["amount"] || 5} --delay 30 #{@wheel[direction]})

  defp xdotool("type", %{"text" => text}), do: ["type", "--delay", "8", "--", text]
  defp xdotool("key", %{"keys" => keys}), do: ["key", "--delay", "50", "--" | String.split(keys)]
  defp xdotool(action, _inputs), do: raise("#{action} needs more: Computer Use's tools say what each action takes")

  defp did("click", %{"x" => x, "y" => y} = inputs), do: "Clicked #{inputs["button"] || "left"} at #{x}, #{y}"
  defp did("double_click", %{"x" => x, "y" => y}), do: "Double-clicked at #{x}, #{y}"
  defp did("move", %{"x" => x, "y" => y}), do: "Moved to #{x}, #{y}"
  defp did("drag", %{"path" => path}), do: "Dragged through #{length(path)} points"
  defp did("scroll", %{"direction" => direction} = inputs), do: "Scrolled #{direction} #{inputs["amount"] || 5}"
  defp did("type", %{"text" => text}), do: "Typed #{String.length(text)} characters"
  defp did("key", %{"keys" => keys}), do: "Pressed #{keys}"

  defp settings(call) do
    case Artifact.read(call.own, "settings") do
      {:ok, data, _tag} -> Map.merge(%{"idle_hours" => @idle_hours}, JSON.decode!(data))
      :absent -> %{"idle_hours" => @idle_hours}
    end
  end

  # The image to create from, and the machine it's on: one of the caller's own made with `commit`, by its name or
  # Podman's, or any other, which Podman pulls, but never another's made with `commit`.
  defp image(call, image, server) do
    all = images(call, :all)
    named? = &(image in [&1.name | &1.tags] or image =~ ~r/^(sha256:)?[0-9a-f]{3,}$/ and String.contains?(&1.id, image))

    case Enum.find(all, &(&1.artifact && named?.(&1) && reaches?(call, &1))) do
      %{server: at} = mine when server in [nil, at] ->
        {mine.id, at}

      %{server: at} ->
        raise "The image #{image} is on #{Service.name(at)}: create it there"

      nil ->
        if Enum.any?(all, &(&1.artifact && named?.(&1))), do: raise("You have no image called #{image}")
        {image, server}
    end
  end

  # Runs a function of the service on the container called `inputs["container"]`, where it is.
  defp on(call, inputs, function, arguments \\ []) do
    name = inputs["container"]
    c = Enum.find(containers(call), &(&1.name == name)) || raise "You have no container called #{name}"
    answer = Service.call(call, Podman, function, [c.id | arguments], c.server)
    if answer == "ok", do: "#{String.capitalize(to_string(function))}: #{name}", else: answer
  end

  # The containers this call may use, on every machine that's up.
  defp containers(call) do
    for server <- Service.servers(call, Podman),
        c <- each(call, server, :list),
        reaches?(call, c),
        do: Map.merge(Map.new(c, fn {key, value} -> {String.to_atom(key), value} end), %{server: server, machine: Service.name(server)})
  end

  # The images this call may create from, or `:all` of them, with whose each is, on every machine that's up.
  defp images(call, which \\ :reached) do
    for server <- Service.servers(call, Podman),
        i <- each(call, server, :images),
        which == :all or i["artifact"] == nil or reaches?(call, i),
        do: Map.merge(Map.new(i, fn {key, value} -> {String.to_atom(key), value} end), %{server: server, machine: Service.name(server)})
  end

  # A machine that's known to be off has nothing to list now: `machines` says which.
  defp each(call, server, function) do
    if is_integer(Service.seen(server)), do: [], else: Service.call(call, Podman, function, [], server)
  end

  defp reaches?(call, %{artifact: artifact}), do: Call.reaches?(call, artifact)
  defp reaches?(call, %{"artifact" => artifact}), do: Call.reaches?(call, artifact)

  defp machine(servers, name) do
    Enum.find(servers, &(String.downcase(Service.name(&1) || "") == String.downcase(name))) ||
      raise "No machine called #{name} runs containers. It's on #{Enum.map_join(servers, ", ", &Service.name/1)}."
  end
end
