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
  alias OneCore.Call.State
  alias OneCore.Plugins.{Installed, Calls, Deploy, Lifecycle, Rename, Store}

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

  # Where a plugin is in its life for whoever it's installed for (`OneCore.Plugins.Installed`).
  @state %{
    state: %{
      type: ["string", "null"],
      enum: ["installing", "installed", "updating", "failed", "recently_deleted", nil],
      description:
        "Where it is in its life: installing, installed, updating (also while its trust switches), failed, in Recently Deleted, or null once it's not installed"
    },
    stage: %{
      type: ["string", "null"],
      enum: ["fetching", "building", "starting", nil],
      description: "While installing or updating: fetching its code, building it, or starting it"
    },
    why: %{type: ["string", "null"], description: "Why it failed, if it did"},
    deleted: %{type: ["integer", "null"], description: "When it was uninstalled, in ms since 1970, while it's in Recently Deleted"},
    update: %{
      type: ["object", "null"],
      description:
        "A newer version to take, while it's installed, or null while it's up to date: `available`, or `needs_approval` where it asks for more permissions",
      properties: %{
        state: %{type: "string", enum: ["available", "needs_approval"]},
        version: %{type: ["integer", "null"], description: "The newer version deployed, if it's one"},
        commit: %{type: ["string", "null"], description: "The newer commit at its git source, if it's one"},
        permissions: %{type: "array", items: %{type: "string"}, description: "What it asks for that the version running doesn't"},
        commits: %{
          type: "array",
          description: "The commits since the version running, newest first: read what changed before taking a stranger's",
          items: %{
            type: "object",
            properties: %{commit: %{type: "string"}, subject: %{type: "string"}, author: %{type: "string"}, date: %{type: "string"}}
          }
        },
        why: %{type: ["string", "null"], description: "Why taking it failed last time, if it did; the version before kept running"}
      },
      required: ["state"]
    }
  }

  # What a tool that moves a plugin in its life answers: where it is now, for whoever it's installed for.
  @moved %{
    type: "object",
    properties:
      Map.merge(@state, %{
        name: %{type: "string", description: "The name it's installed by"},
        for: %{type: "string", description: "Who it's installed for"},
        plugin: %{type: ["string", "null"], description: "Its artifact, or null once it's not installed any more"},
        version: %{type: ["integer", "null"], description: "The version that runs, if one does"},
        trusted: %{type: ["boolean", "null"]},
        permissions: @permissions
      }),
    required: ["name", "for", "plugin", "state", "permissions"]
  }

  # A tool that takes only the name a plugin is installed by.
  @named %{
    type: "object",
    properties: %{name: %{type: "string", description: "The name it's installed by"}},
    required: ["name"]
  }

  @tools [
    %{
      name: "deploy",
      task_support: :optional,
      description: """
      Adds a plugin for you, or updates it, from its files.
      The server builds its apps from their source in client/, compiles its Elixir in server/ and runs the plugin at
      once, with its tools listed and called as <name>_<tool>. If either fails, it says why, and the version already
      running stays.
      The skill `building-plugins` says how to write a plugin. It's yours, and is installed for you: asked for a group
      or for the network, it's theirs. Its code runs on a server of yours, so you need one: set one up in Devices, or
      ask whoever may install plugins for everyone to install it for everyone. An untrusted plugin
      (`trusted: false`) runs in a sandbox, a VM of its own on any of the network's servers, so it needs no server of
      yours: it then reaches only what One Core hands it, and the tools and hosts its permissions/0 names. It answers
      with them. One nobody has installed is removed as anything deleted is, by the plugin that keeps what's deleted,
      where there is one (soft-delete).
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
          trusted: %{
            type: "boolean",
            description:
              "Whether it's trusted, or runs in a sandbox. Left out, it's as the version before was, and a new one is trusted"
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
          trusted: %{type: "boolean", description: "Whether it's trusted, or runs in a sandbox"},
          permissions: @permissions,
          warnings: %{type: ["string", "null"], description: "What the compiler warns of, if anything"}
        },
        required: ["name", "version", "trusted", "permissions", "warnings"]
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
      and the network, and those of yours in Recently Deleted. Each with the name it's installed by, who it's
      installed `for`, its artifact (`plugin`), the version that runs for them, where it is in its life (`state`, and
      its `stage` while installing or updating, `why` it failed), whether it's `trusted` or runs in a sandbox, what it
      may do beyond its own (`permissions`), and a newer version to take (`update`). Each owner's install updates on
      its own. With the latest version's `source`, the hash of the files it was deployed from, and, if it was installed
      from a git repo, where from (`from`): its `source`, the `ref` asked for, the `commit`, whether it had changes not
      committed (`dirty`), and the `hash` of the files as installed, which differs from `source` once it's changed since.
      `automatic` is whether updates are taken without asking, for whoever they're listed for. `plugins_check` looks
      for newer versions now, `plugins_update` takes one.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: ["string", "null"], description: "Whoever they're listed for"},
          automatic: %{type: "boolean", description: "Whether their updates are taken without asking, where they may be"},
          plugins: %{
            type: "array",
            items: %{
              type: "object",
              properties: %{
                name: %{type: "string", description: "The name it's installed by"},
                for: %{type: "string", description: "Who it's installed for"},
                plugin: %{type: "string", description: "Its artifact: users/alice/plugins/notes"},
                version: %{type: ["integer", "null"], description: "The version that runs for them"},
                latest: %{type: ["integer", "null"], description: "Its latest version"},
                trusted: %{type: "boolean"},
                state: @state.state,
                stage: @state.stage,
                why: @state.why,
                deleted: @state.deleted,
                update: @state.update,
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
              required: ["name", "for", "plugin", "trusted", "state", "permissions"]
            }
          }
        },
        required: ["for", "automatic", "plugins"]
      }
    },
    %{
      name: "install",
      task_support: :optional,
      description: """
      Adds a plugin someone else deployed for you: asked for a group or for the network, for them. Also tries
      again one that failed to install. It's installing, a stage at a time, until it answers where it is: installed,
      with what it may do beyond its own (its permissions), or failed, saying why.
      Anyone installs for themselves; for the network or a group, its owners and installers do, and anyone else asks
      one of them to, themselves. Its code runs on a server of whoever it's for, unless it's untrusted.
      It's trusted where it's the network's or your own, and a stranger's (another user's or group's) is untrusted
      and runs in a sandbox, unless `trusted` says otherwise, which only someone who may install for them says.
      `one install <owner>/<repo> --plugin <name>` installs one from a git repo instead, as a copy of your own.
      It's the same plugin, at its latest version, with an artifact of its own for whoever it's installed for, and
      updates on its own: a newer version deployed is an update for them (`plugins`). One in use for a group you're
      in or for the network is open to you; one that isn't, whoever it belongs to shares with you first, as a
      viewer. The plugins it uses are installed for you first.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin, by its artifact's name: users/alice/plugins/notes"},
          name: %{type: "string", description: "What to call it. Its own name, if left out"},
          trusted: %{type: "boolean", description: "Whether it's trusted, or runs in a sandbox. Left out, as where it comes from says"}
        },
        required: ["plugin"]
      },
      outputs: @moved
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
      description: """
      Removes a plugin from what's installed for you. It waits in Recently Deleted with what it kept and made for
      you for 30 days, when `plugins_restore` brings it back as it was, and is then removed for good with them. It
      stays whoever's it is.
      """,
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_restore",
      description: "Brings back a plugin of yours from Recently Deleted, as it was, with what it kept and made for you.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_update",
      description: """
      Updates a plugin of yours to the newer version `plugins` offers (Update), or with `approve` takes one that asks
      for more permissions than it has (Approve), which otherwise waits for that. It's updating, a stage at a time,
      while the version before keeps running, until it answers where it is: installed at the new version, or failed,
      saying why, with the version before still running. It stays trusted, or untrusted, as it was. A stranger's
      plugin is never updated without asking: read the commits it brings first (the skill installing-plugins).
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The name it's installed by"},
          approve: %{type: "boolean", description: "Whether it may ask for more permissions than it has"}
        },
        required: ["name"]
      },
      outputs: @moved
    },
    %{
      name: "plugins_decline",
      description: "Leaves an update that asks for more permissions for now (Not now): it stays there to take.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_retry",
      description: "Tries again what failed of a plugin of yours (Try Again): installing it, updating it or switching its trust.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_dismiss",
      description: """
      Dismisses a plugin of yours that failed: one that failed to install is not installed, and one whose update
      failed carries on as it was, its update still there to take.
      """,
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_trust",
      description: """
      Makes a plugin of yours trusted, running on a server of yours with everything you can reach, or untrusted,
      running in a sandbox with only what it's given. It restarts the other way, updating meanwhile. A stranger's
      plugin is made trusted only by someone who may install for you.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The name it's installed by"},
          trusted: %{type: "boolean", description: "Whether it's trusted, or runs in a sandbox"}
        },
        required: ["name", "trusted"]
      },
      outputs: @moved
    },
    %{
      name: "plugins_check",
      description: """
      Looks now for newer versions of your plugins, as servers do once a day: a new commit at the git source of one
      of your own, and a newer version deployed of anyone's. What it finds, `plugins` lists as their update, and one
      that may be taken without asking is taken.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: "string"},
          plugins: %{type: "array", items: @moved, description: "Each, as it is now"}
        },
        required: ["for", "plugins"]
      }
    },
    %{
      name: "plugins_automatic_updates",
      description: """
      Turns on or off taking updates without asking, for you, a group or the network. It's on unless turned off, and
      takes only an update of a trusted plugin that asks for no more permissions, from the network's own plugins or
      your own repos.
      """,
      inputs: %{
        type: "object",
        properties: %{on: %{type: "boolean"}},
        required: ["on"]
      },
      outputs: %{
        type: "object",
        properties: %{for: %{type: "string"}, automatic: %{type: "boolean"}},
        required: ["for", "automatic"]
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
                    "\"on\", \"restarting\" while it stops to start again or starts, until it's up, or when it was last on, in milliseconds since 1970"
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
      task_support: :optional,
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

  # The tools a plugin has no say in for itself: how one that breaks is undone.
  @apart ~w(deploy uninstall rename plugins_restore plugins_update plugins_decline plugins_retry plugins_dismiss plugins_trust)

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
        apart = if name in @apart, do: {User.owner(elem(principal, 1)), inputs["name"]}

        State.run(fn -> Calls.tool(tool, about, principal, nil, &run(name, &1, principal, forward), apart) end)
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
                "Set one up in Devices, ask whoever may install plugins for everyone to install it for everyone, " <>
                "or deploy it untrusted (trusted: false), to run in a sandbox, which any of the network's servers " <>
                "runs for anyone."

      true ->
        forward.({:running, owner})
    end
  end

  # A plugin is whoever's deployed it. Deploying over one is changing it, and a new one is installed for whoever it's
  # deployed for. Either way it's loaded where it will run.
  # An untrusted plugin runs on any server acting for whoever it's for, the network's among them.
  defp run("deploy", %{"name" => name, "files" => files} = inputs, {_actor, subject} = principal, forward) do
    plugin = Plugins.plugin(subject, name)
    was = plugin == nil or Plugins.running(subject)[name].trusted
    trusted = Map.get(inputs, "trusted", was)
    if trusted, do: runs(subject, forward), else: at(subject, forward)

    case plugin do
      nil -> reaches!(principal, subject, "installer")
      plugin -> reaches!(principal, plugin, "editor")
    end

    # Only someone who may install for them makes a stranger's plugin trusted.
    if trusted and not was, do: reaches!(principal, subject, "installer")

    if plugin == nil and inputs["new"] != true, do: elsewhere!(principal, name)

    case Deploy.deploy(subject, name, Store.received(files), Map.get(inputs, "from", :stored), trusted) do
      {:ok, number, warnings} ->
        %{
          name: name,
          version: number,
          trusted: trusted,
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
    owner = User.owner(subject)

    installed =
      for at <- Plugins.chain(subject),
          {records, _automatic} = Installed.read(at),
          {name, record} <- Enum.sort(records),
          at == owner or record["state"] != "recently_deleted",
          do: {at, name, record}

    latest = Bucket.read_all(for {_at, _name, record} <- installed, do: record["plugin"] <> "/latest")

    plugins =
      for {{at, name, record}, read} <- Enum.zip(installed, latest) do
        about = with {:ok, latest, _tag} <- read, do: JSON.decode!(latest), else: (_ -> %{})

        at
        |> moved(name, record)
        |> Map.merge(%{latest: about["number"], source: about["source"], from: about["from"]})
      end

    %{for: owner, automatic: elem(Installed.read(owner || "network"), 1), plugins: plugins}
  end

  defp run("install", %{"plugin" => plugin} = inputs, {_actor, subject} = principal, forward) do
    name = Map.get(inputs, "name", Path.basename(plugin))
    trusted = Map.get(inputs, "trusted", Lifecycle.trusted?(subject, plugin))
    if trusted, do: runs(subject, forward), else: at(subject, forward)
    opens!(principal, plugin)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.install(subject, name, plugin, trusted))
  end

  # It's renamed where it runs, which is where it's deployed again.
  defp run("rename", %{"name" => name, "to" => to}, {_actor, subject} = principal, forward) do
    plugin = "#{subject}/plugins/#{name}"
    if Store.about(plugin)["trusted"] == false, do: at(subject, forward), else: runs(subject, forward)
    reaches!(principal, plugin, "owner")
    with {:error, why} <- Rename.rename(subject, name, to, principal), do: raise(why)
    %{from: name, to: to}
  end

  defp run("uninstall", %{"name" => name}, {_actor, subject} = principal, forward) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.uninstall(subject, name, principal))
  end

  # Restoring is undoing an uninstall, which is an owner's.
  defp run("plugins_restore", %{"name" => name}, {_actor, subject} = principal, forward) do
    record!(subject, name, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.restore(subject, name, principal))
  end

  defp run("plugins_update", %{"name" => name} = inputs, {_actor, subject} = principal, forward) do
    record!(subject, name, forward)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.update(subject, name, inputs["approve"] == true))
  end

  defp run("plugins_decline", %{"name" => name}, {_actor, subject} = principal, forward) do
    record!(subject, name, forward)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.change(subject, name, :decline))
  end

  defp run("plugins_retry", %{"name" => name}, {_actor, subject} = principal, forward) do
    record!(subject, name, forward)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.retry(subject, name))
  end

  defp run("plugins_dismiss", %{"name" => name}, {_actor, subject} = principal, forward) do
    record!(subject, name, forward)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.change(subject, name, :dismiss))
  end

  # Untrusted, it runs on any server acting for them; trusted, on one that runs their plugins.
  defp run("plugins_trust", %{"name" => name, "trusted" => trusted}, {_actor, subject} = principal, forward) do
    if trusted, do: runs(subject, forward), else: at(subject, forward)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.trust(subject, name, trusted))
  end

  defp run("plugins_check", _inputs, {_actor, subject} = principal, forward) do
    owner = User.owner(subject) || "network"
    at(owner, forward)
    reaches!(principal, owner, "installer")
    Lifecycle.reconcile(owner)
    Lifecycle.check(owner)
    {records, _automatic} = Installed.read(owner)
    %{for: owner, plugins: for({name, record} <- Enum.sort(records), do: moved(owner, name, record))}
  end

  defp run("plugins_automatic_updates", %{"on" => on}, {_actor, subject} = principal, forward) do
    owner = User.owner(subject) || "network"
    at(owner, forward)
    reaches!(principal, owner, "installer")
    Installed.automatic(owner, on)
    if on, do: Task.start(fn -> Lifecycle.reconcile(owner) end)
    %{for: owner, automatic: on}
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
        do: raise("#{here()} can't update itself.\n\nIt wasn't installed with `one serve on`, so it doesn't update itself.")

      %{server: Server.key(), update: Release.update()}
    end)
  end

  # Where a plugin is in its life for `owner`, as a tool that moves it answers: `record` is its record, or `nil` once
  # it's not installed any more.
  defp moved(owner, name, record) do
    record = record || %{}
    version = with number when is_integer(number) <- record["version"], do: Plugins.running(owner)[name]
    update = record["update"]

    %{
      name: name,
      for: owner,
      plugin: record["plugin"],
      version: record["version"],
      trusted: record["trusted"],
      state: record["state"],
      stage: record["stage"],
      why: record["why"],
      deleted: record["deleted"],
      update:
        update &&
          %{
            state: update["state"],
            version: update["version"],
            commit: update["commit"],
            permissions: update["permissions"] || [],
            commits: update["commits"] || [],
            why: update["why"]
          },
      permissions: permitted(version)
    }
  end

  # What failed is an error for whoever asked, as it is where it was asked: the record says so too, until it's tried
  # again or dismissed.
  defp moved!(_owner, _name, {:error, why}), do: raise(why)

  defp moved!(_owner, name, %{"state" => "failed", "why" => why}),
    do: raise("#{why}\n\n#{name} failed. Try again (plugins_retry) or dismiss it (plugins_dismiss).")

  defp moved!(owner, name, record), do: moved(owner, name, record)

  # Has a server that runs `owner`'s plugins move the record of the plugin `name`, or any acting for them where it's
  # untrusted, since the version it moves to runs where this moves it.
  defp record!(owner, name, forward) do
    case elem(Installed.read(owner), 0)[name] do
      %{"trusted" => false} -> at(owner, forward)
      _ -> runs(owner, forward)
    end
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
        for of <- [User.owner(plugin) | Installed.in_use_for(plugin)],
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
  # ends the call here with its answer. One that's restarting is asked too, since only it can answer: a build being
  # tried is asked its build this way before it's up. One that can't be reached yet is waited for.
  defp on(server, tool, inputs, {actor, subject}, here) do
    if server in [nil, Server.key()] do
      here.()
    else
      ask = fn ->
        meta = %{actor: actor, subject: subject}
        Server.request(server, "tools/call", %{name: tool, arguments: inputs, _meta: meta}, restarting: true)
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
