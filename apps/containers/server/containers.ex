defmodule Containers do
  @moduledoc """
  Containers: Linux containers on the machines that run its service, `Containers.Podman`, through Podman.

  A container is an artifact made for whoever created it (`OneCore.Call.make/2`), and its name is a label on the
  container, so who sees and uses one is what `reach` says: its creator, whoever they share it with, and the
  network's owner. An image made with `commit` is one too. A job stops any left running and unused for longer than
  the setting `idle_hours`, whoever's they are: what it saves is the machine's.

  A machine may run another network's server too, sharing its Podman: each server's service gives only what that
  server made, so a network sees and uses only its own. Each name is kept in the app's own artifact, as
  `names/<whose>/<name>`, with the machine its container is on, so it's taken while that machine is off too.
  """
  @behaviour OneCore.App
  alias Containers.Podman
  alias OneCore.{Artifact, Call, Service}

  @idle_hours 4
  @container %{type: "string", description: "the container, by its name"}
  # Where `upload` and `download` send a file or fetch it, and what it's sealed with (`Containers.Sealed`).
  @link %{
    type: "object",
    description: "A short-lived link to one blob, as OneCore.Artifact.link/3 gives it",
    properties: %{method: %{type: "string"}, url: %{type: "string"}, headers: %{type: "object", additionalProperties: %{type: "string"}}},
    required: ["method", "url", "headers"]
  }
  @key %{type: "string", contentEncoding: "base64", description: "The 32-byte key it's sealed with, made for this one blob"}

  @failed %{
    type: "object",
    additionalProperties: %{type: "string"},
    description: "Why each machine that didn't answer couldn't, by its name: what it has is left out"
  }

  # A container, as `list` answers each: on a machine that's off, only its name, its machine and when that was last on.
  @listed %{
    type: "object",
    properties: %{
      name: %{type: "string"},
      description: %{type: ["string", "null"], description: "What it's for"},
      image: %{type: "string"},
      state: %{type: "string", description: "Podman's: created, running, exited, removing, or unknown while its machine is off"},
      exit_code: %{type: "integer", description: "An exited one's, as its command ended"},
      finished: %{type: "integer", description: "When an exited one did, in milliseconds since 1970"},
      machine: %{type: ["string", "null"]},
      last_on: %{type: "integer", description: "When its machine, off now, was last on, in milliseconds since 1970"},
      size: %{type: ["string", "null"], description: "small, medium or large"},
      restart: %{type: "string", enum: ~w(no on-failure always), description: "Its restart policy"},
      by: %{type: ["string", "null"], description: "Who created it"},
      created: %{type: "integer", description: "In milliseconds since 1970"},
      last_used: %{type: "integer", description: "In milliseconds since 1970"},
      ports: %{
        type: "array",
        items: %{
          type: "object",
          properties: %{
            port: %{type: "integer", description: "The container's"},
            host_port: %{type: "integer", description: "Where it's published, on the machine's own loopback"}
          }
        }
      }
    },
    required: ~w(name state machine)
  }

  @named %{type: "object", properties: %{container: %{type: "string"}}, required: ["container"]}

  # What a command it ran gave.
  @ran %{
    type: "object",
    properties: %{
      exit: %{type: "integer", description: "0 when it worked, -1 once its timeout passed"},
      stdout: %{type: "string", description: "Its last 64 KB"},
      stderr: %{type: "string", description: "Its last 64 KB"}
    },
    required: ["exit", "stdout", "stderr"]
  }

  @impl true
  def icon, do: "icon.svg"

  # Its data is everyone's together: the network's settings, and each person's names for their containers.
  @impl true
  def own, do: :installed

  @impl true
  def requires do
    %{
      "core" => ">= 0.34.128",
      "database" => "to keep your containers",
      "webrtc" => "to show a container's screen live"
    }
  end

  @impl true
  def jobs, do: [%{tool: "stop_idle", every: 3600}]

  @impl true
  def hooks, do: ["computer-use.screens"]

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "The containers you can use, on every machine: each one's name, image, state as Podman says it (created, running, exited, with its exit code and when, or removing), machine, size, restart policy, what it's for, who created it, when it was created and last used (ms since 1970), and its ports, each published on the machine's own loopback. One on a machine that's off is unknown, with when its machine was last on. A machine that didn't answer is in `failed`, with why.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{containers: %{type: "array", items: @listed}, failed: @failed}, required: ["containers", "failed"]}
      },
      %{
        name: "machines",
        read_only: true,
        description:
          "The machines that run containers, each by the name `create` takes, whether it's on now, and if not when it last was (ms since 1970), and where it's on and has no desktop image yet, how far its build is. Each machine builds it by itself as Containers starts there, in about a minute, and again each week. One that's on and doesn't answer says why, as `failed`.",
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
                  last_on: %{type: "integer", description: "When it last was, if it's off, in milliseconds since 1970"},
                  desktop: %{
                    type: ["string", "null"],
                    description: ~s(How far the image desktop's build is, as a person reads it, while the machine has none yet: "Installing packages: 40%". Null otherwise.)
                  },
                  failed: %{type: "string", description: "Why it didn't answer, if it's on and didn't: it has no `desktop` then"}
                },
                required: ["machine", "on"]
              }
            }
          },
          required: ["machines"]
        }
      },
      %{
        name: "images",
        read_only: true,
        description:
          "The images on each machine to create a container from: the ones you made with `commit`, by their names, and the ones pulled there. Any other image is pulled from its registry when a container is created from it.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            images: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string", description: "What create's image takes"},
                  tags: %{type: "array", items: %{type: "string"}},
                  size: %{type: ["integer", "null"], description: "In bytes"},
                  created: %{type: "integer", description: "In milliseconds since 1970"},
                  machine: %{type: ["string", "null"]}
                },
                required: ["name", "tags", "size", "created", "machine"]
              }
            },
            failed: @failed
          },
          required: ["images", "failed"]
        }
      },
      %{
        name: "create",
        task_support: :optional,
        description:
          "Create a Linux container, from a Docker image or `desktop`, and start it. It gets no files or network of its machine's, and no privileges: only its own, with the limits of its size, and the ports asked for. It says each stage: pulling an image not on the machine, waiting for `desktop` to be built there where it isn't yet (each machine builds it as Containers starts there, in about a minute, and again each week in the background; `machines` says how far it is), then starting. Gives the container as `list` does.",
        inputs: %{
          type: "object",
          required: ["image", "name", "description"],
          properties: %{
            image: %{
              type: "string",
              description:
                ~s(the image: "desktop" for a Linux desktop with Google Chrome on a display you can screenshot, one you made with `commit`, or any from a registry, like "docker.io/library/ubuntu:24.04")
            },
            name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "its name, unique among yours"},
            description: %{type: "string", description: "what it's for, in a few words, for whoever sees it later"},
            size: %{
              type: "string",
              enum: ["small", "medium", "large"],
              description: "small is 1 CPU and 1 GB of memory (the default), medium 2 and 4 GB, large 4 and 8 GB"
            },
            restart: %{
              type: "string",
              enum: ~w(no on-failure always),
              description:
                "whether it starts again, as Docker's --restart: no (the default) leaves it stopped once its command ends; on-failure starts it again when its command fails; always starts it again whenever its command ends, and after the machine reboots. One stopped by hand, or for going unused, stays stopped until the machine reboots"
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
        },
        outputs: @listed
      },
      %{
        name: "start",
        description: "Start a stopped container.",
        inputs: one(),
        outputs: @named
      },
      %{
        name: "stop",
        description: "Stop a container. Its files stay, and `start` starts it again.",
        inputs: one(),
        outputs: @named
      },
      %{
        name: "remove",
        description: "Remove a container and its files, for good.",
        inputs: one(),
        outputs: @named
      },
      %{
        name: "exec",
        description:
          "Run a command in a running container with sh -c, as its user (root, in most images), and wait for it. Gives { exit, stdout, stderr }, the last 64 KB of each. Once its timeout passes it's stopped waiting for and exit is -1, but what it started may carry on in the container.",
        inputs:
          one(%{
            command: %{type: "string", description: ~s(the command line, like "apt-get install -y git"; it reads no input)},
            dir: %{type: "string", description: "the folder to run it in; default the image's"},
            timeout: %{type: "integer", minimum: 1, maximum: 3600, description: "seconds to wait, 1 to 3600; default 60"}
          }, ["command"]),
        outputs: @ran
      },
      %{
        name: "logs",
        read_only: true,
        description: "What the container's own command printed: its last lines.",
        inputs: one(%{tail: %{type: "integer", minimum: 1, description: "how many lines; default 200"}}),
        outputs: @ran
      },
      %{
        name: "read_file",
        read_only: true,
        description:
          "Read a file of a container, running or stopped: text as text, anything else as the file (a picture is seen). Up to 16 MB.",
        inputs: one(%{path: %{type: "string", description: "its absolute path in the container"}}, ["path"]),
        outputs: %{
          type: "object",
          description: "Text as text, or anything else as the file, which an agent sees where it's a picture",
          properties: %{
            path: %{type: "string"},
            text: %{type: "string"},
            mimeType: %{type: "string", description: "The file's kind, where it isn't text"},
            size: %{type: "integer"}
          }
        }
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
          ),
        outputs: %{type: "object", properties: %{path: %{type: "string"}}, required: ["path"]}
      },
      %{
        name: "upload",
        description:
          "For an app's code: send a file of a container, running or stopped, to the bucket by `link`, which writes one blob, sealed with `key`. A file of any size, a chunk at a time, never through a tool's result. `download` brings it back.",
        inputs: one(%{path: %{type: "string", description: "its absolute path in the container"}, link: @link, key: @key}, ["path", "link", "key"]),
        outputs: %{
          type: "object",
          properties: %{path: %{type: "string"}, size: %{type: "integer", description: "Its size in bytes"}},
          required: ["path", "size"]
        }
      },
      %{
        name: "download",
        description:
          "For an app's code: write a file into a container from a blob `upload` sent, by `link`, which reads it, opened with `key`. Its folder must be there already.",
        inputs: one(%{path: %{type: "string", description: "its absolute path in the container"}, link: @link, key: @key}, ["path", "link", "key"]),
        outputs: %{
          type: "object",
          properties: %{path: %{type: "string"}, size: %{type: "integer", description: "Its size in bytes"}},
          required: ["path", "size"]
        }
      },
      %{
        name: "commit",
        description:
          "Make an image of a container as it is now: its files, not what's running. `create` makes throwaway copies from it, on the same machine.",
        inputs:
          one(%{name: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the image's name"}}, ["name"]),
        outputs: %{type: "object", properties: %{image: %{type: "string", description: "What create's image takes"}}, required: ["image"]}
      },
      %{
        name: "remove_image",
        description: "Remove an image you made with `commit`, once no container uses it.",
        inputs: %{
          type: "object",
          required: ["name"],
          properties: %{name: %{type: "string", description: "the image's name"}}
        },
        outputs: %{type: "object", properties: %{image: %{type: "string"}}, required: ["image"]}
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
          ),
        outputs: %{
          type: "object",
          properties: %{
            status: %{type: "integer"},
            headers: %{type: "object", additionalProperties: %{type: "string"}},
            body: %{type: "string", description: "As text, up to 16 MB"}
          },
          required: ["status", "headers", "body"]
        }
      },
      %{
        name: "screenshot",
        read_only: true,
        description:
          "A picture of a container's screen: the X display its DISPLAY names, as a PNG. A container of the image desktop has one, 1280×800 until `screen` resizes it, with Google Chrome on it; `exec` with xdotool moves, clicks and types there.",
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
          "Look at or act on a container's screen, for Computer Use, whose tools are how agents use a screen: they say what each action takes. `resize` makes the display width × height, and a maximised window follows it. `stream` is for a person's app to show it as live video over WebRTC (the image desktop's, or any with an X display): with no `offer`, the ICE servers to connect with; with the app's whole SDP `offer`, {answer}, the screen's.",
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
              width: %{type: "integer", minimum: 320, maximum: 7680},
              height: %{type: "integer", minimum: 240, maximum: 4320}
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
        name: "settings",
        description:
          "The settings: idle_hours, how long a running container goes unused before it's stopped (default #{@idle_hours}). Gives them; with one given, its owner changes it.",
        inputs: %{
          type: "object",
          properties: %{idle_hours: %{type: "integer", minimum: 1, description: "hours a container may go unused"}}
        },
        outputs: %{
          type: "object",
          properties: %{idle_hours: %{type: "integer", description: "Hours a container may go unused before it's stopped"}},
          required: ["idle_hours"]
        }
      },
      %{
        name: "stop_idle",
        description:
          "Stop every running container, anyone's, that has gone unused longer than idle_hours, on each machine. It's run every hour, and gives the ones it stopped.",
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
            },
            failed: @failed
          },
          required: ["stopped", "failed"]
        }
      }
    ]
  end

  defp one(properties \\ %{}, required \\ []) do
    %{type: "object", required: ["container" | required], properties: Map.put(properties, :container, @container)}
  end

  @impl true
  def call(tool, _inputs, %Call{caller: nil}) when tool in ~w(upload download),
    do: raise("containers_#{tool} is for an app's code, which says where the file goes and what it's sealed with.")

  def call("list", _inputs, call) do
    {containers, failed} = listed(call)
    %{containers: containers, failed: failed}
  end

  def call("machines", _inputs, call) do
    servers = Service.servers(call, Podman)

    machines =
      for {m, server} <- Enum.zip(Service.machines(call, Podman), servers) do
        if m.on do
          try do
            Map.put(m, :desktop, Service.call(call, Podman, :progress, [], server))
          rescue
            problem -> Map.put(m, :failed, Exception.message(problem))
          end
        else
          m
        end
      end

    %{machines: machines}
  end

  def call("images", _inputs, call) do
    {images, failed} = images(call)
    %{images: Enum.map(images, &Map.drop(&1, [:server, :artifact, :id, :ours])), failed: failed}
  end

  def call("create", inputs, call) do
    name = inputs["name"]
    # A name on a machine that doesn't answer is caught below, where it's taken (`take_name!/3`).
    {containers, _failed} = containers(call)
    if Enum.any?(containers, &(&1.name == name)), do: raise("You have a container called #{name} already")
    servers = Service.servers(call, Podman)
    if servers == [], do: raise("No computer runs containers yet. Ask your network's admin to set one up.\n\n`install_service` runs Containers.Podman on one.")
    {image, server} = image(call, inputs["image"], inputs["machine"] && Service.machine!(call, Podman, inputs["machine"]))
    server = server || hd(servers)

    id = Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)
    artifact = Artifact.name(Call.make(call, id))
    take_name!(call, name, server)
    machine = Service.name(server)

    image =
      Service.call(call, Podman, :image, [image], server) ||
        if image == "desktop" do
          Call.stage(call, "Building the desktop image on #{machine}, which takes about a minute: `machines` says how far it is")
          Service.call(call, Podman, :build, [], server)
        else
          Call.stage(call, "Pulling #{image} on #{machine}")
          Service.call(call, Podman, :pull, [image], server)
        end

    spec = %{
      "id" => "#{name}-#{id}",
      "image" => image,
      "size" => inputs["size"] || "small",
      "restart" => inputs["restart"] || "no",
      "ports" => inputs["ports"],
      "command" => inputs["command"],
      "env" => inputs["env"],
      "mounts" => inputs["mounts"],
      "labels" => %{
        "artifact" => artifact,
        "name" => name,
        "description" => inputs["description"],
        "size" => inputs["size"] || "small",
        "restart" => inputs["restart"] || "no",
        "by" => call.actor || call.subject
      }
    }

    Call.stage(call, "Starting #{name} on #{machine}")
    Service.call(call, Podman, :create, [spec], server)
    Enum.find(elem(listed(call), 0), &(&1.name == name))
  end

  def call(tool, %{"container" => name} = inputs, call) when tool in ~w(start stop) do
    on(call, inputs, String.to_existing_atom(tool))
    %{container: name}
  end

  def call("remove", inputs, call) do
    c = container!(call, inputs)
    Service.call(call, Podman, :remove, [c.id], c.server)
    Artifact.remove(call.own, name_record(String.replace(c.artifact, ~r{/made/.*}, ""), c.name))
    %{container: c.name}
  end

  def call("logs", inputs, call), do: on(call, inputs, :logs, [Map.get(inputs, "tail", 200)])

  def call("exec", inputs, call) do
    if String.trim(inputs["command"]) == "", do: raise("Give a command")
    on(call, inputs, :exec, [inputs["command"], inputs["dir"], Map.get(inputs, "timeout", 60)])
  end

  def call("read_file", %{"path" => path} = inputs, call) do
    data = Base.decode64!(on(call, inputs, :read_file, [path]))

    if String.valid?(data) and not String.contains?(data, <<0>>),
      do: %{path: path, text: data},
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
    %{path: path}
  end

  def call(tool, %{"path" => path, "link" => link, "key" => key} = inputs, call) when tool in ~w(upload download),
    do: %{path: path, size: on(call, inputs, String.to_existing_atom(tool), [path, link, key])}

  def call("commit", %{"name" => name} = inputs, call) do
    {images, _failed} = images(call)
    if Enum.any?(images, &(&1.artifact && &1.name == name)), do: raise("You have an image called #{name} already")
    id = Base.encode16(:crypto.strong_rand_bytes(4), case: :lower)
    labels = %{"artifact" => Artifact.name(Call.make(call, "image-" <> id)), "name" => name}
    on(call, inputs, :commit, ["localhost/#{name}-#{id}", labels])
    %{image: name}
  end

  def call("remove_image", %{"name" => name}, call) do
    {images, failed} = images(call)
    image = Enum.find(images, &(&1.artifact && &1.name == name)) || raise "You have no image called #{name}#{Service.unanswered(failed)}"
    Service.call(call, Podman, :remove_image, [image.id], image.server)
    %{image: name}
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

  def call("screen", %{"action" => "resize", "width" => width, "height" => height} = inputs, call) when is_integer(width) and is_integer(height) do
    on(call, inputs, :resize, [width, height])
    %{done: "Resized to #{width}×#{height}"}
  end

  # A stream's ICE servers, for the app, then, with its offer, WebRTC's sender put in the container and started with
  # its own, and its answer. Both are given only to whoever reaches the container, as everything here is.
  def call("screen", %{"action" => "stream"} = inputs, call) do
    c = container!(call, inputs)
    ice_servers = Call.tool(call, "webrtc", "ice_servers", %{})

    case inputs["offer"] do
      nil -> ice_servers
      offer ->
        request = Map.merge(ice_servers, %{"offer" => offer, "sender" => Call.tool(call, "webrtc", "sender", %{})})
        %{"answer" => Service.call(call, Podman, :stream, [c.id, JSON.encode!(request)], c.server)}
    end
  end

  def call("screen", %{"action" => action} = inputs, call) do
    on(call, inputs, :xdotool, [xdotool(action, inputs)])
    %{done: did(action, inputs)}
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

    {stopped, failed} = Service.stop_idle(call, Podman, idle)

    if stopped != [], do: Call.changed(call)
    %{stopped: stopped, failed: failed}
  end

  # Computer Use asks for the caller's screens: each running container of theirs with a display is one, called what
  # the container is, and acted on with `screen`.
  @impl true
  def hook("computer-use.screens", _about, call) do
    {answers, failed} = Service.each(call, Podman, :screens)

    screens =
      for {server, s} <- answers, reaches?(call, s) do
        %{
          "name" => s["name"],
          "description" => "#{s["description"]}, a container on #{Service.name(server)}",
          "width" => s["width"],
          "height" => s["height"],
          "tool" => "screen",
          "inputs" => %{"container" => s["name"]}
        }
      end

    %{"screens" => screens, "failed" => failed}
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

  @doc false
  # How far the desktop image's build is changed: every app watching is told. For `Containers.Podman`.
  def changed(call), do: Call.changed(call)

  defp settings(call) do
    case Artifact.read(call.own, "settings") do
      {:ok, data, _tag} -> Map.merge(%{"idle_hours" => @idle_hours}, JSON.decode!(data))
      :absent -> %{"idle_hours" => @idle_hours}
    end
  end

  # Takes the name for a container of whoever the call is for, on `server`: refused while one of theirs on a machine
  # that's off has it. A name whose machine is on, where `create` found no container of theirs called that, was left
  # by one that went some other way (removed by hand, or never made), and is taken again.
  defp take_name!(call, name, server) do
    record = name_record(call.subject, name)

    with :exists <- Artifact.write(call.own, record, server),
         {:ok, at, tag} <- Artifact.read(call.own, record) do
      if is_integer(Service.seen(at)), do: raise("You have a container called #{name} on #{Service.name(at)}, which is off")
      if Artifact.replace(call.own, record, server, tag) != :ok, do: raise("You have a container called #{name} already")
    end
  end

  defp name_record(whose, name), do: "names/#{whose}/#{name}"

  # The image to create from, and the machine it's on: one of the caller's own made with `commit`, by its name or
  # Podman's, or any other, which Podman pulls, but never another's made with `commit`, nor another network's.
  defp image(call, image, server) do
    # An image on a machine that doesn't answer is as one pulled there: created where it's asked to be.
    {all, _failed} = images(call, :all)
    named? = &(image in [&1.name | &1.tags] or image =~ ~r/^(sha256:)?[0-9a-f]{3,}$/ and String.contains?(&1.id, image))

    case Enum.find(all, &(&1.artifact && &1.ours && named?.(&1) && reaches?(call, &1))) do
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
    c = container!(call, inputs)
    Service.call(call, Podman, function, [c.id | arguments], c.server)
  end

  # The containers this call may use, as `list` answers them, and those on a machine that's off, unknown till it's on,
  # and why each machine that didn't answer couldn't, by its name.
  defp listed(call) do
    {containers, failed} = containers(call)
    {Enum.map(containers, &Map.drop(&1, [:server, :artifact, :id])) ++ off(call), failed}
  end

  # A container on a machine that's off is known only by its name, kept with its machine: whoever may reach what this
  # app made for whose it is sees it. Podman, which holds the rest, is on that machine.
  defp off(call) do
    off = for server <- Service.servers(call, Podman), at = Service.seen(server), is_integer(at), into: %{}, do: {server, at}

    for record <- if(off == %{}, do: [], else: Artifact.list(call.own, "names/")),
        {whose, [name]} = Enum.split(tl(String.split(record, "/")), -1),
        Call.reaches?(call, Enum.join(whose ++ ["made", call.app], "/")),
        {:ok, server, _tag} <- [Artifact.read(call.own, record)],
        Map.has_key?(off, server),
        do: %{name: name, machine: Service.name(server), state: "unknown", last_on: off[server]}
  end

  defp container!(call, %{"container" => name}) do
    {containers, failed} = containers(call)

    Enum.find(containers, &(&1.name == name)) ||
      case Enum.find(off(call), &(&1.name == name)) do
        nil -> raise "You have no container called #{name}#{Service.unanswered(failed)}"
        c -> raise "#{name} is on #{c.machine}, which is off"
      end
  end

  # The containers this call may use, on every machine that's up, and why each that didn't answer couldn't.
  defp containers(call) do
    {answers, failed} = Service.each(call, Podman, :list)

    {for({server, c} <- answers, reaches?(call, c), do: Map.merge(Map.new(c, fn {key, value} -> {String.to_atom(key), value} end), %{server: server, machine: Service.name(server)})),
     failed}
  end

  # The images this call may create from, or `:all` of them, with whose each is, on every machine that's up.
  defp images(call, which \\ :reached) do
    {answers, failed} = Service.each(call, Podman, :images)

    {for({server, i} <- answers,
         which == :all or i["artifact"] == nil or (i["ours"] and reaches?(call, i)),
         do: Map.merge(Map.new(i, fn {key, value} -> {String.to_atom(key), value} end), %{server: server, machine: Service.name(server)})),
     failed}
  end

  defp reaches?(call, %{artifact: artifact}), do: Call.reaches?(call, artifact)
  defp reaches?(call, %{"artifact" => artifact}), do: Call.reaches?(call, artifact)
end
