defmodule OneCore.Tools.Definitions do
  @moduledoc """
  What each of One Core's own tools says of itself, as an app's `tools/0` does: its name, what it takes and gives,
  and whether it only reads, runs as a task or exempts the app it names (`OneCore.Tools`). It's data alone, so what
  runs beneath the tools, the hooks and calls among them, reads it without depending on what runs them. Kept beside
  each tool's handler instead, as an app's are, it would pull those modules into the cycle `mix compile` checks
  (27 files to 35, 20 Nov 2026).
  """

  # A short-lived link to the bucket, as `OneCore.Bucket.link/2` gives it: sent by its `method` to its `url` with its
  # `headers`.
  # A link to send to: a bucket's presigned one, with the headers it was signed with, or the core's release's, with none.
  @link %{
    type: "object",
    properties: %{
      method: %{type: "string"},
      url: %{type: "string"},
      headers: %{type: "object", additionalProperties: %{type: "string"}}
    },
    required: ["method", "url"]
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
          properties: %{
            commit: %{type: "string"},
            subject: %{type: "string"},
            author: %{type: "string"},
            date: %{type: "string"}
          }
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
      "Whether its version runs on this server's platform, worked out from the two versions: runs or waiting_for_server (it needs a newer platform), and why. Null where no server has described it",
    properties: %{
      state: %{type: "string", enum: ["runs", "waiting_for_server"]},
      why: %{
        type: ["string", "null"],
        description:
          "As a person reads it: gmail waits for the servers' update: it requires core >= 0.21.0, and this server is on 0.20.3"
      }
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
      deleted: %{
        type: ["integer", "null"],
        description: "When it was uninstalled, in ms since 1970, while it's in Recently Deleted"
      },
      version: %{type: ["integer", "null"], description: "The app's version, the same for everyone who has it"},
      trusted: %{type: ["boolean", "null"], description: "Whether the app is trusted, or runs in a sandbox"},
      update: @app_update,
      requires: @requires,
      platform: @platform,
      also: %{
        type: "array",
        items: %{type: "string"},
        description:
          "What else installing it installed, a line each saying why: Gmail uses Google Account to reach your mail, so it's installed too"
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

  @agents [
    %{
      listed: :always,
      name: "skills",
      description: """
      Lists the skills you have and reads one. A skill says when and how to use an app's tools, and One Core's own \
      say how to make an app (building-apps), install one (installing-apps) and write its screens (ui). {} lists \
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
    }
  ]

  @apps [
    %{
      name: "deploy",
      exempts_app: true,
      task_support: :optional,
      description: """
      Adds an app for you, or updates it, from its files.
      The server builds its client from its source in client/, compiles its Elixir in server/ and runs the app at
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
      Where a group or the network has a member's own copy of it, deploying it for them from the same repo makes it
      theirs: the one copy from then on, and the member's goes to Recently Deleted, keeping what they kept.
      `one deploy --files-from <folder>` in a git repo says so too. What's live is a commit: it's refused where that
      commit isn't on the remote's default branch, or has changes not committed (but on a local network being built),
      and, as `git push` refuses a non-fast-forward, where the live version came from a commit that yours doesn't
      contain, unless `--force`.
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
              "The git repo its files came from, if they did: its `source` (owner/repo, a URL or a path), the `ref` asked for, the `commit`, and whether they had changes not committed (`dirty`), which only a network being built takes. Left out, the version before's stays; null says they came from none",
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
      Gives an app's files as they were deployed, its client's source among them, by their paths: what `deploy`
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
                  source: %{
                    type: ["string", "null"],
                    description: "The hash of the files its version was deployed from"
                  },
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
      its own for whoever it's installed for. An app has one copy: installed for a group or the network, it supersedes
      the copies of the same app (the same name from the same git repo) that only they have, whose installs move to
      it, keeping what each person kept, and which go to Recently Deleted. One in use for a group you're in or for the
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
      exempts_app: true,
      description: """
      Removes an app from what's installed for you. It waits in Recently Deleted with what it kept and made for
      you, as one item, for 30 days, when Recently Deleted's restore brings it back as it was (`soft-delete
      restore`), and is then removed for good with them. It stays whoever's it is. Where other apps you have require it, it's refused, saying which and why: they can't
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
      name: "apps_update",
      exempts_app: true,
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
      exempts_app: true,
      task_support: :optional,
      description: "Tries again what failed (Try Again): installing an app of yours, or its update.",
      inputs: @named,
      outputs: @moved
    },
    %{
      name: "apps_trust",
      exempts_app: true,
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
    }
  ]

  @servers [
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
                os: %{
                  type: ["array", "null"],
                  items: %{type: "string"},
                  description: "The systems it runs on, as npm's os field names them: darwin, linux, win32. Null for any"
                },
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
    }
  ]

  @builds [
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
          network: %{
            type: ["object", "null"],
            description:
              "The network's update, as its updater says, if its edge has one: `network_update_status` (up_to_date, " <>
                "preparing, restarting or held), the `build` it takes every server to, how many " <>
                "are `done` `of` those taking part, what it's `waiting_for` (quiet: a lone server waits " <>
                "for no calls a while, or Update Now), why it's `held`, the servers `skipped` as asleep or gone, " <>
                "those `behind` its build, and when each step was reached, in ms after the build was published (`timings`)"
          },
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
      name: "update",
      task_support: :optional,
      description: """
      Software Update's Update Now: has the network's update (`builds`' `network`) take every server to the latest build
      published to the network now, where a lone server waits for a quiet moment before it restarts, or try again
      where it's held. It answers once the network is up to date, or held, saying which server
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

  @doc "Every one of them, as `tools/0` lists them: code mode's first, then the apps', the servers' and the builds'."
  def tools, do: @agents ++ @apps ++ @servers ++ @builds

  @doc "Those an agent finds and runs the others with (`OneCore.Tools.Agents`)."
  def agents, do: @agents

  @doc "Those that deploy, install, update and remove apps (`OneCore.Tools.Apps`)."
  def apps, do: @apps

  @doc "Those that say which servers act for whom and what each runs (`OneCore.Tools.Servers`)."
  def servers, do: @servers

  @doc "Those for the platform's builds and the network's update (`OneCore.Tools.Builds`)."
  def builds, do: @builds
end
