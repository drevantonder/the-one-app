defmodule OneCore.Tools.Apps do
  @moduledoc "One Core's own tools that deploy, install, update and remove apps, and read and upload to them, as `OneCore.Tools` calls them. "
  import OneCore.Tools.Helpers

  alias OneCore.{Artifact, Bucket, Call, Hooks, Apps, User}

  alias OneCore.Apps.{Installed, Calls, Deploy, Lifecycle, Requires, Store, Update, Version}

  @link link()

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

  @tools [
    %{
      name: "deploy",
      exempts_app: true,
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
      Where a group or the network has a member's own copy of it, deploying it for them from the same repo makes it
      theirs: the one copy from then on, and the member's goes to Recently Deleted, keeping what they kept.
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

  # What each takes and gives is read once here, as the schemas its inputs and answers are checked against on every
  # call.
  @taking Enum.map(@tools, &OneCore.Apps.Version.schemas/1)

  @doc "Its tools, each as an app says one of its own."
  def tools, do: @taking

  # An app is whoever's deployed it. Deploying over one is changing it, and a new one is installed for whoever it's
  # deployed for. Either way it's loaded where it will run.
  # An untrusted app runs on any server acting for whoever it's for, the network's among them.
  @doc "Runs its tool `name` with `inputs`, as `OneCore.Tools.call/5` hands it on."
  def run("deploy", %{"name" => name, "files" => files} = inputs, {_actor, subject} = principal, forward, _request) do
    app = Apps.app(subject, name)
    trusted = Map.get(inputs, "trusted", app == nil or Store.trusted?(app))
    runs(subject, trusted, forward)

    case app do
      nil -> reaches!(principal, subject, "installer")
      # Whether it runs trusted, on its servers with their full access, is its owner's to switch, as `apps_trust` has it.
      app -> reaches!(principal, app, if(trusted == Store.trusted?(app), do: "editor", else: "owner"))
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

      {:error, why, details} ->
        throw({Call, {:refused, why, details}})

      {:error, why} ->
        raise why
    end
  end

  def run("code", %{"name" => name}, {_actor, subject} = principal, _pass, _request) do
    case Apps.running(subject) do
      %{^name => version} ->
        opens!(principal, version.artifact)
        %{files: Store.sent(Store.source(version.artifact, version.code))}

      _ ->
        raise "There's no app named #{name}"
    end
  end

  def run("apps", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject)

    installed =
      for at <- Apps.chain(subject),
          {name, record} <- Enum.sort(Installed.read(at)),
          at == owner or record["state"] in ["installing", "installed"],
          do: {at, name, record}

    %{for: owner, apps: listed(installed)}
  end

  def run("install", %{"app" => app} = inputs, {actor, subject} = principal, forward, _request) do
    name = Map.get(inputs, "name", Path.basename(app))
    runs(subject, Store.trusted?(app), forward)
    opens!(principal, app)
    reaches!(principal, subject, "installer")
    moved!(subject, name, Lifecycle.install(subject, name, app, actor))
  end

  def run("uninstall", %{"name" => name} = inputs, {_actor, subject} = principal, forward, _request) do
    at(subject, forward)
    reaches!(principal, subject, "owner")
    moved!(subject, name, Lifecycle.uninstall(subject, name, principal, inputs["confirm"] == true))
  end

  def run("apps_update", %{"name" => name}, {_actor, subject} = principal, forward, _request) do
    app = updates!(principal, subject, name, forward)

    case Lifecycle.update(app) do
      {:error, why} ->
        raise why

      %{"state" => "failed", "why" => why} ->
        raise "#{why}\n\n#{name}'s version before keeps running. Try again (apps_retry)."

      _ ->
        moved!(subject, name, Installed.read(subject)[name])
    end
  end

  # Try Again: installing it, where that failed, or its app's update.
  def run("apps_retry", %{"name" => name}, {_actor, subject} = principal, forward, request) do
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
  def run("apps_trust", %{"name" => name, "trusted" => trusted}, {_actor, subject} = principal, forward, _request) do
    app = record!(subject, name)["app"]
    whose = Store.whose(app)
    runs(whose, trusted, forward)
    reaches!(principal, app, "owner")
    with {:error, why} <- Lifecycle.trust(app, trusted), do: raise(why)
    moved!(subject, name, Installed.read(subject)[name])
  end

  def run("apps_check", _inputs, {_actor, subject}, _pass, _request) do
    owner = User.owner(subject) || "network"
    records = Installed.read(owner)

    apps = for {_name, %{"app" => app, "state" => "installed"}} <- records, Lifecycle.takes?(app), do: app
    Enum.each(Enum.uniq(apps), &Lifecycle.check/1)

    %{for: owner, apps: listed(for {name, record} <- Enum.sort(Installed.read(owner)), do: {owner, name, record})}
  end

  # A file sent this way lands in the app's own artifact, whoever's its calls for them reach (`Calls.whose/3`), under a
  # name nobody could guess.
  def run("upload", %{"app" => app}, {_actor, subject}, forward, _request) do
    with %{^app => version} <- Apps.running(subject),
         {installed, ^app} <- Apps.installed(subject, version.artifact) do
      at(installed, forward)
      name = "uploads/" <> Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)
      own = %Artifact{prefix: "#{Calls.whose(installed, version, subject)}/own/#{app}/"}
      Map.put(Artifact.link(own, name, :write), :upload, name)
    else
      _ -> raise "There's no app named #{app}"
    end
  end

  defp requires(nil), do: %{}

  defp requires(version), do: version.requires

  # Has a server that runs an app installed for `owner`, `trusted` or not, run this call, if this one doesn't, since
  # what it deploys or installs runs there (`OneCore.Apps.runs?/2`): a trusted app on a server that runs their
  # apps, and an untrusted one on any acting for them.
  defp runs(owner, trusted, forward) do
    cond do
      Apps.runs?(owner, trusted) ->
        true

      not trusted ->
        forward.(owner)

      Apps.servers(owner) == [] ->
        raise "An app for #{owner} needs a server of their own, since its code runs there and never on the network's. " <>
                "Set one up in Devices, ask whoever may install apps for everyone to install it for everyone, " <>
                "or deploy it untrusted (trusted: false), to run in a sandbox, which any of the network's servers " <>
                "runs for anyone."

      true ->
        forward.({:running, owner})
    end
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
  defp moved(owner, name, record) do
    app = record && record["app"]
    moved(owner, name, record, nil, app && Update.read(app))
  end

  defp moved(owner, name, record, about, update) do
    record = record || %{}
    app = record["app"]
    about = about || if(app, do: Store.about(app), else: %{})

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

  # The app `name` is for `owner`, whose update whoever asked may take: someone who may install for whoever's the app
  # is, or an editor of it, since its update is for everyone who has it, not only for whoever installed this copy.
  # Its update is deployed where it is: on a server of whoever the app is, which runs their apps where it's trusted.
  defp updates!(principal, owner, name, forward) do
    app = record!(owner, name)["app"]
    whose = Store.whose(app)
    runs(whose, Store.trusted?(app), forward)

    if reaches(principal, whose, "installer") != :ok,
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

  # Raises unless whoever asked may read an app's code and add it: a `viewer` of it, or a member of a group or
  # the network that it's whoever's or installed for, as the apps that validate `reach` say. That's VISION's "what's
  # made is open to everyone inside once it's in use": what only its author has is theirs and whoever they share
  # it with, and taking it away from a group takes it away from them. Only its code is opened: what it keeps for
  # anyone is never the app's artifact, and is reached as ever.
  defp opens!(principal, app), do: with({:refuse, why} <- Lifecycle.opens(principal, app), do: raise(why))
end
