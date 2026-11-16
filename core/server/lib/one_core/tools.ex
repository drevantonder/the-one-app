defmodule OneCore.Tools do
  @moduledoc """
  One Core's own tools: what installs, updates and removes plugins, servers and services.

  They're called as a plugin's tools are, through the same hooks (`OneCore.Plugins.Calls.tool/6`): the plugins that
  mutate a call's inputs mutate theirs, the ones that validate a call validate theirs, and the ones that observe
  are told. What each changes it reaches as a plugin reaches an artifact, so the plugins that validate `reach` say
  whether whoever asked may: an `editor` of a plugin deploys over it, a `viewer` reads its code and installs it, and
  so does a `member` of the group or the network it's whoever's or installed for, since what's in use is open to
  everyone it's in use for. An `installer` of a user, a group or the network adds plugins for them, which an `owner`
  is, and only an `owner` removes them and says which servers act for them. A plugin is added only where a server
  would run it (`OneCore.Servers.runs?/1`): for a user or a group, one of their own. A server is its owner's: who it was started for, and only their `owner` says
  who else it acts for and what it runs. Nothing is asked about what's whoever's asked, which is what their name
  starts. With no plugin validating `reach`, they're the network's first owner's alone.

  What a tool changes is whoever's it is, so a server acting for them runs it: the one asked, or one it passes
  the call on to.
  """
  alias OneCore.{Artifact, Bucket, Builds, Call, Hooks, Plugins, Release, Server, Servers, Services, User}
  alias OneCore.Plugins.{Installed, Calls, Deploy, Rename, Store}

  # What a plugin may do beyond its own, as `permissions/0` says it.
  @permissions %{
    type: "array",
    items: %{type: "string"},
    description: "What it may do beyond its own: other plugins' tools it uses, and hosts it sends data to"
  }

  @files %{
    type: "object",
    additionalProperties: %{
      anyOf: [
        %{type: "string"},
        %{
          type: "object",
          properties: %{base64: %{type: "string", contentEncoding: "base64"}},
          required: ["base64"],
          additionalProperties: false
        }
      ]
    }
  }

  # A short-lived link to the bucket, as `OneCore.Bucket.link/2` gives it: sent by its `method` to its `url` with its
  # `headers`.
  @link %{
    type: "object",
    properties: %{
      method: %{type: "string"},
      url: %{type: "string"},
      headers: %{type: "object", additionalProperties: %{type: "string"}}
    },
    required: ["method", "url", "headers"]
  }

  @download %{
    type: "object",
    properties: %{
      build: %{type: "string"},
      sha256: %{type: "string"},
      size: %{type: "integer"},
      link: @link
    },
    required: ["build", "sha256", "size", "link"]
  }

  @update %{
    type: ["string", "null"],
    enum: ["downloading", "restarting", "starting", nil],
    description:
      "Where an update of it is: downloading the latest build, restarting to run it, or starting it, which is " <>
        "kept once it answers and otherwise left for the one before. Nothing while none is under way"
  }

  @acts_for %{
    type: "object",
    properties: %{
      server: %{type: "string", description: "The server's key"},
      for: %{type: "string", description: "Who it acts for, or no longer does"}
    },
    required: ["server", "for"]
  }

  @service %{
    type: "object",
    properties: %{
      plugin: %{type: "string"},
      service: %{type: "string"},
      server: %{type: "string", description: "The key of the server it runs on, or no longer does"}
    },
    required: ["plugin", "service", "server"]
  }

  @tools [
    %{
      name: "deploy",
      description: """
      Adds a plugin for you, or updates it, from its files.
      The server builds its apps from their source in client/, compiles its Elixir in server/ and runs the plugin at
      once, with its tools listed and called as <name>_<tool>. If either fails, it says why, and the version already
      running stays.
      The skill `building-plugins` says how to write a plugin. It's yours, and is installed for you: asked for a group
      or for the network, it's theirs. Its code runs on a server of yours, so you need one: set one up in Devices, or
      ask for it to be installed for everyone, by the network's launcher (its skill says how). In sandbox mode
      (`sandbox`) it runs in a VM of its own behind its machine's sandbox, on any of the network's servers, so it needs
      no server of yours: it then reaches only what One Core hands it, and the tools and hosts its permissions/0
      names. It answers with them. One nobody has installed is removed as anything deleted is, by the plugin that keeps
      what's deleted, where there is one (recently-deleted).
      `one install <owner>/<repo> --plugin <name>` deploys one from a git repo, saying where it came from (`from`).
      `one deploy --files-from <folder>` in a git repo says so too, and, as `git push` refuses a non-fast-forward, is
      refused where the live version came from a commit that yours doesn't contain, unless `--force`.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The plugin's name: lowercase letters, digits and hyphens"},
          files:
            Map.put(
              @files,
              :description,
              "The plugin's files by their paths in it: each one's text, or {\"base64\": ...} for one that isn't text"
            ),
          from: %{
            type: ["object", "null"],
            description:
              "The git repo its files came from, if they did: its `source` (owner/repo, a URL or a path), the `ref` asked for, the `commit`, and whether they had changes not committed (`dirty`). Left out, the version before's stays; null says they came from none",
            properties: %{
              source: %{type: "string"},
              ref: %{type: ["string", "null"]},
              commit: %{type: "string", pattern: "^[0-9a-f]{40}$"},
              dirty: %{type: "boolean"}
            },
            required: ["source", "commit"],
            additionalProperties: false
          },
          sandbox: %{
            type: "boolean",
            description:
              "Whether it runs in sandbox mode. Left out, it runs as the version before did, and a new one doesn't"
          },
          new: %{
            type: "boolean",
            description:
              "Whether a new plugin is meant where one by this name is installed for someone else you're in: the network, a group of yours or you. Left out, that's refused, since the new one would answer first, empty"
          }
        },
        required: ["name", "files"]
      },
      outputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The plugin's name"},
          version: %{type: "integer", description: "The version now running"},
          sandbox: %{type: "boolean", description: "Whether it runs in sandbox mode"},
          permissions: @permissions,
          warnings: %{type: ["string", "null"], description: "What the compiler warns of, if anything"}
        },
        required: ["name", "version", "sandbox", "permissions", "warnings"]
      }
    },
    %{
      name: "code",
      description: """
      Gives a plugin's files as they were deployed, its app's source among them, by their paths: what `deploy`
      takes, to change and deploy again. Asked for a group or the network, it's one of the plugins in use for them,
      which their members may read.
      """,
      read_only: true,
      inputs: %{
        type: "object",
        properties: %{name: %{type: "string", description: "The plugin's name"}},
        required: ["name"]
      },
      outputs: %{
        type: "object",
        properties: %{files: Map.put(@files, :description, "Its files by their paths, as `deploy` takes them")},
        required: ["files"]
      }
    },
    %{
      name: "plugins",
      read_only: true,
      description: """
      Lists the plugins installed for you and for the network, as `npm ls` does packages: asked for a group, for it
      and the network. Each with the name it's installed by, who it's installed `for`, its artifact (`plugin`), its
      latest version, whether it runs in sandbox mode (`sandbox`), what it may do beyond its own (`permissions`), the hash
      of the files that version was deployed from (`source`), and, if it was installed from
      a git repo, where from (`from`): its `source`, the `ref` asked for, the `commit`, whether it had changes not
      committed (`dirty`), and the `hash` of the files as installed, which differs from `source` once it's changed since. `one plugins check` says which of those
      have moved on upstream or changed since, and `one plugins update` installs the newer.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: ["string", "null"], description: "Whoever they're listed for"},
          plugins: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                name: %{type: "string", description: "The name it's installed by"},
                for: %{type: "string", description: "Who it's installed for"},
                plugin: %{type: "string", description: "Its artifact: users/alice/plugins/notes"},
                version: %{type: ["integer", "null"], description: "Its latest version"},
                sandbox: %{type: "boolean"},
                permissions: @permissions,
                source: %{type: ["string", "null"], description: "The hash of the files its latest version was deployed from"},
                from: %{
                  type: ["object", "null"],
                  description: "The git repo it was installed from, if it was",
                  properties: %{
                    source: %{type: "string"},
                    ref: %{type: ["string", "null"]},
                    commit: %{type: "string"},
                    dirty: %{type: "boolean"},
                    hash: %{type: "string"}
                  }
                }
              },
              required: ["name", "for", "plugin", "sandbox", "permissions"]
            }
          }
        },
        required: ["for", "plugins"]
      }
    },
    %{
      name: "install",
      description: """
      Adds a plugin someone else deployed for you: asked for a group or for the network, for them. It answers with
      what the plugin may do beyond its own (its permissions).
      Its code runs on a server of yours, so you need one, unless it runs in sandbox mode; for the network, its owners
      and installers add plugins, and anyone may ask them to, by the network's launcher (its skill says how), which
      also puts an app of a plugin in use for the network on your page without installing anything.
      `one install <owner>/<repo> --plugin <name>` installs one from a git repo instead, as a copy of your own.
      It's the same plugin, at its latest version, with an artifact of its own for whoever it's installed for. One in
      use for a group you're in or for the network is open to you; one that isn't, whoever it belongs to shares
      with you first, as a viewer. The plugins it uses are installed for you first.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin, by its artifact's name: users/alice/plugins/notes"},
          name: %{type: "string", description: "What to call it. Its own name, if left out"}
        },
        required: ["plugin"]
      },
      outputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin, by its artifact's name"},
          for: %{type: "string", description: "Who it's installed for"},
          name: %{type: "string", description: "The name it's installed by"},
          permissions: @permissions
        },
        required: ["plugin", "for", "name", "permissions"]
      }
    },
    %{
      name: "rename",
      description: """
      Renames a plugin of yours, as `git mv` does a file: asked for a group or for the network, theirs. Its history,
      what it keeps and where it's installed, for everyone who has it, all go to the new name, and the plugins that
      keep something of a plugin's follow it, such as a Home Screen page's places. The old name then names nothing.
      An owner of the plugin renames it. One whose services are installed, or that makes artifacts for people, or
      that another plugin names in its code, isn't renamed, and it says why.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The plugin's name"},
          to: %{type: "string", description: "Its new name: lowercase letters, digits and hyphens"}
        },
        required: ["name", "to"]
      },
      outputs: %{
        type: "object",
        properties: %{from: %{type: "string"}, to: %{type: "string"}},
        required: ["from", "to"]
      }
    },
    %{
      name: "uninstall",
      description: "Removes a plugin from what's installed for you. It stays whoever's it is, and what it kept stays.",
      inputs: %{
        type: "object",
        properties: %{name: %{type: "string", description: "The plugin's name"}},
        required: ["name"]
      },
      outputs: %{
        type: "object",
        properties: %{name: %{type: "string"}, for: %{type: "string", description: "Who it was installed for"}},
        required: ["name", "for"]
      }
    },
    %{
      name: "upload",
      description: """
      Gives somewhere to send a file that's too large to go in a call: a link that takes it once, within a minute.
      Send the file as the link says, by its `method` to its `url` with its `headers`, then give `upload`, its
      name, to the plugin's tool that takes one. `one <plugin> <tool> --<input>-from <path>` does all of this.
      """,
      inputs: %{
        type: "object",
        properties: %{plugin: %{type: "string", description: "The plugin whose tool the file is for"}},
        required: ["plugin"]
      },
      outputs: %{
        type: "object",
        properties:
          Map.put(@link.properties, :upload, %{type: "string", description: "The file's name, which the tool takes"}),
        required: ["method", "url", "headers", "upload"]
      }
    },
    %{
      name: "add_server",
      description: """
      Lets a server act for you: asked for a group or for the network, for them.
      It then reads and writes what's yours, runs the plugins installed for you, and takes over when another of your
      servers stops. A new server is its owner's alone, who adds it first. Someone else's acts for you once its
      owner chose you, with `add_member`, and you add it.
      """,
      inputs: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key, which it says when it's started"}},
        required: ["key"]
      },
      outputs: @acts_for
    },
    %{
      name: "remove_server",
      description: """
      Takes a server off your list: asked for a group or for the network, off theirs.
      It no longer acts for you, and is handed nothing of yours.
      """,
      inputs: %{
        type: "object",
        properties: %{key: %{type: "string", description: "The server's key"}},
        required: ["key"]
      },
      outputs: @acts_for
    },
    %{
      name: "add_member",
      description: """
      Chooses someone else a server of yours acts for too: a user, a group or the network.
      It runs the plugins they add on its machine, once they add it with `add_server`.
      """,
      inputs: %{
        type: "object",
        properties: %{
          who: %{type: "string", description: "Who: users/<name>, groups/<name> or network"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["who"]
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The server's key"},
          member: %{type: "string", description: "Who it acts for too"}
        },
        required: ["server", "member"]
      }
    },
    %{
      name: "services",
      read_only: true,
      description: """
      Lists the services your plugins have, what each needs of a machine, and the servers that run it, with
      what each server's machine is called. A service is a part of a plugin that runs on particular machines,
      for what is only there.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server that answered"},
          servers: %{type: "array", items: %{type: "string"}, description: "The keys of the servers acting for you"},
          names: %{
            type: "object",
            additionalProperties: %{type: ["string", "null"]},
            description: "What each server's machine is called, by its key"
          },
          services: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                plugin: %{type: "string"},
                service: %{type: "string"},
                called: %{type: "string", description: "What it's called where a person reads it"},
                needs: %{type: "string", description: "What it needs of a machine"},
                servers: %{type: "array", items: %{type: "string"}, description: "The keys of the servers running it"}
              },
              required: ["plugin", "service", "called", "needs", "servers"]
            }
          }
        },
        required: ["server", "servers", "names", "services"]
      }
    },
    %{
      name: "install_service",
      description: """
      Runs a plugin's service on a server from now on.
      That is the server's owner's to decide, knowing what the service needs of the machine, which `services` says.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      },
      outputs: @service
    },
    %{
      name: "uninstall_service",
      description: "Stops running a plugin's service on a server.",
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["plugin", "service"]
      },
      outputs: @service
    },
    %{
      name: "builds",
      read_only: true,
      description: """
      Lists the build each of your servers runs, with what its machine is called and whether it's on, and the
      build each of your devices last said it is, and the latest for each kind of machine, so it says whether
      everything is on the same build, and which plugins The One App ships with the server is still deploying, or
      failed to deploy and why, under `bundled`.
      A build is the commit it was built from. With `server`, that server answers, saying its own.
      """,
      inputs: %{
        type: "object",
        properties: %{server: %{type: "string", description: "The server's key. The server that answers, if left out"}}
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server that answered"},
          build: %{type: "string", description: "Its build: <count>-<commit>"},
          latest: %{type: "object", description: "The latest build of each kind of machine, by its name: server/linux-x86_64"},
          servers: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                server: %{type: "string", description: "Its key"},
                name: %{type: ["string", "null"], description: "What its machine is called"},
                build: %{type: "string"},
                seen: %{
                  type: ["string", "integer", "null"],
                  description:
                    "\"on\", \"restarting\" while it stops to start again, or when it was last on, in milliseconds since 1970"
                },
                protocol: %{
                  type: "integer",
                  description: "The version of what it asks other servers, which those it asks share"
                },
                update: @update,
                went_back: %{type: ["string", "null"], description: "Why its last update didn't take, until one does"}
              },
              required: ["server", "name", "seen"]
            }
          },
          devices: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{device: %{type: "string", description: "The device's key"}, build: %{type: "string"}},
              required: ["device"]
            }
          },
          bundled: %{
            type: "object",
            additionalProperties: %{type: "string"},
            description: "Each plugin it ships that it's still deploying, by its name: waiting, building, or why it failed"
          }
        },
        required: ["server", "build", "latest", "servers", "devices", "bundled"]
      }
    },
    %{
      name: "download",
      read_only: true,
      description: """
      Gives the latest builds for a kind of machine, each with its SHA-256 and a link that downloads it within a
      minute: `server` for a server installed there, and `one` for the program.
      `one serve on` and `one update` take them, checking each against its SHA-256 before they run it.
      """,
      inputs: %{
        type: "object",
        properties: %{
          target: %{type: "string", description: "The kind of machine: linux-x86_64, linux-aarch64, macos-aarch64"}
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
              "Each build: its name (server/<target>, one/<target> or one-cli/<target>), build, sha256 and size",
            items: %{
              type: "object",
              properties: %{
                name: %{type: "string", pattern: "^(server|one|one-cli)/[a-z0-9_]+-[a-z0-9_]+$"},
                build: %{type: "string"},
                sha256: %{type: "string", pattern: "^[0-9a-f]{64}$"},
                size: %{type: "integer"}
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
      description: """
      Has a server's machine take the latest builds, its server's and its program's, if it isn't on them already.
      It checks each before it runs it, hands its work to another server meanwhile, and goes back to the build it
      had if the new one doesn't start and answer. It answers once the update has begun, with its stage; one under
      way already is left to finish. `builds` then says each stage as it moves on, which build it's on once it's
      done, and if it went back, why.
      """,
      inputs: %{
        type: "object",
        properties: %{server: %{type: "string", description: "The server's key. The server that answers, if left out"}}
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server taking them"},
          update: @update
        },
        required: ["server"]
      }
    }
  ]

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Plugins.Version.schemas/1)

  @doc "One Core's own tools, each as a plugin says one of its own."
  def tools, do: @taking

  @doc """
  Calls one of One Core's own tools for `asked`, who asked and who for. Gives what `OneCore.Plugins.Calls.call/5` does,
  or `:none` if `name` isn't one.
  """
  def call(name, inputs, principal, forwarded \\ false) do
    case Enum.find(@taking, &(&1.name == name)) do
      nil ->
        :none

      tool ->
        # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
        ask = {"tools/call", %{name: name, arguments: inputs}}
        # To a server acting for someone, or with `{:running, owner}` to one that runs their plugins.
        forward = fn
          {:running, owner} -> Call.run_elsewhere(ask, principal, forwarded, owner, "A plugin for #{owner}")
          owner -> Call.forward(ask, principal, forwarded, owner)
        end

        about = %{"plugin" => nil, "tool" => name, "inputs" => inputs}
        # A plugin has no say in its own removal, or in being deployed over: that's how one that breaks is undone.
        apart = if name in ["deploy", "uninstall", "rename"], do: {User.owner(elem(principal, 1)), inputs["name"]}

        Calls.tool(tool, about, principal, nil, &run(name, &1, principal, forward), apart)
    end
  end

  # What a plugin may do beyond its own, as `permissions/0` says it.
  defp permitted(nil), do: []
  defp permitted(version), do: version.permissions

  # Has a server acting for `owner` run this call, if this one doesn't act for them: `forward` ends it here.
  defp at(owner, forward), do: Servers.here?(owner) || forward.(owner)

  # Has a server that runs the plugins installed for `owner` run this call, if this one doesn't, since what it
  # deploys or installs runs there.
  defp runs(owner, forward) do
    cond do
      Servers.runs?(owner) ->
        true

      Servers.running(owner) == [] ->
        raise "A plugin for #{owner} needs a server of their own, since its code runs there and never on the network's. " <>
                "Set one up in Devices, #{Deploy.ask_to_install()}, or run it in sandbox mode (deploy with " <>
                "sandbox: true), which any of the network's servers runs for anyone."

      true ->
        forward.({:running, owner})
    end
  end

  # A plugin is whoever's deployed it. Deploying over one is changing it, and a new one is installed for whoever it's
  # deployed for. Either way it's loaded where it will run.
  # A plugin in sandbox mode runs on any server acting for whoever it's for, the network's among them.
  defp run("deploy", %{"name" => name, "files" => files} = inputs, {_actor, subject} = principal, forward) do
    plugin = Plugins.plugin(subject, name)
    sandbox = Map.get(inputs, "sandbox", plugin != nil and Store.about(plugin)["sandbox"] == true)
    if sandbox, do: at(subject, forward), else: runs(subject, forward)

    case plugin do
      nil -> reaches!(principal, subject, "installer")
      plugin -> reaches!(principal, plugin, "editor")
    end

    if plugin == nil and inputs["new"] != true, do: elsewhere!(principal, name)

    case Deploy.deploy(subject, name, Store.received(files), Map.get(inputs, "from", :stored), sandbox) do
      {:ok, number, warnings} ->
        %{
          name: name,
          version: number,
          sandbox: sandbox,
          permissions: permitted(Plugins.running(subject)[name]),
          warnings: if(warnings == "", do: nil, else: warnings)
        }

      {:error, why} ->
        raise why
    end
  end

  defp run("code", %{"name" => name}, {_actor, subject} = principal, _pass) do
    case Plugins.running(subject) do
      %{^name => version} ->
        opens!(principal, version.artifact)
        %{files: Store.sent(Store.source(version.artifact, version.code))}

      _ ->
        raise "There's no plugin named #{name}"
    end
  end

  defp run("plugins", _inputs, {_actor, subject}, _pass) do
    installed =
      for at <- Plugins.chain(subject), {name, version} <- Enum.sort(Plugins.versions([at])), do: {at, name, version}

    latest = Bucket.read_all(for {_at, _name, version} <- installed, do: version.artifact <> "/latest")

    plugins =
      for {{at, name, version}, read} <- Enum.zip(installed, latest) do
        about = with {:ok, latest, _tag} <- read, do: JSON.decode!(latest), else: (_ -> %{})

        %{
          name: name,
          for: at,
          plugin: version.artifact,
          version: about["number"],
          sandbox: version.sandbox,
          permissions: version.permissions,
          source: about["source"],
          from: about["from"]
        }
      end

    %{for: User.owner(subject), plugins: plugins}
  end

  defp run("install", %{"plugin" => plugin} = inputs, {_actor, subject} = principal, forward) do
    name = Map.get(inputs, "name", Path.basename(plugin))
    if Store.about(plugin)["sandbox"] == true, do: at(subject, forward), else: runs(subject, forward)
    opens!(principal, plugin)
    reaches!(principal, subject, "installer")
    with {:error, why} <- Installed.install(subject, name, plugin), do: raise(why)
    %{plugin: plugin, for: subject, name: name, permissions: permitted(Plugins.running(subject)[name])}
  end

  # It's renamed where it runs, which is where it's deployed again.
  defp run("rename", %{"name" => name, "to" => to}, {_actor, subject} = principal, forward) do
    plugin = "#{subject}/plugins/#{name}"
    if Store.about(plugin)["sandbox"] == true, do: at(subject, forward), else: runs(subject, forward)
    reaches!(principal, plugin, "owner")
    with {:error, why} <- Rename.rename(subject, name, to, principal), do: raise(why)
    %{from: name, to: to}
  end

  defp run("uninstall", %{"name" => name}, {_actor, subject} = principal, forward) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    with {:error, why} <- Installed.uninstall(subject, name), do: raise(why)
    %{name: name, for: subject}
  end

  # A file sent this way lands in the plugin's own artifact, for whoever it was installed for, under a name nobody
  # could guess.
  defp run("upload", %{"plugin" => plugin}, {_actor, subject}, forward) do
    with %{^plugin => version} <- Plugins.running(subject),
         {installed, ^plugin} <- Plugins.installed(subject, version.artifact) do
      at(installed, forward)
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: "#{installed}/own/#{plugin}/"}
      Map.put(Artifact.link(own, name, :write), :upload, name)
    else
      _ -> raise "There's no plugin named #{plugin}"
    end
  end

  # Whoever a server acts for put it on their list, and its owner chose them. A server with no members yet is a
  # new one, which whoever adds it first is bringing.
  defp run("add_server", %{"key" => key}, {_actor, subject} = principal, forward) do
    at(subject, forward)
    reaches!(principal, subject, "owner")

    Servers.forget()
    members = Servers.members(key)

    if members != [] and subject not in members,
      do: raise("That server is #{hd(members)}'s, who hasn't chosen #{subject}. Its owner does, with add_member.")

    with {:error, why} <- Servers.add(key, subject), do: raise(why)
    Hooks.observe("joined", %{"server" => key, "for" => subject, "by" => Server.key()}, principal)
    Server.notify_servers([subject])
    %{server: key, for: subject}
  end

  # The server taken off is told too, so that it stops acting for them at once.
  defp run("remove_server", %{"key" => key}, {_actor, subject} = principal, forward) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    acting = Servers.acting(subject)
    with {:error, why} <- Servers.remove(key, subject), do: raise(why)
    Server.notify(acting, [subject])
    %{server: key, for: subject}
  end

  defp run("add_member", %{"who" => member} = inputs, principal, _pass) do
    on(inputs["server"], "add_member", inputs, principal, fn ->
      if not User.owner?(member),
        do: raise("A server acts for a user, a group or the network: users/<name>, groups/<name> or network")

      reaches!(principal, owner(), "owner")
      Servers.choose(member)
      Server.notify_servers([member, owner()])
      %{server: Server.key(), member: member}
    end)
  end

  defp run("services", _inputs, {_actor, subject}, _pass) do
    servers = Servers.acting(User.owner(subject) || "network")
    names = Map.new(servers, &{&1, Servers.name(&1)})
    %{server: Server.key(), servers: servers, names: names, services: Services.list(subject)}
  end

  defp run("install_service", %{"plugin" => name, "service" => service} = inputs, {_, subject} = principal, _pass) do
    on(inputs["server"], "install_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.install(plugin(subject, name), service), do: raise(why)
      %{plugin: name, service: service, server: Server.key()}
    end)
  end

  defp run(
         "uninstall_service",
         %{"plugin" => name, "service" => service} = inputs,
         {_actor, subject} = principal,
         _pass
       ) do
    on(inputs["server"], "uninstall_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.remove(plugin(subject, name), service), do: raise(why)
      %{plugin: name, service: service, server: Server.key()}
    end)
  end

  defp run("builds", inputs, {actor, subject} = principal, _pass) do
    on(inputs["server"], "builds", inputs, principal, fn ->
      Builds.list(actor, User.owner(subject) || "network")
    end)
  end

  defp run("download", %{"target" => target}, _principal, _pass), do: Builds.download(target)

  # What the network's servers run is the network's.
  defp run("publish", %{"builds" => builds}, principal, forward) do
    at("network", forward)
    reaches!(principal, "network", "owner")

    case Builds.publish(builds) do
      {:send, links} -> %{send: links}
      {:ok, latest} -> %{latest: latest}
    end
  end

  # What takes the latest builds is the program that keeps an installed server running, which hears this said.
  defp run("update", inputs, principal, _pass) do
    on(inputs["server"], "update", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")

      if not Application.get_env(:one_core, :installed, false),
        do: raise("#{here()} wasn't installed with `one serve on`, so it doesn't update itself.")

      %{server: Server.key(), update: Release.update()}
    end)
  end

  # Raises where a plugin named `name` is installed for someone else whoever asked is in, whose plugins they find
  # too: a new one would answer first for whoever it's deployed for, empty, as a second copy.
  defp elsewhere!({actor, subject}, name) do
    member = &(Hooks.validate("reach", %{"who" => actor, "relation" => "member", "artifact" => &1}, {actor, &1}) == :ok)
    groups = for {"groups/" <> _ = group, %{^name => _}} <- Plugins.installed(), member.(group), do: group
    others = for owner <- Enum.uniq([User.owner(actor), "network" | groups]), owner not in [nil, subject], do: owner

    case for(owner <- others, Plugins.plugin(owner, name), do: owner) do
      [] ->
        :ok

      [owner | _] ->
        to = if owner == actor, do: "without ONE_SUBJECT", else: "with ONE_SUBJECT=#{owner}"

        raise "#{name} is installed for #{owner}, so this would make a second, empty #{name} for #{subject} that answers " <>
                "first. To update the one there is, deploy it #{to}; to make a second one, pass --new."
    end
  end

  # This server, as a person picks it: what its machine is called, or its key where it has no name.
  defp here, do: Servers.name(Server.key()) || Server.key()

  # Whose this server is: who it was started for.
  defp owner, do: hd(Servers.members(Server.key()))

  # Raises unless whoever asked may reach the artifact as `relation`, which the plugins that validate `reach` say.
  # Nothing is asked about what's their own: themselves, and what their name starts.
  defp reaches!(principal, artifact, relation) do
    with {:refuse, why} <- reaches(principal, artifact, relation), do: raise(why)
  end

  defp reaches({actor, _subject} = principal, artifact, relation) do
    if artifact == actor or String.starts_with?(artifact, actor <> "/"),
      do: :ok,
      else: Hooks.validate("reach", %{"who" => actor, "relation" => relation, "artifact" => artifact}, principal)
  end

  # Raises unless whoever asked may read a plugin's code and add it: a `viewer` of it, or a member of a group or
  # the network that it's whoever's or installed for, as the plugins that validate `reach` say. That's VISION's "what's
  # made is open to everyone inside once it's in use": what only its author has is theirs and whoever they share
  # it with, and taking it away from a group takes it away from them. Only its code is opened: what it keeps for
  # anyone is never the plugin's artifact, and is reached as ever.
  defp opens!({actor, _subject} = principal, plugin) do
    with {:refuse, why} <- reaches(principal, plugin, "viewer") do
      in_use =
        for of <- [User.owner(plugin) | Installed.installed_for(plugin)],
            of == "network" or match?("groups/" <> _, of),
            do: of

      member =
        &(Hooks.validate("reach", %{"who" => actor, "relation" => "member", "artifact" => &1}, {actor, &1}) == :ok)

      if not Enum.any?(Enum.uniq(in_use), member), do: raise(why)
    end
  end

  # The plugin that something running for `who` finds by `name`, as its artifact's name.
  defp plugin(who, name) do
    case Plugins.running(who) do
      %{^name => version} -> version.artifact
      _ -> raise "There's no plugin named #{name}"
    end
  end

  # Runs what a tool does on the server it names: here, or by asking that server to, for whoever asked, which
  # ends the call here with its answer. One that's restarting is waited for.
  defp on(server, tool, inputs, {actor, subject}, here) do
    if server in [nil, Server.key()] do
      here.()
    else
      ask = fn ->
        Server.request(server, "tools/call", %{name: tool, arguments: inputs, _meta: %{actor: actor, subject: subject}})
      end

      answered =
        case ask.() do
          :unreachable -> if Servers.back([server]), do: ask.(), else: :unreachable
          answered -> answered
        end

      case answered do
        {:ok, result} -> throw({Call, {:forwarded, result}})
        {:error, _code, why} -> raise why
        down -> raise Server.error_message(down, server)
      end
    end
  end
end
