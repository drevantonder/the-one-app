defmodule OneCore.Tools.Builds do
  @moduledoc "One Core's own tools for the platform's builds: which there are, downloading and publishing them, and the network's update, as `OneCore.Tools` calls them. "
  import OneCore.Tools.Helpers

  alias OneCore.{Builds, Release, User}

  @link link()

  @download %{
    type: "object",
    properties: %{
      build: %{type: "string"},
      sha256: %{type: "string"},
      size: %{type: "integer"},
      signature: %{
        type: "string",
        description: "The platform's Ed25519 signature of `<name> <build> <sha256>`, in base64"
      },
      link: @link
    },
    required: ["build", "sha256", "size", "link"]
  }

  @protocol %{
    type: ["object", "null"],
    properties: %{oldest: %{type: "integer"}, newest: %{type: "integer"}},
    description:
      "The protocols it speaks with the network's other parts (`OneCore.Protocol`), from `oldest` to `newest`, or " <>
        "null for none this server knows"
  }

  @server_update_status %{
    type: "string",
    enum: ["up_to_date", "downloading", "checked", "verifying"],
    description:
      "Where an update of it is: up_to_date, downloading the latest build and program while it answers on and " <>
        "checking the build starts, checked and waiting for the network's update to say go, or verifying them, by " <>
        "starting both in its place, kept once the build says it's running at the edge, and otherwise gone back from"
  }

  @server_update_stage %{
    type: ["string", "null"],
    enum: ["checking", "starting", nil],
    description:
      "Where downloading is once it's downloaded, checking the build starts, and where verifying is, starting it " <>
        "in the server's place"
  }

  @tools [
    %{
      name: "builds",
      read_only: true,
      description: """
      Lists the build each of your servers runs, with what its machine is called and whether it's ready: active
      (`server_membership`) and running (`server_daemon_status`), or why not; and the build each of your devices
      last said it is, and the latest for each kind of machine, so it says whether everything is on the same build; and whether its apps are `synced` with what's
      installed or `syncing`, under `apps`.
      A build is the version of the commit it was built from, Semantic Versioning's, with its commit's count and
      hash as build metadata: 0.15.2+2974.febd37350abc. Servers, the edge and clients talk where the protocols they
      speak overlap, whatever their versions (`protocol`, from `oldest` to `newest`): `why` says what's on one that
      doesn't. With `server`, that server answers, saying its own.
      """,
      inputs: %{
        type: "object",
        properties: %{server: %{type: "string", description: "The server's key. The server that answers, if left out"}}
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server that answered"},
          build: %{type: "string", description: "Its build: <version>+<count>.<commit>"},
          protocol: @protocol,
          edge: %{type: ["string", "null"], description: "The build the network's edge is, if it says"},
          edge_protocol: %{
            type: ["object", "null"],
            description: "The protocols the network's edge speaks, if it's reached"
          },
          latest: %{
            type: "object",
            description: "The latest build of each kind of machine, by its name: server-runtime/linux-x86_64"
          },
          servers: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                server: %{type: "string", description: "Its key"},
                name: %{type: ["string", "null"], description: "What its machine is called"},
                build: %{type: "string"},
                server_membership: %{
                  type: "string",
                  network: %{
                    type: ["object", "null"],
                    description:
                      "The network's update, as its updater says, if its edge has one: `network_update_status` (up_to_date, " <>
                        "preparing, updating_edge, loading, restarting or held), the `build` it takes every server to, how many " <>
                        "are `done` `of` those taking part, what it's `waiting_for` (quiet: a lone server that restarts waits " <>
                        "for no calls a while, or Update Now), why it's `held`, the servers `skipped` as asleep or gone, " <>
                        "those `behind` its build, and when each step was reached, in ms after the build was published (`timings`)"
                  },
                  enum: ["inactive", "joining", "active", "leaving"],
                  description:
                    "Whether it's one of the network's servers: joining until it answers by its key, and leaving while it drains"
                },
                server_daemon_status: %{
                  type: "string",
                  enum: ["starting", "running", "stopping", "stopped"],
                  description: "Its program, which its machine keeps running: starting until it answers by its key"
                },
                server_ready: %{type: "boolean", description: "Whether it's sent calls: it's active and running"},
                server_ready_reason: %{
                  type: ["string", "null"],
                  enum: ["joining", "starting", "draining", "stopped", nil],
                  description: "Why it isn't ready"
                },
                last_seen: %{
                  type: ["integer", "null"],
                  description: "When it was last connected, in milliseconds since 1970, if it isn't now"
                },
                why: %{
                  type: "string",
                  description:
                    "Why it doesn't talk with the server that answered or the edge: they speak no protocol in common"
                },
                protocol: @protocol,
                server_update_status: @server_update_status,
                server_update_stage: @server_update_stage,
                runtime: %{type: ["string", "null"], description: "The server-runtime build it runs on"},
                went_back: %{type: ["string", "null"], description: "Why its last update didn't take, until one does"}
              },
              required: ["server", "name"]
            }
          },
          devices: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                device: %{type: "string", description: "The device's key"},
                build: %{type: "string"},
                protocol: @protocol,
                connected: %{type: "boolean", description: "Whether it's connected to the network's edge now"},
                last_seen: %{type: "integer", description: "When it was last connected, in milliseconds since 1970"}
              },
              required: ["device"]
            }
          },
          apps: %{
            type: "string",
            enum: ["synced", "syncing"],
            description:
              "Whether what runs on it is what's installed for everyone it runs apps for, or it's syncing them"
          },
          waiting: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{name: %{type: "string"}, state: %{type: "string"}, why: %{type: "string"}}
            },
            description:
              "The apps installed for you whose version doesn't run on the server that answered, waiting_for_server, and why"
          }
        },
        required: ["server", "build", "edge", "latest", "servers", "devices", "apps"]
      }
    },
    %{
      name: "download",
      read_only: true,
      description: """
      Gives the latest builds for a kind of machine, each with its SHA-256, the platform's signature and a link that
      downloads it within a minute: `server` for a server installed there, and `one` for the program.
      `one serve on` and `one update` take them, checking each against its signature and SHA-256 before they run it.
      """,
      inputs: %{
        type: "object",
        properties: %{
          target: %{
            type: "string",
            description: "The kind of machine: linux-x86_64, linux-aarch64, macos-aarch64, windows-x86_64"
          }
        },
        required: ["target"]
      },
      outputs: %{
        type: "object",
        properties: %{server: @download, one: @download},
        description: "Each of the latest builds there is for that kind of machine"
      }
    },
    %{
      name: "publish",
      description: """
      Makes builds the latest for the network's machines, which they take when they're asked to update.
      While the network lacks any of their files, it gives a link to send each by, with its `method` and
      `headers`, and changes nothing: send them, and ask again. `tooling/release` does all of this.
      """,
      inputs: %{
        type: "object",
        properties: %{
          builds: %{
            type: "array",
            description:
              "Each build: its name, <what>/<target> for a kind of machine (server-runtime/linux-x86_64, " <>
                "one/macos-aarch64, one-setup/windows-x86_64) or <what> for every kind (server-code), its build, " <>
                "sha256 and size, a runtime's inputs, the SHA-256 of what it's built from, and the platform's " <>
                "signature of it, which `one` checks before it takes it",
            items: %{
              type: "object",
              properties: %{
                name: %{
                  type: "string",
                  pattern: "^[a-z0-9-]+(/[a-z0-9_]+-[a-z0-9_]+)?$"
                },
                build: %{type: "string"},
                inputs: %{type: "string", pattern: "^[0-9a-f]{64}$"},
                sha256: %{type: "string", pattern: "^[0-9a-f]{64}$"},
                size: %{type: "integer"},
                signature: %{type: "string", description: "Ed25519's of `<name> <build> <sha256>`, in base64"}
              },
              required: ["name", "build", "sha256", "size"]
            }
          }
        },
        required: ["builds"]
      },
      outputs: %{
        type: "object",
        properties: %{
          send: %{
            type: "object",
            additionalProperties: @link,
            description: "Where to send each build's file the network lacks, by its name, if it lacks any"
          },
          latest: %{type: "object", description: "The latest builds, by their names, once it has them all"}
        }
      }
    },
    %{
      name: "update",
      task_support: :optional,
      description: """
      Software Update's Update Now: has the network's update (`builds`' `network`) take every server to the latest
      build published to the network now, where it waits, as a lone server that must restart waits for a quiet
      moment, or try again where it's held. It answers once the network is up to date, or held, saying which server
      didn't take it and why. Servers take each build published by themselves, so it's needed only for those.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{network: %{type: ["object", "null"], description: "The network's update, as `builds` says it"}},
        required: ["network"]
      }
    }
  ]

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Apps.Version.schemas/1)

  @doc "Its tools, each as an app says one of its own."
  def tools, do: @taking

  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  def run("builds", inputs, {actor, subject} = principal, _pass, _request) do
    on(inputs["server"], "builds", inputs, principal, fn ->
      Builds.list(actor, User.owner(subject) || "network")
    end)
  end

  def run("download", %{"target" => target}, _principal, _pass, _request), do: Builds.download(target)

  # What the network's servers run is the network's.
  def run("publish", %{"builds" => builds}, principal, forward, _request) do
    at("network", forward)
    reaches!(principal, "network", "owner")

    case Builds.publish(builds) do
      {:send, links} -> %{send: links}
      {:ok, latest} -> %{latest: latest}
    end
  end

  # What takes the latest builds is the program that keeps an installed server running, which hears this said.
  def run("update", _inputs, principal, _pass, _request) do
    reaches!(principal, "network", "owner")
    network = Release.update_now()
    {:content, %{network: network}, [%{type: "text", text: Release.software_update(network)}]}
  end
end
