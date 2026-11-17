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
  alias OneCore.Plugins.{Installed, Calls, Deploy, Lifecycle, Requires, Store, Update, Version}

  # The other plugins a plugin needs, as `requires/0` says them.
  @requires %{
    type: "object",
    additionalProperties: %{type: "string"},
    description: "The other plugins it requires, by name, each with why: {\"google-account\": \"to reach your mail\"}"
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

  # Where an install is in its life (`OneCore.Plugins.Installed`).
  @state %{
    type: ["string", "null"],
    enum: ["installing", "installed", "not_installed", "recently_deleted", nil],
    description:
      "Where it is in its life for whoever it's installed for: installing, installed, not installed where installing failed (why, Try Again), in Recently Deleted, or null once it's gone"
  }

  # Where a plugin's updates are (`OneCore.Plugins.Update`), the same for everyone who has it.
  @plugin_update %{
    type: ["object", "null"],
    description:
      "The plugin's update, or null while it's up to date: `available` (an untrusted one's, waiting for Update), `updating`, or `failed` (why, Try Again), its version before still running",
    properties: %{
      state: %{type: "string", enum: ["available", "updating", "failed"]},
      stage: %{type: ["string", "null"], enum: ["fetching", "building", "starting", nil]},
      commit: %{type: "string", description: "The newer commit at its git source"},
      commits: %{
        type: "array",
        description: "The commits since the version running, newest first: read what changed before Update",
        items: %{
          type: "object",
          properties: %{commit: %{type: "string"}, subject: %{type: "string"}, author: %{type: "string"}, date: %{type: "string"}}
        }
      },
      why: %{type: ["string", "null"], description: "Why it failed"}
    },
    required: ["state"]
  }

  # What a tool that moves an install or a plugin answers: where it is now, for whoever it's installed for.
  @moved %{
    type: "object",
    properties: %{
      name: %{type: "string", description: "The name it's installed by"},
      for: %{type: "string", description: "Who it's installed for"},
      plugin: %{type: ["string", "null"], description: "Its artifact, or null once it's not installed any more"},
      state: @state,
      stage: %{
        type: ["string", "null"],
        enum: ["requirements", "fetching", "building", "starting", nil],
        description: "While installing: installing its requirements, fetching its code, building it, or starting it"
      },
      why: %{type: ["string", "null"], description: "Why it isn't installed, where installing failed"},
      deleted: %{type: ["integer", "null"], description: "When it was uninstalled, in ms since 1970, while it's in Recently Deleted"},
      version: %{type: ["integer", "null"], description: "The plugin's version, the same for everyone who has it"},
      trusted: %{type: ["boolean", "null"], description: "Whether the plugin is trusted, or runs in a sandbox"},
      update: @plugin_update,
      requires: @requires,
      also: %{
        type: "array",
        items: %{type: "string"},
        description: "What else installing it installed, a line each saying why: Gmail uses Google Account to reach your mail, so it's installed too"
      }
    },
    required: ["name", "for", "plugin", "state", "requires"]
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
      yours: it then reaches only what One Core hands it, and the tools of the plugins its requires/0 names, and no
      website. It answers with those plugins. One nobody has installed is removed as anything deleted is, by the plugin that keeps what's deleted,
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
          requires: @requires,
          warnings: %{type: ["string", "null"], description: "What the compiler warns of, if anything"},
          listing: %{
            type: "object",
            description:
              "What a person reads of it where it's listed, as the plugin directory takes it: its title, icon, subtitle, about, category, apps and widgets"
          }
        },
        required: ["name", "version", "trusted", "requires", "warnings", "listing"]
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
      and the network, and those of yours in Recently Deleted or that failed to install. Each with the name it's
      installed by, who it's installed `for`, its artifact (`plugin`), where the install is in its life (`state`, its
      `stage` while installing, `why` it isn't installed), and of the plugin, the same for everyone who has it: its
      `version`, whether it's `trusted` or runs in a sandbox, the other plugins it `requires` and the ones it's
      `required_by`, its `update`, the hash of the files it was deployed from (`source`), and, if it was installed from a git repo, where
      from (`from`): its `source`, the `ref` asked for, the `commit`, whether it had changes not committed (`dirty`),
      and the `hash` of the files as installed, which differs from `source` once it's changed since. A trusted plugin
      updates by itself; `plugins_update` updates an untrusted one. `plugins_check` looks for newer versions now.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: ["string", "null"], description: "Whoever they're listed for"},
          plugins: %{
            type: "array",
            items:
              Map.update!(@moved, :properties, fn properties ->
                Map.merge(properties, %{
                  required_by: %{
                    type: "array",
                    items: %{type: "string"},
                    description: "The plugins installed for the same owner that require it, directly or through another"
                  },
                  source: %{type: ["string", "null"], description: "The hash of the files its version was deployed from"},
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
                })
              end)
          }
        },
        required: ["for", "plugins"]
      }
    },
    %{
      name: "install",
      task_support: :optional,
      description: """
      Adds a plugin someone else deployed for you: asked for a group or for the network, for them. Also tries
      again one that failed to install. It's installing, a stage at a time, until it answers: installed, with what
      it requires, or not installed, saying why.
      Anyone installs for themselves; for the network or a group, its owners and installers do, and anyone else asks
      one of them to, themselves. Its code runs on a server of whoever it's for, unless it's untrusted.
      `one install <owner>/<repo> --plugin <name>` installs one from a git repo instead, as a copy of your own.
      It's the same plugin, with its one version and whether it's trusted, for everyone who has it, with an artifact of
      its own for whoever it's installed for: a copy is another plugin. One in use for a group you're in or for the
      network is open to you; one that isn't, whoever it belongs to shares with you first, as a viewer. The plugins it
      uses are installed for you first.
      """,
      inputs: %{
        type: "object",
        properties: %{
          plugin: %{type: "string", description: "The plugin, by its artifact's name: users/alice/plugins/notes"},
          name: %{type: "string", description: "What to call it. Its own name, if left out"}
        },
        required: ["plugin"]
      },
      outputs: @moved
    },
    %{
      name: "uninstall",
      description: """
      Removes a plugin from what's installed for you. It waits in Recently Deleted with what it kept and made for
      you for 30 days, when `plugins_restore` brings it back as it was, and is then removed for good with them. It
      stays whoever's it is. Where other plugins you have require it, it's refused, saying which and why: they can't
      work without it. Ask the person, and on their yes uninstall it again with `confirm`, which takes them to Recently
      Deleted with it, as one, and Restore brings them all back. What it only requires stays installed.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The name it's installed by"},
          confirm: %{type: "boolean", description: "That the person said yes to the plugins that require it going with it"}
        },
        required: ["name"]
      },
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
      task_support: :optional,
      description: """
      Updates an untrusted plugin you have to the newer commit `plugins` offers (Update), for everyone who has it, or
      tries one that failed again. Its version before keeps running while it's updating, and if it fails, saying why.
      Whoever installed it, or someone who may install where it's used, updates it, once an agent has read what
      changed (the skill installing-plugins). A trusted plugin updates by itself; this tries its failed update again.
      """,
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_retry",
      task_support: :optional,
      description: "Tries again what failed (Try Again): installing a plugin of yours, or its update.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "plugins_trust",
      task_support: :optional,
      description: """
      Makes a plugin trusted, running on servers of whoever has it with everything they can reach, or untrusted,
      running in a sandbox with only what it's given, for everyone who has it: it's deployed again the other way.
      Whoever owns the plugin switches it.
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
      Looks now for newer versions of the plugins you have, as servers do once a day: a new commit at the git source
      each was deployed from. A trusted plugin takes it at once, and an untrusted one offers it as its `update`.
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
      A build is the version of the commit it was built from, Semantic Versioning's, with its commit's count and
      hash as build metadata: 0.15.2+2974.febd37350abc. Servers, the edge and clients talk only on the same 0.minor:
      `why` says what's on one that doesn't. With `server`, that server answers, saying its own.
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
          edge: %{type: ["string", "null"], description: "The build the network's edge is, if it says"},
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
                why: %{
                  type: "string",
                  description: "Why it doesn't talk with the server that answered or the edge, being on another 0.minor"
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
        required: ["server", "build", "edge", "latest", "servers", "devices", "bundled"]
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
  @apart ~w(deploy uninstall plugins_restore plugins_update plugins_retry plugins_trust)

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

  defp requires(nil), do: %{}
  defp requires(version), do: version.requires

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
    trusted = Map.get(inputs, "trusted", plugin == nil or Store.about(plugin)["trusted"] != false)
    if trusted, do: runs(subject, forward), else: at(subject, forward)

    case plugin do
      nil -> reaches!(principal, subject, "installer")
      plugin -> reaches!(principal, plugin, "editor")
    end

    if plugin == nil and inputs["new"] != true, do: elsewhere!(principal, name)

    case Deploy.deploy(subject, name, Store.received(files), Map.get(inputs, "from", :stored), trusted) do
      {:ok, number, warnings} ->
        %{
          name: name,
          version: number,
          trusted: trusted,
          requires: requires(Plugins.running(subject)[name]),
          warnings: if(warnings == "", do: nil, else: warnings),
          listing: Version.listing(Plugins.running(subject)[name])
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
          {name, record} <- Enum.sort(Installed.read(at)),
          at == owner or record["state"] in ["installing", "installed"],
          do: {at, name, record}

    %{for: owner, plugins: listed(installed)}
  end

  defp run("install", %{"plugin" => plugin} = inputs, {actor, subject} = principal, forward) do
    name = Map.get(inputs, "name", Path.basename(plugin))
    if Store.about(plugin)["trusted"] == false, do: at(subject, forward), else: runs(subject, forward)
    opens!(principal, plugin)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.install(subject, name, plugin, actor))
  end

  defp run("uninstall", %{"name" => name} = inputs, {_actor, subject} = principal, forward) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.uninstall(subject, name, principal, inputs["confirm"] == true))
  end

  # Restoring is undoing an uninstall, which is an owner's.
  defp run("plugins_restore", %{"name" => name}, {_actor, subject} = principal, forward) do
    record = record!(subject, name)
    if Store.about(record["plugin"])["trusted"] == false, do: at(subject, forward), else: runs(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.restore(subject, name, principal))
  end

  defp run("plugins_update", %{"name" => name}, {_actor, subject} = principal, forward) do
    plugin = updates!(principal, subject, name, forward)

    case Lifecycle.update(plugin) do
      {:error, why} -> raise why
      %{"state" => "failed", "why" => why} -> raise "#{why}\n\n#{name}'s version before keeps running. Try again (plugins_retry)."
      _ -> moved!(subject, name, Installed.read(subject)[name])
    end
  end

  # Try Again: installing it, where that failed, or its plugin's update.
  defp run("plugins_retry", %{"name" => name}, {_actor, subject} = principal, forward) do
    case record!(subject, name) do
      %{"state" => "not_installed", "plugin" => plugin} ->
        if Store.about(plugin)["trusted"] == false, do: at(subject, forward), else: runs(subject, forward)
        reaches!(principal, subject, "installer")
        moved!(subject, name, Lifecycle.retry(subject, name))

      _ ->
        run("plugins_update", %{"name" => name}, principal, forward)
    end
  end

  # A plugin's trust is its owner's to switch, for everyone who has it, where it's deployed again.
  defp run("plugins_trust", %{"name" => name, "trusted" => trusted}, {_actor, subject} = principal, forward) do
    plugin = record!(subject, name)["plugin"]
    whose = Store.whose(plugin)
    if trusted, do: runs(whose, forward), else: at(whose, forward)
    reaches!(principal, plugin, "owner")
    with {:error, why} <- Lifecycle.trust(plugin, trusted), do: raise(why)
    moved!(subject, name, Installed.read(subject)[name])
  end

  defp run("plugins_check", _inputs, {_actor, subject}, _pass) do
    owner = User.owner(subject) || "network"
    records = Installed.read(owner)

    plugins = for {_name, %{"plugin" => plugin, "state" => "installed"}} <- records, Lifecycle.takes?(plugin), do: plugin
    Enum.each(Enum.uniq(plugins), &Lifecycle.check/1)

    %{for: owner, plugins: listed(for {name, record} <- Enum.sort(Installed.read(owner)), do: {owner, name, record})}
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

    Servers.invalidate()
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

  # Each install as `plugins` lists it, with its plugin's version, trust, update and where it came from, read at once.
  defp listed(installed) do
    plugins = for {_at, _name, record} <- installed, do: record["plugin"]
    latest = Bucket.read_all(Enum.map(plugins, &(&1 <> "/latest")))
    updates = Bucket.read_all(Enum.map(plugins, &(&1 <> "/update")))

    for {{at, name, record}, latest, update} <- Enum.zip([installed, latest, updates]) do
      about = with {:ok, json, _tag} <- latest, do: JSON.decode!(json), else: (_ -> %{})
      update = with {:ok, json, _tag} when json != "" <- update, do: JSON.decode!(json), else: (_ -> nil)

      at
      |> moved(name, record, about, update)
      |> Map.merge(%{source: about["source"], from: about["from"], required_by: Requires.required_by(at, name)})
    end
  end

  # Where an install is in its life for `owner`, with its plugin's version, trust and update, as a tool that moves
  # either answers: `record` is its record, or `nil` once it's gone.
  defp moved(owner, name, record), do: moved(owner, name, record, nil, :read)

  defp moved(owner, name, record, about, update) do
    record = record || %{}
    plugin = record["plugin"]
    about = about || if(plugin, do: Store.about(plugin), else: %{})
    update = if update == :read and plugin, do: Update.read(plugin), else: if(update == :read, do: nil, else: update)

    %{
      name: name,
      for: owner,
      plugin: plugin,
      state: record["state"],
      stage: record["stage"],
      why: record["why"],
      deleted: record["deleted"],
      version: about["number"],
      trusted: if(plugin, do: about["trusted"] != false),
      update:
        update &&
          %{
            state: update["state"],
            stage: update["stage"],
            commit: update["commit"],
            commits: update["commits"] || [],
            why: update["why"]
          },
      requires: requires(Plugins.running(owner)[name])
    }
  end

  # What failed is an error for whoever asked, as it is where it was asked: the record says so too, until it's tried
  # again.
  defp moved!(_owner, _name, {:error, why}), do: raise(why)

  defp moved!(_owner, name, %{"state" => "not_installed", "why" => why}),
    do: raise("#{why}\n\n#{name} wasn't installed. Try again (plugins_retry).")

  defp moved!(owner, name, record) do
    moved = moved(owner, name, record)
    if also = record["also"], do: Map.put(moved, :also, also), else: moved
  end

  defp record!(owner, name), do: Installed.read(owner)[name] || raise("#{name} isn't installed for #{owner}.")

  # The plugin `name` is for `owner`, whose update whoever asked may take: whoever installed it, someone who may
  # install where it's used, or an editor of it. Its update is deployed where it is: on a server of whoever the plugin
  # is, which runs their plugins where it's trusted.
  defp updates!({actor, _subject} = principal, owner, name, forward) do
    record = record!(owner, name)
    plugin = record["plugin"]
    whose = Store.whose(plugin)
    if Store.about(plugin)["trusted"] == false, do: at(whose, forward), else: runs(whose, forward)

    if record["by"] != actor and reaches(principal, owner, "installer") != :ok,
      do: reaches!(principal, plugin, "editor")

    plugin
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
  defp opens!(principal, plugin), do: with({:refuse, why} <- Lifecycle.opens(principal, plugin), do: raise(why))

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
