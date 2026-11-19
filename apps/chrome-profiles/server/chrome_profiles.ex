defmodule ChromeProfiles do
  @moduledoc """
  Chrome Profiles: browser profiles, like Personal and Work, that a Chrome browser starts from, signed in to what its
  person signed in to.

  A profile is an item in its person's own Vault, `Chrome profile: <name>`, which this app keeps (`keep/4`). Its
  contents are Chrome's whole user data dir as Chrome hands it over, a gzipped tarball without caches, which the
  Vault keeps as 1Password keeps a file (`write_contents/4`): a blob sealed under a key made for that save, which
  only the item holds. The tarball goes from the browser's container to the bucket, and back into another, by
  Containers' `upload` and `download`, a chunk at a time, so a profile of any size is saved and never passes through
  a tool's result. Nor through a hook, which has seconds: a browser starts from a link to it that `chrome.starting`
  hands Chrome, and it's saved by `save`, which quits the browser's Chrome (Chrome's `quit`), sends what it packed,
  then closes it, and says each step as it goes. Nothing answers with a profile or its key. Beside it the item keeps when it was last saved, and
  what it shows of itself, read from the profile as it's saved: the pages it visited last, from its History, and the
  extensions installed in it, from its Extensions and Preferences, each with its name and icon.

  Which browser is on which profile is a note in its person's own artifact of the app's, `browsers/<session>`, written as
  `open` or `update` opens it: Chrome hands a browser that's starting the note's profile, and refuses to close it
  (`chrome.closing`) until it's saved or discarded, unless the note says not to save it. An update's note says when
  it ends: the job `save_updates` saves and closes any past it, once.

  An update's note also holds its `state`, so every app and agent sees the same one. It changes only as `@steps`
  says, and every app watching is told as it does: `starting` (its container made, which may mean building or
  pulling its image), `opening` (the profile going in), `ready`, then `saving` or `discarding`, and the note is gone
  once it's closed. `failed`, with `why`, is one that couldn't start. Each call is one step from the state it finds,
  so a step that isn't one (Save while it's starting, Update while it's saving) is refused, and one a server left
  half done is carried on by the next call or `save_updates`. A save that fails leaves the browser's container as it
  was, with why, so nothing in it is lost, and it's saved once its person tries again. A browser an agent `open`s
  is `ready` from the start and has no end.
  """
  @behaviour OneCore.App
  alias OneCore.{Artifact, Call}

  @item "Chrome profile: "
  @update_minutes 15
  # A save or a close says each stage it reaches, so one at a stage longer than this was lost with the server running
  # it, which says nothing as it goes: `save_updates` carries it on, as it tries again one that couldn't be saved.
  @lost_minutes 30
  @profile %{type: "string", description: "the profile, by its name, like Personal or Work"}
  @browser %{type: "string", description: "a browser you opened on it, by its session; default the one your person is updating it in"}
  # What a profile shows of itself, read from it in its container once Chrome has quit, as it's saved, into @shown as
  # JSON: the 50 pages it visited last, from its History (Chrome counts in microseconds since 1601), and the
  # extensions installed in it, those its Preferences name that are in its Extensions, with their names (a name a
  # manifest gives as __MSG_<key>__ is in its default locale's messages) and icons, 32 px or the nearest larger.
  @shown "/tmp/profile.json"
  @show ~S"""
  cd /tmp && rm -f history.* && cp /root/.chromium/Default/History history.db 2>/dev/null
  cp /root/.chromium/Default/History-wal history.db-wal 2>/dev/null
  sqlite3 -json history.db "select url, title, last_visit_time / 1000 - 11644473600000 as visited from urls where url like 'http%' and hidden = 0 order by last_visit_time desc limit 50" > history.json 2>/dev/null
  node - > profile.json <<'JS'
  const fs = require("fs")
  const at = "/root/.chromium/Default"
  const json = (file) => { try { return JSON.parse(fs.readFileSync(file, "utf8")) } catch { return undefined } }
  const settings = { ...json(`${at}/Preferences`)?.extensions?.settings, ...json(`${at}/Secure Preferences`)?.extensions?.settings }
  const extensions = []
  for (const [id, setting] of Object.entries(settings)) {
    const folder = `${at}/Extensions/${setting.path}`
    const manifest = typeof setting.path === "string" && json(`${folder}/manifest.json`)
    if (!manifest) continue
    let name = String(manifest.name ?? id)
    const message = name.match(/^__MSG_(.+)__$/)
    if (message) {
      const messages = json(`${folder}/_locales/${manifest.default_locale ?? "en"}/messages.json`) ?? {}
      const key = Object.keys(messages).find((key) => key.toLowerCase() === message[1].toLowerCase())
      name = messages[key]?.message ?? name
    }
    const icons = manifest.icons ?? {}
    const sizes = Object.keys(icons).map(Number).sort((a, b) => a - b)
    const size = sizes.find((size) => size >= 32) ?? sizes.at(-1)
    const file = size && `${folder}/${icons[size]}`.replace(/\/+/g, "/")
    let icon = null
    if (file && fs.existsSync(file) && fs.statSync(file).size < 65536)
      icon = `data:image/${file.endsWith(".svg") ? "svg+xml" : "png"};base64,${fs.readFileSync(file).toString("base64")}`
    extensions.push({ id, name, icon })
  }
  extensions.sort((a, b) => a.name.localeCompare(b.name))
  console.log(JSON.stringify({ history: json("/tmp/history.json") ?? [], extensions }))
  JS
  status=$?
  rm -f history.*
  exit $status
  """

  # An update's states, by the state it's in and what happens, as the moduledoc says. `update` is `update` on a
  # browser that's ready, `start` on any other, and `expire` is `save_updates`.
  @steps %{
    {nil, "start"} => "starting",
    {"failed", "start"} => "starting",
    {"ready", "start"} => "starting",
    {"ready", "update"} => "ready",
    {"starting", "opening"} => "opening",
    {"starting", "opened"} => "ready",
    {"opening", "opened"} => "ready",
    {"ready", "opened"} => "ready",
    {"starting", "fail"} => "failed",
    {"opening", "fail"} => "failed",
    {"ready", "save"} => "saving",
    {"ready", "discard"} => "discarding",
    {"failed", "discard"} => "discarding",
    {"saving", "fail"} => "ready",
    {"discarding", "fail"} => "ready",
    {"ready", "expire"} => "saving",
    {"starting", "expire"} => "discarding",
    {"opening", "expire"} => "discarding",
    {"failed", "expire"} => "discarding",
    {"saving", "expire"} => "saving",
    {"discarding", "expire"} => "discarding"
  }
  @states @steps |> Map.values() |> Enum.uniq()

  @impl true
  def requires do
    %{
      "core" => ">= 0.32.0",
      "vault" => "to keep your profiles",
      "containers" => "to move a profile in and out of its browser",
      "chrome" => "to open a browser with a profile",
      "database" => "to save an update by itself once its time is up"
    }
  end

  @impl true
  def hooks, do: ["chrome.starting", "chrome.closing", "vault.types"]

  @impl true
  def jobs, do: [%{tool: "save_updates", every: 60}]

  # It has no app: its profiles are items of a type of the Vault's, which lists them and draws one with this app's
  # component (`vault.types`), so it draws itself.
  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "Your profiles: each one's name, the pages it visited last and the extensions installed in it, as it was last saved, when that was (ms since 1970), and, while your person is updating it, its browser's session and when that ends. Never what's signed in itself.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            profiles: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  profile: %{type: "string"},
                  history: %{
                    type: "array",
                    description: "The pages it visited last, as it was last saved, newest first",
                    items: %{
                      type: "object",
                      properties: %{
                        url: %{type: "string"},
                        title: %{type: "string"},
                        visited: %{type: "integer", description: "When, in milliseconds since 1970"}
                      },
                      required: ["url", "title", "visited"]
                    }
                  },
                  extensions: %{
                    type: "array",
                    description: "The extensions installed in it, as it was last saved",
                    items: %{
                      type: "object",
                      properties: %{
                        id: %{type: "string"},
                        name: %{type: "string"},
                        icon: %{type: ["string", "null"], description: "A data: address"}
                      },
                      required: ["id", "name", "icon"]
                    }
                  },
                  saved: %{type: ["integer", "null"], description: "When it was last saved, in milliseconds since 1970"},
                  updating: %{
                    type: ["object", "null"],
                    description:
                      "While your person is updating it: its browser's session, when that ends, its state, and why, where it went wrong",
                    properties: %{
                      session: %{type: "string"},
                      until: %{type: "integer"},
                      stage: %{type: ["string", "null"], description: "What it's doing while it's saving or closing"},
                      state: %{type: "string", enum: @states},
                      why: %{type: ["string", "null"]}
                    },
                    required: ["session", "until", "state", "why"]
                  }
                },
                required: ["profile", "history", "extensions", "saved", "updating"]
              }
            }
          },
          required: ["profiles"]
        }
      },
      %{
        name: "create",
        description: "Create a profile, signed in to nothing yet.",
        inputs: %{
          type: "object",
          required: ["name"],
          properties: %{name: %{type: "string", minLength: 1, maxLength: 40, description: "like Personal or Work"}}
        },
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "rename",
        description: "Rename a profile.",
        inputs: %{type: "object", required: ["profile", "name"], properties: %{profile: @profile, name: %{type: "string", minLength: 1, maxLength: 40}}},
        outputs: %{type: "object", properties: %{profile: @profile, was: %{type: "string"}}, required: ["profile", "was"]}
      },
      %{
        name: "remove",
        description: "Delete a profile and everything signed in on it: it's in Recently Deleted for 30 days, where it can be restored, then gone.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "open",
        description:
          "Open a Chrome browser on a profile: it starts as the profile was last saved, signed in to what it is. Then use Chrome's tools with its session. `save` with its session saves it back to the profile and closes it, and `discard` closes it without saving; with `save` false, Chrome's `close` does too. Two browsers on one profile each start from its last save, and the last saved is what's kept.",
        inputs: %{
          type: "object",
          required: ["profile", "session", "description"],
          properties: %{
            profile: @profile,
            session: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the browser's name, which Chrome's tools take"},
            description: %{type: "string", description: "what it's for, in a few words"},
            url: %{type: "string", description: "the page to open"},
            save: %{type: "boolean", description: "whether it's saved back to the profile before it's closed; default true. false for a throwaway"}
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            profile: @profile,
            session: %{type: "string", description: "What Chrome's tools take"},
            text: %{type: "string", description: "What navigating to url said, if it was given"}
          },
          required: ["profile", "session"]
        }
      },
      %{
        name: "update",
        description:
          "Open a profile's browser for your person, in Vault on any device of theirs, to sign in to sites themselves, or anything else they'd do in their browser. Never sign in for them. It's saved when they tap Save there, or after #{@update_minutes} minutes.",
        inputs: %{
          type: "object",
          required: ["profile"],
          properties: %{profile: @profile, url: %{type: "string", description: "a page to open it at, like a site's sign-in page; default a new tab"}}
        },
        outputs: %{
          type: "object",
          properties: %{
            profile: @profile,
            session: %{type: "string", description: "Its browser, which you don't use while they do"},
            until: %{type: "integer", description: "When it's saved by itself, in milliseconds since 1970"},
            then: %{type: "string", description: "What to tell them"}
          },
          required: ["profile", "session", "until", "then"]
        }
      },
      %{
        name: "save",
        task_support: :optional,
        description:
          "Save a profile's browser back to it, and close the browser: the one your person is updating it in, or one you opened on it, by its session. A profile of hundreds of MB takes a minute or so. One that couldn't be saved stays open, with nothing lost, and saving it again tries again.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile, session: @browser}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "discard",
        description:
          "Close a profile's browser without saving it: the one your person is updating it in, or one you opened on it, by its session. The profile stays as it was last saved.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile, session: @browser}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "save_updates",
        task_support: :optional,
        description: "Save and close the updates past their #{@update_minutes} minutes. It's run every minute.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{saved: %{type: "array", items: %{type: "string"}, description: "The browsers saved and closed"}},
          required: ["saved"]
        }
      }
    ]
  end

  @impl true
  def call("list", _inputs, call) do
    updating =
      for {session, %{"update" => until} = note} <- notes(call),
          until,
          into: %{},
          do: {note["profile"], %{session: session, until: until, state: note["state"], stage: note["stage"], why: note["why"]}}

    profiles =
      for name <- profiles(call) do
        fields = vault(call).values(call, @item <> name)

        # One never saved has nothing to show yet.
        %{
          profile: name,
          history: JSON.decode!(fields["history"] || "[]"),
          extensions: JSON.decode!(fields["extensions"] || "[]"),
          saved: if(fields["saved"] in [nil, ""], do: nil, else: String.to_integer(fields["saved"])),
          updating: updating[name]
        }
      end

    %{profiles: profiles}
  end

  def call("create", %{"name" => name}, call) do
    name = String.trim(name)
    if name == "", do: raise("A profile has a name")
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(name))), do: raise("You have a profile called #{name} already")
    vault(call).keep(call, @item <> name, %{})
    %{profile: name}
  end

  def call("rename", %{"profile" => from, "name" => to}, call) do
    from = profile!(call, from)
    to = String.trim(to)
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(to) and &1 != from)), do: raise("You have a profile called #{to} already")
    vault(call).rename(call, @item <> from, @item <> to)
    for {session, %{"profile" => ^from}} <- notes(call), do: note(call, session, &%{&1 | "profile" => to})
    %{profile: to, was: from}
  end

  def call("remove", %{"profile" => name}, call) do
    name = profile!(call, name)
    vault(call).remove(call, @item <> name)
    %{profile: name}
  end

  def call("open", %{"profile" => name, "session" => session} = inputs, call) do
    name = profile!(call, name)
    open!(call, session, name)
    note(call, session, fn _ -> %{"profile" => name, "save" => Map.get(inputs, "save", true), "update" => nil, "state" => "ready"} end)
    opened = Call.tool(call, "chrome", "open", Map.take(inputs, ~w(session description url)))
    Map.put(opened, "profile", name)
  end

  def call("update", %{"profile" => name} = inputs, call) do
    name = profile!(call, name)
    session = update_session(name)
    running = open!(call, session, name)
    until = System.system_time(:millisecond) + @update_minutes * 60_000
    changes = %{"profile" => name, "save" => true, "update" => until, "why" => nil}
    # Only a browser that's ready is carried on with as it is: any other (one a carry left half done) starts.
    event = if running and match?(%{"state" => "ready"}, note(call, session)), do: "update", else: "start"
    %{"update" => until, "state" => state} = step(call, session, event, changes)
    opening = Map.merge(%{"session" => session, "description" => "Updating #{name}"}, Map.take(inputs, ["url"]))

    try do
      Call.tool(call, "chrome", "open", opening)
    rescue
      # One that was running already is as it was: the caller hears why.
      e ->
        if state != "ready", do: step(call, session, "fail", %{"why" => Exception.message(e)})
        reraise e, __STACKTRACE__
    end

    step(call, session, "opened")

    %{
      profile: name,
      session: session,
      until: until,
      then:
        "It's open for your person, in Vault on any device of theirs, where the profile says Updating under Chrome Profiles. " <>
          "Ask them to open it there (notifications_send reaches their devices) and sign in to what they want, in its own address bar and tabs. " <>
          "It's saved when they tap Save, or in #{@update_minutes} minutes. Don't sign in for them, and don't use its browser while they do."
    }
  end

  def call(tool, %{"profile" => name} = inputs, call) when tool in ~w(save discard) do
    name = profile!(call, name)
    close(call, name, inputs["session"] || update_session(name), tool)
    %{profile: name}
  end

  # One past its end is saved, or closed if it never started. One that couldn't be saved waits for its person to try
  # again, and is tried again only once it hasn't moved in @lost_minutes, as one a server lost midway is.
  def call("save_updates", _inputs, call) do
    now = System.system_time(:millisecond)
    lost = now - @lost_minutes * 60_000

    saved =
      for {session, %{"update" => until, "profile" => name, "state" => state} = note} <- notes(call),
          until && until < now,
          state in ~w(starting opening failed) or (state == "ready" and note["why"] == nil) or (note["at"] || 0) < lost do
        close(call, name, session, "expire")
        session
      end

    %{saved: saved}
  end

  # A profile is an item of the Vault's, in a section of its own, drawn by `client/profile.tsx`.
  @impl true
  def hook("vault.types", _about, _call),
    do: [
      %{
        name: "Chrome profile",
        section: "Chrome Profiles",
        prefix: @item,
        component: "profile",
        create: %{tool: "create", name: true}
      }
    ]

  # A browser on a profile is starting: it starts from the profile's last save, if it has one, which Chrome fetches
  # by the link and key handed it.
  def hook("chrome.starting", %{"session" => session}, call) do
    if match?(%{"state" => "starting"}, note(call, session)), do: step(call, session, "opening")

    with %{"profile" => name} <- note(call, session),
         true <- name in profiles(call),
         %{} = user_data <- vault(call).read_contents(call, @item <> name, "profile", &Map.take(&1, ["link", "key"])) do
      %{"user_data" => user_data}
    else
      _ -> %{}
    end
  end

  # A browser on a profile is closing: one whose note says it's saved closes only by `save` or `discard`, so what's in
  # it isn't lost, and its note goes as it does.
  def hook("chrome.closing", %{"session" => session}, call) do
    case note(call, session) do
      %{"profile" => name, "save" => true} ->
        if name in profiles(call),
          do:
            {:refuse,
             "#{session} is on the profile #{name}, which keeps what it closes with.\n\n" <>
               "Chrome Profiles' `save` (profile #{name}, session #{session}) saves it and closes it, and `discard` closes it without saving."},
          else: forget(call, session)

      _ ->
        forget(call, session)
    end
  end

  # Saves a profile from its browser, saying each stage as it goes: Chrome quit and the user data dir packed, then sent
  # as its contents in the Vault, which keeps the last save until this one is whole, then when it was saved and what
  # it shows of itself.
  defp save(call, name, session) do
    stage(call, session, "Quitting Chrome")
    %{"container" => container, "user_data" => tarball, "size" => size} = Call.tool(call, "chrome", "quit", %{"session" => session})
    stage(call, session, "Saving #{Float.round(size / 1_000_000, 1)} MB to your Vault")
    vault(call).write_contents(call, @item <> name, "profile", &upload(call, container, tarball, &1)["size"])
    stage(call, session, "Reading its history and extensions")
    ran = Call.tool(call, "containers", "exec", %{"container" => container, "command" => @show, "timeout" => 60})
    if ran["exit"] != 0, do: raise("Couldn't read its history and extensions: #{String.trim(ran["stderr"])}")
    shown = JSON.decode!(Call.tool(call, "containers", "read_file", %{"container" => container, "path" => @shown})["text"])

    vault(call).keep(call, @item <> name, %{
      "saved" => Integer.to_string(System.system_time(:millisecond)),
      "history" => JSON.encode!(shown["history"]),
      "extensions" => JSON.encode!(shown["extensions"])
    })
  end

  # Containers' `upload` of a profile's tarball, in a container, by what the Vault hands over.
  defp upload(call, container, path, %{"link" => link, "key" => key}),
    do: Call.tool(call, "containers", "upload", %{"container" => container, "path" => path, "link" => link, "key" => key})

  # The browser a person updates a profile in.
  defp update_session(name), do: "update-" <> (name |> String.downcase() |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-"))

  # Saves or discards a browser on a profile (`event`), as a step to `saving` or `discarding`, then closes it. One that
  # couldn't be saved or closed is ready again, saying why, and kept as it is.
  defp close(call, name, session, event) do
    if not match?(%{"profile" => ^name}, note(call, session)) do
      raise "#{name} has no browser #{if session == update_session(name), do: "your person is updating it in", else: "called #{session}"}"
    end

    %{"state" => state} = step(call, session, event)

    try do
      if state == "saving", do: save(call, name, session)
      stage(call, session, "Closing its browser")
      step(call, session, nil, %{"save" => false})
      Call.tool(call, "chrome", "close", %{"session" => session})
    rescue
      e ->
        if note(call, session), do: step(call, session, "fail", %{"why" => Exception.message(e)})
        reraise e, __STACKTRACE__
    end

    # A browser that was gone already closes without `chrome.closing`, so its note goes here.
    forget(call, session)
  end

  # Says what a save or a close is doing: to its caller, and in its note, for its person.
  defp stage(call, session, text) do
    Call.stage(call, text)
    step(call, session, nil, %{"stage" => text})
  end

  # Takes an update one step (`event`, by `@steps`) from the state its note is in, with `changes` to the note, and
  # gives the note. A `nil` event only changes the note.
  defp step(call, session, event, changes \\ %{}) do
    note =
      note(call, session, fn note ->
        was = note && note["state"]

        state =
          cond do
            event == nil -> was
            to = @steps[{was, event}] -> to
            true -> raise "#{(note || changes)["profile"]} can't #{event} while it's #{was || "not being updated"}"
          end

        # A stage is of the state it's said in, and when it was said is when the note last moved.
        note = if event, do: Map.delete(note || %{}, "stage"), else: note || %{}
        note = Map.merge(note, changes) |> Map.merge(%{"state" => state, "at" => System.system_time(:millisecond)})
        if state in ~w(starting ready saving discarding) and event != "fail", do: Map.put(note, "why", nil), else: note
      end)

    shown(call, note)
    Call.changed(call)
    note
  end

  # Which of the Vault's three states the profile is in, from its update's own: busy while its browser starts, is
  # open for its person, saves or closes; needs_action, with Try Again, where it couldn't start or couldn't save;
  # ready with no update. Its person isn't notified: they're in the update, which shows what went wrong.
  defp shown(call, %{"profile" => name, "update" => until} = note) when until != nil do
    {state, message, action} =
      case note do
        %{"state" => "failed"} -> {:needs_action, "Couldn't start its browser", %{label: "Try Again", tool: "update", inputs: %{profile: name}}}
        %{"state" => "ready", "why" => why} when why != nil -> {:needs_action, "Couldn't save", %{label: "Try Again", tool: "save", inputs: %{profile: name}}}
        %{"state" => "ready"} -> {:busy, "Updating", nil}
        %{"state" => "saving"} -> {:busy, note["stage"] || "Saving…", nil}
        %{"state" => "discarding"} -> {:busy, "Closing…", nil}
        %{"state" => _starting} -> {:busy, "Starting…", nil}
      end

    if name in profiles(call), do: vault(call).state(call, @item <> name, state, message, action)
  end

  defp shown(_call, _note), do: :ok

  # A browser that's open already is only carried on with on the same profile: Chrome hands it a profile as it starts.
  # Whether it's running.
  defp open!(call, session, name) do
    running = Enum.find(Call.tool(call, "chrome", "list", %{})["browsers"], &(&1["session"] == session and &1["state"] == "running"))

    case {running, note(call, session)} do
      {nil, _} -> false
      {_, %{"profile" => ^name}} -> true
      _ -> raise "#{session} is open already, not on #{name}.\n\nChrome's `close` closes it, or open another session."
    end
  end

  # The profiles' names, by their items in whoever this call is for's Vault, which this app keeps.
  defp profiles(call), do: for(%{name: @item <> name} <- vault(call).items(call), do: name)

  defp profile!(call, name) do
    Enum.find(profiles(call), &(String.downcase(&1) == String.downcase(String.trim(name)))) ||
      raise "You have no profile called #{name}.\n\n`list` shows yours, and `create` makes one."
  end

  defp vault(call), do: Call.app(call, "vault")

  # Which profile each of whoever this call is for's browsers is on, by its session.
  defp notes(call) do
    prefix = "browsers/"
    names = Artifact.list(call.own, prefix)
    for {name, {:ok, data, _tag}} <- Enum.zip(names, Artifact.read_all(call.own, names)), do: {String.replace_prefix(name, prefix, ""), JSON.decode!(data)}
  end

  defp note(call, session) do
    case Artifact.read(call.own, at(session)) do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> nil
    end
  end

  # Changes the session's note in one step, and gives it: `change` is given it as it is now, or `nil`.
  defp note(call, session, change) do
    JSON.decode!(Artifact.update(call.own, at(session), &JSON.encode!(change.(&1 && JSON.decode!(&1)))))
  end

  # A browser's note goes as it closes, and its profile is ready again.
  defp forget(call, session) do
    note = note(call, session)
    Artifact.remove(call.own, at(session))
    with %{"profile" => name, "update" => until} when until != nil <- note, true <- name in profiles(call), do: vault(call).state(call, @item <> name, :ready)
    :ok
  end
  defp at(session), do: "browsers/#{session}"
end
