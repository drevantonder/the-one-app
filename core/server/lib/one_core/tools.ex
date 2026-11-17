defmodule OneCore.Tools do
  @moduledoc """
  One Core's own tools: what installs, updates and removes apps, servers and services.

  They're called as an app's tools are, through the same hooks (`OneCore.Apps.Calls.tool/6`): the apps that
  mutate a call's inputs mutate theirs, the ones that validate a call validate theirs, and the ones that observe
  are told. What each changes it reaches as an app reaches an artifact, so the apps that validate `reach` say
  whether whoever asked may: an `editor` of an app deploys over it, a `viewer` reads its code and installs it, and
  so does a `member` of the group or the network it's whoever's or installed for, since what's in use is open to
  everyone it's in use for. An `installer` of a user, a group or the network adds apps for them, which an `owner`
  is, and only an `owner` removes them and says which servers act for them. An app is added only where a server
  would run it (`OneCore.Servers.runs?/1`): for a user or a group, one of their own. A server is its owner's: who it was started for, and only their `owner` says
  who else it acts for and what it runs. Nothing is asked about what's whoever's asked, which is what their name
  starts. With no app validating `reach`, they're the network's first owner's alone.

  What a tool changes is whoever's it is, so a server acting for them runs it: the one asked, or one it passes
  the call on to.
  """
  alias OneCore.{
    Artifact,
    Bucket,
    Builds,
    Call,
    CodeMode,
    Hooks,
    Apps,
    Release,
    Search,
    Server,
    Servers,
    Services,
    Skills,
    User
  }

  alias OneCore.Call.State
  alias OneCore.Apps.{Installed, Calls, Deploy, Lifecycle, Requires, Store, Update, Version}

  # The other apps an app needs, as `requires/0` says them.
  @requires %{
    type: "object",
    additionalProperties: %{type: "string"},
    description: "The other apps it requires, by name, each with why: {\"google-account\": \"to reach your mail\"}"
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

  @server_update_status %{
    type: "string",
    enum: ["up_to_date", "downloading", "verifying"],
    description:
      "Where an update of it is: up_to_date, downloading the latest build and program while it answers on, or " <>
        "verifying them, by loading the build into it or by starting both in its place, kept once it answers as " <>
        "the new build and otherwise gone back from"
  }

  @server_update_stage %{
    type: ["string", "null"],
    enum: ["checking", "loading", "starting", nil],
    description:
      "Where verifying is: checking the build starts and whether it can be loaded into the server while it runs, " <>
        "then loading it, which takes seconds, or starting it in the server's place"
  }

  @update_method %{
    type: ["string", "null"],
    enum: ["hot_load", "restart", nil],
    description:
      "How checking chose to take the build: hot_load, loaded into the server while it runs, or restart, started " <>
        "in its place, for a new runtime, `one`, 0.minor, supervision tree, struct, macro or -on_load module"
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
      app: %{type: "string"},
      service: %{type: "string"},
      server: %{type: "string", description: "The key of the server it runs on, or no longer does"}
    },
    required: ["app", "service", "server"]
  }

  # Where an install is in its life (`OneCore.Apps.Installed`).
  @state %{
    type: ["string", "null"],
    enum: ["installing", "installed", "not_installed", "recently_deleted", nil],
    description:
      "Where it is in its life for whoever it's installed for: installing, installed, not installed where installing failed (why, Try Again), in Recently Deleted, or null once it's gone"
  }

  # Where an app's updates are (`OneCore.Apps.Update`), the same for everyone who has it.
  @app_update %{
    type: ["object", "null"],
    description:
      "The app's update, or null while it's up to date: `available` (an untrusted one's, waiting for Update), `updating`, or `failed` (why, Try Again), its version before still running",
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

  # What a tool that moves an install or an app answers: where it is now, for whoever it's installed for.
  @platform %{
    type: ["object", "null"],
    description:
      "Whether its version runs on this server's platform, worked out from the two versions: runs, waiting_for_server (it needs a newer platform) or waiting_for_app (the server is newer than it allows), and why. Null where no server has described it",
    properties: %{
      state: %{type: "string", enum: ["runs", "waiting_for_server", "waiting_for_app"]},
      why: %{type: ["string", "null"], description: "As a person reads it: gmail needs an update for 0.18"}
    }
  }

  # What a tool that moves an install or an app answers: where it is now, for whoever it's installed for.
  @moved %{
    type: "object",
    properties: %{
      name: %{type: "string", description: "The name it's installed by"},
      for: %{type: "string", description: "Who it's installed for"},
      app: %{type: ["string", "null"], description: "Its artifact, or null once it's not installed any more"},
      state: @state,
      stage: %{
        type: ["string", "null"],
        enum: ["requirements", "fetching", "building", "starting", nil],
        description: "While installing: installing its requirements, fetching its code, building it, or starting it"
      },
      why: %{type: ["string", "null"], description: "Why it isn't installed, where installing failed"},
      deleted: %{type: ["integer", "null"], description: "When it was uninstalled, in ms since 1970, while it's in Recently Deleted"},
      version: %{type: ["integer", "null"], description: "The app's version, the same for everyone who has it"},
      trusted: %{type: ["boolean", "null"], description: "Whether the app is trusted, or runs in a sandbox"},
      update: @app_update,
      requires: @requires,
      platform: @platform,
      also: %{
        type: "array",
        items: %{type: "string"},
        description: "What else installing it installed, a line each saying why: Gmail uses Google Account to reach your mail, so it's installed too"
      }
    },
    required: ["name", "for", "app", "state", "requires"]
  }

  # A tool that takes only the name an app is installed by.
  @named %{
    type: "object",
    properties: %{name: %{type: "string", description: "The name it's installed by"}},
    required: ["name"]
  }

  # One of code mode's calls, as `call` takes it.
  @code_call %{
    type: "object",
    properties: %{
      path: %{type: "string", description: "The tool's path, exactly as search gives it: tools.gmail.search"},
      input: %{type: "object", description: "Its inputs, as its signature says"}
    },
    required: ["path"]
  }

  # Code mode lists only `search`, `execute`, `call` and `skills`, and lists every tool but those three otherwise
  # (`listed`): a client that runs scripts finds the rest by searching.
  @tools [
    %{
      listed: :always,
      name: "skills",
      description: """
      Lists the skills you have and reads one. A skill says when and how to use an app's tools, and One Core's own \
      say how to make an app (building-apps), install one (installing-apps) and write its app (ui). {} lists \
      every skill, each with its app, name, what it's for and its other files; { app: "gmail" } lists one \
      app's; { app: "gmail", name: "gmail" } reads that skill, and { name: "building-apps" } one of One \
      Core's own; add file to read one of its other files.\
      """,
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app the skill is in, by the name its tools start with"},
          name: %{
            type: "string",
            description: "The skill's name, as its list gives it: an app's own is named as the app"
          },
          file: %{type: "string", description: "One of the skill's other files, by its path in the skill's folder"}
        },
        dependentRequired: %{file: ["name"]}
      },
      outputs: %{
        type: "object",
        properties: %{
          skills: %{
            type: "array",
            description: "With no name: the skills, each with its app, name, what it's for and its other files",
            items: %{
              type: "object",
              properties: %{
                app: %{type: "string", description: "The app it's in, or none for One Core's own"},
                name: %{type: "string"},
                description: %{type: "string"},
                files: %{type: "array", items: %{type: "string"}, description: "Its other files, by their paths"}
              },
              required: ["name", "description"]
            }
          },
          text: %{type: "string", description: "With a name: the skill's SKILL.md, or the file asked for"}
        }
      },
      read_only: true
    },
    %{
      listed: :code,
      name: "search",
      description: """
      Finds the tools for a task across every app: give a few words, like "unread email" or "add an event". \
      Answers the best matches, each with its exact call and what it does, then each one's app's skill, and \
      the apps you could install that do it. Search before guessing, to learn an app, and before saying you \
      can't.\
      """,
      inputs: %{
        type: "object",
        properties: %{
          query: %{type: "string", description: "A few words for the task, or an app's name"},
          limit: %{type: "integer", description: "How many tools at most, 8 if left out"}
        },
        required: ["query"]
      },
      outputs: %{
        type: "object",
        properties: %{text: %{type: "string", description: "The matches, their skills and what you could install"}},
        required: ["text"]
      },
      read_only: true
    },
    %{
      listed: :code,
      name: "execute",
      description: """
      Runs a short script that calls tools, and gives back what it returns. It's JavaScript without imports, timers \
      or network: call a tool as `await tools.<app>.<tool>({ ... })`, or One Core's own as `await \
      tools.<tool>({ ... })` (search gives each exact call, and `search({ query })` in the script finds them too), use \
      plain JavaScript between calls, and `return` what you want back; what it logs comes back too. Use it where one \
      call's answer feeds another, or to cut a large answer down; for one call or a few that don't depend on each \
      other, use call. Each call runs as you, as any tool does, so what sends or can't be undone waits for the \
      person's yes, as always. It runs until it's done or cancelled, and gives back up to 60 KB; one that takes long \
      can run as a task. Never put a password, key or token in a script: for one the person pasted or must give, \
      call vault.request, which has them type it on their own device.\
      """,
      task_support: :optional,
      inputs: %{
        type: "object",
        properties: %{code: %{type: "string", description: "The script"}},
        required: ["code"]
      },
      outputs: %{
        type: "object",
        properties: %{text: %{type: "string", description: "What it returned, or what went wrong, with what it logged"}},
        required: ["text"]
      },
      read_only: false
    },
    %{
      listed: :code,
      name: "call",
      description: """
      Calls tools without a script: give one { path, input }, or several in a list, which run at once. Each path is \
      exactly as search gives it, like tools.gmail.search. Answers each in the same order, with ok and its value or \
      its error; a picture comes back as one. Use execute instead where one call's answer feeds another, or to cut a \
      large answer down before it reaches you. Each call runs as you, as any tool does, so what sends or can't be \
      undone waits for the person's yes, as always. Never put a password, key or token in an input: for one the \
      person pasted or must give, call vault.request, which has them type it on their own device.\
      """,
      inputs: %{
        type: "object",
        properties: %{calls: %{anyOf: [@code_call, %{type: "array", items: @code_call, minItems: 1}]}},
        required: ["calls"]
      },
      outputs: %{
        type: "object",
        properties: %{
          calls: %{
            type: "array",
            description: "Each call's answer, in the order asked",
            items: %{
              type: "object",
              properties: %{
                path: %{type: "string"},
                ok: %{type: "boolean"},
                value: %{description: "What it answered, as its signature says"},
                error: %{type: "string", description: "Why it failed, where it did"}
              },
              required: ["path", "ok"]
            }
          }
        },
        required: ["calls"]
      },
      read_only: false
    },
    %{
      name: "deploy",
      task_support: :optional,
      description: """
      Adds an app for you, or updates it, from its files.
      The server builds its apps from their source in client/, compiles its Elixir in server/ and runs the app at
      once, with its tools listed and called as <name>_<tool>. If either fails, it says why, and the version already
      running stays.
      The skill `building-apps` says how to write an app. It's yours, and is installed for you: asked for a group
      or for the network, it's theirs. Its code runs on a server of yours, so you need one: set one up in Devices, or
      ask whoever may install apps for everyone to install it for everyone. An untrusted app
      (`trusted: false`) runs in a sandbox, a VM of its own on any of the network's servers, so it needs no server of
      yours: it then reaches only what One Core hands it, and the tools of the apps its requires/0 names, and no
      website. It answers with those apps. One nobody has installed is removed as anything deleted is, by the app that keeps what's deleted,
      where there is one (soft-delete).
      `one install <owner>/<repo> --app <name>` deploys one from a git repo, saying where it came from (`from`).
      `one deploy --files-from <folder>` in a git repo says so too, and, as `git push` refuses a non-fast-forward, is
      refused where the live version came from a commit that yours doesn't contain, unless `--force`.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The app's name: lowercase letters, digits and hyphens"},
          files:
            Map.put(
              @files,
              :description,
              "The app's files by their paths in it: each one's text, or {\"base64\": ...} for one that isn't text"
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
              "Whether a new app is meant where one by this name is installed for someone else you're in: the network, a group of yours or you. Left out, that's refused, since the new one would answer first, empty"
          }
        },
        required: ["name", "files"]
      },
      outputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The app's name"},
          version: %{type: "integer", description: "The version now running"},
          trusted: %{type: "boolean", description: "Whether it's trusted, or runs in a sandbox"},
          requires: @requires,
          warnings: %{type: ["string", "null"], description: "What the compiler warns of, if anything"},
          listing: %{
            type: "object",
            description:
              "What a person reads of it where it's listed, as the app directory takes it: its title, icon, subtitle, about, category, apps and widgets"
          }
        },
        required: ["name", "version", "trusted", "requires", "warnings", "listing"]
      }
    },
    %{
      name: "code",
      description: """
      Gives an app's files as they were deployed, its app's source among them, by their paths: what `deploy`
      takes, to change and deploy again. Asked for a group or the network, it's one of the apps in use for them,
      which their members may read.
      """,
      read_only: true,
      inputs: %{
        type: "object",
        properties: %{name: %{type: "string", description: "The app's name"}},
        required: ["name"]
      },
      outputs: %{
        type: "object",
        properties: %{files: Map.put(@files, :description, "Its files by their paths, as `deploy` takes them")},
        required: ["files"]
      }
    },
    %{
      name: "apps",
      read_only: true,
      description: """
      Lists the apps installed for you and for the network, as `npm ls` does packages: asked for a group, for it
      and the network, and those of yours in Recently Deleted or that failed to install. Each with the name it's
      installed by, who it's installed `for`, its artifact (`app`), where the install is in its life (`state`, its
      `stage` while installing, `why` it isn't installed), and of the app, the same for everyone who has it: its
      `version`, whether it's `trusted` or runs in a sandbox, the other apps it `requires` and the ones it's
      `required_by`, its `update`, the hash of the files it was deployed from (`source`), and, if it was installed from a git repo, where
      from (`from`): its `source`, the `ref` asked for, the `commit`, whether it had changes not committed (`dirty`),
      and the `hash` of the files as installed, which differs from `source` once it's changed since. A trusted app
      updates by itself; `apps_update` updates an untrusted one. `apps_check` looks for newer versions now.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: ["string", "null"], description: "Whoever they're listed for"},
          apps: %{
            type: "array",
            items:
              Map.update!(@moved, :properties, fn properties ->
                Map.merge(properties, %{
                  required_by: %{
                    type: "array",
                    items: %{type: "string"},
                    description: "The apps installed for the same owner that require it, directly or through another"
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
        required: ["for", "apps"]
      }
    },
    %{
      name: "install",
      task_support: :optional,
      description: """
      Adds an app someone else deployed for you: asked for a group or for the network, for them. Also tries
      again one that failed to install. It's installing, a stage at a time, until it answers: installed, with what
      it requires, or not installed, saying why.
      Anyone installs for themselves; for the network or a group, its owners and installers do, and anyone else asks
      one of them to, themselves. Its code runs on a server of whoever it's for, unless it's untrusted.
      `one install <owner>/<repo> --app <name>` installs one from a git repo instead, as a copy of your own.
      It's the same app, with its one version and whether it's trusted, for everyone who has it, with an artifact of
      its own for whoever it's installed for: a copy is another app. One in use for a group you're in or for the
      network is open to you; one that isn't, whoever it belongs to shares with you first, as a viewer. The apps it
      requires are installed for you first.
      """,
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app, by its artifact's name: users/alice/apps/notes"},
          name: %{type: "string", description: "What to call it. Its own name, if left out"}
        },
        required: ["app"]
      },
      outputs: @moved
    },
    %{
      name: "uninstall",
      description: """
      Removes an app from what's installed for you. It waits in Recently Deleted with what it kept and made for
      you for 30 days, when `apps_restore` brings it back as it was, and is then removed for good with them. It
      stays whoever's it is. Where other apps you have require it, it's refused, saying which and why: they can't
      work without it. Ask the person, and on their yes uninstall it again with `confirm`, which takes them to Recently
      Deleted with it, as one, and Restore brings them all back. What it only requires stays installed.
      """,
      inputs: %{
        type: "object",
        properties: %{
          name: %{type: "string", description: "The name it's installed by"},
          confirm: %{type: "boolean", description: "That the person said yes to the apps that require it going with it"}
        },
        required: ["name"]
      },
      outputs: @moved
    },
    %{
      name: "apps_restore",
      description: "Brings back an app of yours from Recently Deleted, as it was, with what it kept and made for you.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "apps_update",
      task_support: :optional,
      description: """
      Updates an untrusted app you have to the newer commit `apps` offers (Update), for everyone who has it, or
      tries one that failed again. Its version before keeps running while it's updating, and if it fails, saying why.
      Whoever installed it, or someone who may install where it's used, updates it, once an agent has read what
      changed (the skill installing-apps). A trusted app updates by itself; this tries its failed update again.
      """,
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "apps_retry",
      task_support: :optional,
      description: "Tries again what failed (Try Again): installing an app of yours, or its update.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "apps_trust",
      task_support: :optional,
      description: """
      Makes an app trusted, running on servers of whoever has it with everything they can reach, or untrusted,
      running in a sandbox with only what it's given, for everyone who has it: it's deployed again the other way.
      Whoever owns the app switches it.
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
      name: "apps_check",
      description: """
      Looks now for newer versions of the apps you have, as servers do once a day: a new commit at the git source
      each was deployed from. A trusted app takes it at once, and an untrusted one offers it as its `update`.
      """,
      inputs: %{type: "object", properties: %{}},
      outputs: %{
        type: "object",
        properties: %{
          for: %{type: "string"},
          apps: %{type: "array", items: @moved, description: "Each, as it is now"}
        },
        required: ["for", "apps"]
      }
    },
    %{
      name: "upload",
      description: """
      Gives somewhere to send a file that's too large to go in a call: a link that takes it once, within a minute.
      Send the file as the link says, by its `method` to its `url` with its `headers`, then give `upload`, its
      name, to the app's tool that takes one. `one <app> <tool> --<input>-from <path>` does all of this.
      """,
      inputs: %{
        type: "object",
        properties: %{app: %{type: "string", description: "The app whose tool the file is for"}},
        required: ["app"]
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
      It then reads and writes what's yours, runs the apps installed for you, and takes over when another of your
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
      It runs the apps they add on its machine, once they add it with `add_server`.
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
      Lists the services your apps have, what each needs of a machine, and the servers that run it, with
      what each server's machine is called. A service is a part of an app that runs on particular machines,
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
                app: %{type: "string"},
                service: %{type: "string"},
                called: %{type: "string", description: "What it's called where a person reads it"},
                needs: %{type: "string", description: "What it needs of a machine"},
                servers: %{type: "array", items: %{type: "string"}, description: "The keys of the servers running it"}
              },
              required: ["app", "service", "called", "needs", "servers"]
            }
          }
        },
        required: ["server", "servers", "names", "services"]
      }
    },
    %{
      name: "install_service",
      description: """
      Runs an app's service on a server from now on.
      That is the server's owner's to decide, knowing what the service needs of the machine, which `services` says.
      """,
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["app", "service"]
      },
      outputs: @service
    },
    %{
      name: "uninstall_service",
      description: "Stops running an app's service on a server.",
      inputs: %{
        type: "object",
        properties: %{
          app: %{type: "string", description: "The app's name"},
          service: %{type: "string", description: "The service's name, as `services` lists it"},
          server: %{type: "string", description: "The server's key. The server that answers, if left out"}
        },
        required: ["app", "service"]
      },
      outputs: @service
    },
    %{
      name: "builds",
      read_only: true,
      description: """
      Lists the build each of your servers runs, with what its machine is called and whether it's ready: active
      (`server_membership`) and running (`server_daemon_status`), or why not; and the build each of your devices
      last said it is, and the latest for each kind of machine, so it says whether everything is on the same build; and whether its apps are `synced` with what's
      installed or `syncing`, under `apps`.
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
          latest: %{type: "object", description: "The latest build of each kind of machine, by its name: server-runtime/linux-x86_64"},
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
                  enum: ["inactive", "joining", "active", "leaving"],
                  description: "Whether it's one of the network's servers: joining until it answers by its key, and leaving while it drains"
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
                  description: "Why it doesn't talk with the server that answered or the edge, being on another 0.minor"
                },
                server_update_status: @server_update_status,
                server_update_stage: @server_update_stage,
                update_method: @update_method,
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
              properties: %{device: %{type: "string", description: "The device's key"}, build: %{type: "string"}},
              required: ["device"]
            }
          },
          apps: %{
            type: "string",
            enum: ["synced", "syncing"],
            description: "Whether what runs on it is what's installed for everyone it runs apps for, or it's syncing them"
          },
          waiting: %{
            type: "array",
            items: %{type: "object", properties: %{name: %{type: "string"}, state: %{type: "string"}, why: %{type: "string"}}},
            description:
              "The apps installed for you whose version doesn't run on the server that answered: waiting_for_server or waiting_for_app, and why"
          }
        },
        required: ["server", "build", "edge", "latest", "servers", "devices", "apps"]
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
              "Each build: its name (server-runtime/<target>, server-code, one/<target> or " <>
                "one-cli/<target>), build, sha256 and size, and a runtime's inputs, the SHA-256 of what it's built from",
            items: %{
              type: "object",
              properties: %{
                name: %{
                  type: "string",
                  pattern: "^((server-runtime|one|one-cli)/[a-z0-9_]+-[a-z0-9_]+|server-code)$"
                },
                build: %{type: "string"},
                inputs: %{type: "string", pattern: "^[0-9a-f]{64}$"},
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
      Has a server's machine take the latest builds, its server's and its program's together, if it isn't on them
      already. It checks each before it runs it, hands its work to another server meanwhile, and goes back to the
      pair it had if the new one doesn't answer. A build that only changed One Core's code is loaded into the server
      while it runs, in seconds, and it answers once that's done, with the build it's on and, if it went back, why.
      One that must restart (`update_method`) answers once it begins, and one under way already is left to finish:
      `builds` then says it as it moves on, which build it's on once it's done, and if it went back, why.
      """,
      inputs: %{
        type: "object",
        properties: %{server: %{type: "string", description: "The server's key. The server that answers, if left out"}}
      },
      outputs: %{
        type: "object",
        properties: %{
          server: %{type: "string", description: "The key of the server taking them"},
          build: %{type: "string", description: "The build it's on as it answers"},
          server_update_status: @server_update_status,
          server_update_stage: @server_update_stage,
          update_method: @update_method,
          went_back: %{type: ["string", "null"], description: "Why its last update didn't take, until one does"}
        },
        required: ["server", "server_update_status"]
      }
    }
  ]

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Apps.Version.schemas/1)

  # The tools an app has no say in for itself: how one that breaks is undone.
  @apart ~w(deploy uninstall apps_restore apps_update apps_retry apps_trust)

  @doc "One Core's own tools, each as an app says one of its own."
  def tools, do: @taking

  @doc """
  Calls one of One Core's own tools for `asked`, who asked and who for, as an app's tool is called
  (`OneCore.Apps.Calls.tool/6`), hooks and all. `request` is how it was asked: its `mode` (`:mcp`, `:code` or
  `:cli`), `tools`, a function giving the tools a script has, as MCP lists them, and `call`, a function that calls
  one of them by its name and inputs as MCP would, for code mode's own. Gives what `OneCore.Apps.Calls.call/5` does,
  or `:none` if `name` isn't one.
  """
  def call(name, inputs, principal, forwarded, request) do
    case Enum.find(@taking, &(&1.name == name)) do
      nil ->
        :none

      tool ->
        # What's passed on is what was asked, so the server it's passed to hooks into the same thing.
        ask = {"tools/call", %{name: name, arguments: inputs}}
        # To a server acting for someone, or with `{:running, owner}` to one that runs their apps.
        forward = fn
          {:running, owner} -> Call.run_elsewhere(ask, principal, forwarded, owner, "An app for #{owner}")
          owner -> Call.forward(ask, principal, forwarded, owner)
        end

        about = %{"app" => nil, "tool" => name, "inputs" => inputs}
        # An app has no say in its own removal, or in being deployed over: that's how one that breaks is undone.
        apart = if name in @apart, do: {User.owner(elem(principal, 1)), inputs["name"]}

        State.run(fn -> Calls.tool(tool, about, principal, nil, &run(name, &1, principal, forward, request), apart) end)
    end
  end

  defp requires(nil), do: %{}
  defp requires(version), do: version.requires

  # Has a server acting for `owner` run this call, if this one doesn't act for them: `forward` ends it here.
  defp at(owner, forward), do: Servers.here?(owner) || forward.(owner)

  # Has a server that runs an app installed for `owner`, `trusted` or not, run this call, if this one doesn't, since
  # what it deploys or installs runs there (`OneCore.Servers.runs?/2`): a trusted app on a server that runs their
  # apps, and an untrusted one on any acting for them.
  defp runs(owner, trusted, forward) do
    cond do
      Servers.runs?(owner, trusted) ->
        true

      not trusted ->
        forward.(owner)

      Servers.running(owner) == [] ->
        raise "An app for #{owner} needs a server of their own, since its code runs there and never on the network's. " <>
                "Set one up in Devices, ask whoever may install apps for everyone to install it for everyone, " <>
                "or deploy it untrusted (trusted: false), to run in a sandbox, which any of the network's servers " <>
                "runs for anyone."

      true ->
        forward.({:running, owner})
    end
  end

  defp run("skills", inputs, {_actor, subject}, _forward, _request) do
    with {:error, why} <- Skills.run(inputs, subject), do: raise(why), else: ({:ok, read} -> read)
  end

  # What anyone has deployed is read from the bucket, meanwhile.
  defp run("search", %{"query" => query} = inputs, {_actor, subject}, _forward, request) do
    deployed = Task.async(&Deploy.deployed/0)
    tools = request.tools.()

    %{
      text:
        Search.run(
          query,
          Map.get(inputs, "limit", 8),
          tools,
          Apps.running(subject),
          Task.await(deployed, 30_000),
          request.mode
        )
    }
  end

  # A script's calls are each answered here as that call of the tool would be, for whoever asked, and so are
  # `call`'s. An app's tools in a script are in a namespace of its name, which says what the app is for.
  defp run("execute", %{"code" => code}, {_actor, subject}, _forward, request) do
    tools =
      for tool <- request.tools.(),
          do: %{
            name: tool.name,
            app: tool[:_meta][:app],
            description: tool.description,
            input: tool.inputSchema,
            output: tool[:outputSchema]
          }

    namespaces = Map.new(Apps.running(subject), fn {name, version} -> {name, Search.described(version)} end)

    case CodeMode.run(code, tools, namespaces, request.call) do
      {:ok, text} -> %{text: text}
      {:error, text} -> raise text
    end
  end

  defp run("call", %{"calls" => calls}, _principal, _forward, request) do
    # A path as search gives it, or without its `tools.`, as a script's `search()` gives it.
    named =
      Map.new(
        for tool <- request.tools.(),
            dotted = CodeMode.path(tool),
            path <- [Search.path(dotted), dotted],
            do: {path, tool.name}
      )

    CodeMode.calls(calls, named, request.call)
  end

  # An app is whoever's deployed it. Deploying over one is changing it, and a new one is installed for whoever it's
  # deployed for. Either way it's loaded where it will run.
  # An untrusted app runs on any server acting for whoever it's for, the network's among them.
  defp run("deploy", %{"name" => name, "files" => files} = inputs, {_actor, subject} = principal, forward, _request) do
    app = Apps.app(subject, name)
    trusted = Map.get(inputs, "trusted", app == nil or Store.trusted?(app))
    runs(subject, trusted, forward)

    case app do
      nil -> reaches!(principal, subject, "installer")
      app -> reaches!(principal, app, "editor")
    end

    if app == nil and inputs["new"] != true, do: elsewhere!(principal, name)

    case Deploy.deploy(subject, name, Store.received(files), Map.get(inputs, "from", :stored), trusted) do
      {:ok, number, warnings} ->
        %{
          name: name,
          version: number,
          trusted: trusted,
          requires: requires(Apps.running(subject)[name]),
          warnings: if(warnings == "", do: nil, else: warnings),
          listing: Version.listing(Apps.running(subject)[name])
        }

      {:error, why} ->
        raise why
    end
  end

  defp run("code", %{"name" => name}, {_actor, subject} = principal, _pass, _request) do
    case Apps.running(subject) do
      %{^name => version} ->
        opens!(principal, version.artifact)
        %{files: Store.sent(Store.source(version.artifact, version.code))}

      _ ->
        raise "There's no app named #{name}"
    end
  end

  defp run("apps", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject)

    installed =
      for at <- Apps.chain(subject),
          {name, record} <- Enum.sort(Installed.read(at)),
          at == owner or record["state"] in ["installing", "installed"],
          do: {at, name, record}

    %{for: owner, apps: listed(installed)}
  end

  defp run("install", %{"app" => app} = inputs, {actor, subject} = principal, forward, _request) do
    name = Map.get(inputs, "name", Path.basename(app))
    runs(subject, Store.trusted?(app), forward)
    opens!(principal, app)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.install(subject, name, app, actor))
  end

  defp run("uninstall", %{"name" => name} = inputs, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.uninstall(subject, name, principal, inputs["confirm"] == true))
  end

  # Restoring is undoing an uninstall, which is an owner's.
  defp run("apps_restore", %{"name" => name}, {_actor, subject} = principal, forward, _request) do
    record = record!(subject, name)
    runs(subject, Store.trusted?(record["app"]), forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.restore(subject, name, principal))
  end

  defp run("apps_update", %{"name" => name}, {_actor, subject} = principal, forward, _request) do
    app = updates!(principal, subject, name, forward)

    case Lifecycle.update(app) do
      {:error, why} -> raise why
      %{"state" => "failed", "why" => why} -> raise "#{why}\n\n#{name}'s version before keeps running. Try again (apps_retry)."
      _ -> moved!(subject, name, Installed.read(subject)[name])
    end
  end

  # Try Again: installing it, where that failed, or its app's update.
  defp run("apps_retry", %{"name" => name}, {_actor, subject} = principal, forward, request) do
    case record!(subject, name) do
      %{"state" => "not_installed", "app" => app} ->
        runs(subject, Store.trusted?(app), forward)
        reaches!(principal, subject, "installer")
        moved!(subject, name, Lifecycle.retry(subject, name))

      _ ->
        run("apps_update", %{"name" => name}, principal, forward, request)
    end
  end

  # An app's trust is its owner's to switch, for everyone who has it, where it's deployed again.
  defp run("apps_trust", %{"name" => name, "trusted" => trusted}, {_actor, subject} = principal, forward, _request) do
    app = record!(subject, name)["app"]
    whose = Store.whose(app)
    runs(whose, trusted, forward)
    reaches!(principal, app, "owner")
    with {:error, why} <- Lifecycle.trust(app, trusted), do: raise(why)
    moved!(subject, name, Installed.read(subject)[name])
  end

  defp run("apps_check", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject) || "network"
    records = Installed.read(owner)

    apps = for {_name, %{"app" => app, "state" => "installed"}} <- records, Lifecycle.takes?(app), do: app
    Enum.each(Enum.uniq(apps), &Lifecycle.check/1)

    %{for: owner, apps: listed(for {name, record} <- Enum.sort(Installed.read(owner)), do: {owner, name, record})}
  end

  # A file sent this way lands in the app's own artifact, for whoever it was installed for, under a name nobody
  # could guess.
  defp run("upload", %{"app" => app}, {_actor, subject}, forward, _request) do
    with %{^app => version} <- Apps.running(subject),
         {installed, ^app} <- Apps.installed(subject, version.artifact) do
      at(installed, forward)
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: "#{installed}/own/#{app}/"}
      Map.put(Artifact.link(own, name, :write), :upload, name)
    else
      _ -> raise "There's no app named #{app}"
    end
  end

  # Whoever a server acts for put it on their list, and its owner chose them. A server with no members yet is a
  # new one, which whoever adds it first is bringing.
  defp run("add_server", %{"key" => key}, {_actor, subject} = principal, forward, _request) do
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
  defp run("remove_server", %{"key" => key}, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    acting = Servers.acting(subject)
    with {:error, why} <- Servers.remove(key, subject), do: raise(why)
    Server.notify(acting, [subject])
    %{server: key, for: subject}
  end

  defp run("add_member", %{"who" => member} = inputs, principal, _pass, _request) do
    on(inputs["server"], "add_member", inputs, principal, fn ->
      if not User.owner?(member),
        do: raise("A server acts for a user, a group or the network: users/<name>, groups/<name> or network")

      reaches!(principal, owner(), "owner")
      Servers.choose(member)
      Server.notify_servers([member, owner()])
      %{server: Server.key(), member: member}
    end)
  end

  defp run("services", _inputs, {_actor, subject}, _pass, _request) do
    servers = Servers.acting(User.owner(subject) || "network")
    names = Map.new(servers, &{&1, Servers.name(&1)})
    %{server: Server.key(), servers: servers, names: names, services: Services.list(subject)}
  end

  defp run(
         "install_service",
         %{"app" => name, "service" => service} = inputs,
         {_, subject} = principal,
         _pass,
         _request
       ) do
    on(inputs["server"], "install_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.install(app(subject, name), service), do: raise(why)
      %{app: name, service: service, server: Server.key()}
    end)
  end

  defp run(
         "uninstall_service",
         %{"app" => name, "service" => service} = inputs,
         {_actor, subject} = principal,
         _pass,
         _request
       ) do
    on(inputs["server"], "uninstall_service", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")
      with {:error, why} <- Services.remove(app(subject, name), service), do: raise(why)
      %{app: name, service: service, server: Server.key()}
    end)
  end

  defp run("builds", inputs, {actor, subject} = principal, _pass, _request) do
    on(inputs["server"], "builds", inputs, principal, fn ->
      Builds.list(actor, User.owner(subject) || "network")
    end)
  end

  defp run("download", %{"target" => target}, _principal, _pass, _request), do: Builds.download(target)

  # What the network's servers run is the network's.
  defp run("publish", %{"builds" => builds}, principal, forward, _request) do
    at("network", forward)
    reaches!(principal, "network", "owner")

    case Builds.publish(builds) do
      {:send, links} -> %{send: links}
      {:ok, latest} -> %{latest: latest}
    end
  end

  # What takes the latest builds is the program that keeps an installed server running, which hears this said.
  defp run("update", inputs, principal, _pass, _request) do
    on(inputs["server"], "update", inputs, principal, fn ->
      reaches!(principal, owner(), "owner")

      if not Application.get_env(:one_core, :installed, false),
        do: raise("#{here()} can't update itself.\n\nIt wasn't installed with `one serve on`, so it doesn't update itself.")

      status = Release.update()

      %{server: Server.key(), build: OneCore.Builds.build()}
      |> Map.merge(Map.take(status, ~w(server_update_status server_update_stage update_method went_back)))
    end)
  end

  # Each install as `apps` lists it, with its app's version, trust, update and where it came from, read at once.
  defp listed(installed) do
    apps = for {_at, _name, record} <- installed, do: record["app"]
    latest = Bucket.read_all(Enum.map(apps, &(&1 <> "/latest")))
    updates = Bucket.read_all(Enum.map(apps, &(&1 <> "/update")))

    for {{at, name, record}, latest, update} <- Enum.zip([installed, latest, updates]) do
      about = with {:ok, json, _tag} <- latest, do: JSON.decode!(json), else: (_ -> %{})
      update = with {:ok, json, _tag} when json != "" <- update, do: JSON.decode!(json), else: (_ -> nil)

      at
      |> moved(name, record, about, update)
      |> Map.merge(%{source: about["source"], from: about["from"], required_by: Requires.required_by(at, name)})
    end
  end

  # Where an install is in its life for `owner`, with its app's version, trust and update, as a tool that moves
  # either answers: `record` is its record, or `nil` once it's gone.
  defp moved(owner, name, record), do: moved(owner, name, record, nil, :read)

  defp moved(owner, name, record, about, update) do
    record = record || %{}
    app = record["app"]
    about = about || if(app, do: Store.about(app), else: %{})
    update = if update == :read and app, do: Update.read(app), else: if(update == :read, do: nil, else: update)

    %{
      name: name,
      for: owner,
      app: app,
      state: record["state"],
      stage: record["stage"],
      why: record["why"],
      deleted: record["deleted"],
      version: about["number"],
      trusted: if(app, do: Store.trusted?(about)),
      update:
        update &&
          %{
            state: update["state"],
            stage: update["stage"],
            commit: update["commit"],
            commits: update["commits"] || [],
            why: update["why"]
          },
      requires: requires(Apps.running(owner)[name]),
      platform: platform(owner, name, app)
    }
  end

  # Whether an app's version runs on this server's platform: one running here does, and only one that doesn't is
  # read from what it said of itself.
  defp platform(_owner, _name, nil), do: nil

  defp platform(owner, name, app) do
    if is_map_key(Apps.running(owner), name), do: %{state: :runs, why: nil}, else: Store.platform(name, app)
  end

  # What failed is an error for whoever asked, as it is where it was asked: the record says so too, until it's tried
  # again.
  defp moved!(_owner, _name, {:error, why}), do: raise(why)

  defp moved!(_owner, name, %{"state" => "not_installed", "why" => why}),
    do: raise("#{why}\n\n#{name} wasn't installed. Try again (apps_retry).")

  defp moved!(owner, name, record) do
    moved = moved(owner, name, record)
    if also = record["also"], do: Map.put(moved, :also, also), else: moved
  end

  defp record!(owner, name), do: Installed.read(owner)[name] || raise("#{name} isn't installed for #{owner}.")

  # The app `name` is for `owner`, whose update whoever asked may take: whoever installed it, someone who may
  # install where it's used, or an editor of it. Its update is deployed where it is: on a server of whoever the app
  # is, which runs their apps where it's trusted.
  defp updates!({actor, _subject} = principal, owner, name, forward) do
    record = record!(owner, name)
    app = record["app"]
    whose = Store.whose(app)
    runs(whose, Store.trusted?(app), forward)

    if record["by"] != actor and reaches(principal, owner, "installer") != :ok,
      do: reaches!(principal, app, "editor")

    app
  end

  # Raises where an app named `name` is installed for someone else whoever asked is in, whose apps they find
  # too: a new one would answer first for whoever it's deployed for, empty, as a second copy.
  defp elsewhere!({actor, subject}, name) do
    member = &(Hooks.validate("reach", %{"who" => actor, "relation" => "member", "artifact" => &1}, {actor, &1}) == :ok)
    groups = for {"groups/" <> _ = group, %{^name => _}} <- Apps.installed(), member.(group), do: group
    others = for owner <- Enum.uniq([User.owner(actor), "network" | groups]), owner not in [nil, subject], do: owner

    case for(owner <- others, Apps.app(owner, name), do: owner) do
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

  # Raises unless whoever asked may reach the artifact as `relation`, which the apps that validate `reach` say.
  # Nothing is asked about what's their own: themselves, and what their name starts.
  defp reaches!(principal, artifact, relation) do
    with {:refuse, why} <- reaches(principal, artifact, relation), do: raise(why)
  end

  defp reaches({actor, _subject} = principal, artifact, relation) do
    if artifact == actor or String.starts_with?(artifact, actor <> "/"),
      do: :ok,
      else: Hooks.validate("reach", %{"who" => actor, "relation" => relation, "artifact" => artifact}, principal)
  end

  # Raises unless whoever asked may read an app's code and add it: a `viewer` of it, or a member of a group or
  # the network that it's whoever's or installed for, as the apps that validate `reach` say. That's VISION's "what's
  # made is open to everyone inside once it's in use": what only its author has is theirs and whoever they share
  # it with, and taking it away from a group takes it away from them. Only its code is opened: what it keeps for
  # anyone is never the app's artifact, and is reached as ever.
  defp opens!(principal, app), do: with({:refuse, why} <- Lifecycle.opens(principal, app), do: raise(why))

  # The app that something running for `who` finds by `name`, as its artifact's name.
  defp app(who, name) do
    case Apps.running(who) do
      %{^name => version} -> version.artifact
      _ -> raise "There's no app named #{name}"
    end
  end

  # Runs what a tool does on the server it names: here, or by asking that server to, for whoever asked, which
  # ends the call here with its answer. One that isn't ready is asked too, since only it can answer: a build starting
  # is asked its build this way before it's ready. One that's starting or stopping is waited for.
  defp on(server, tool, inputs, principal, here) do
    if server in [nil, Server.key()],
      do: here.(),
      else: Call.run_on({"tools/call", %{name: tool, arguments: inputs}}, principal, server, ready: :any)
  end
end
