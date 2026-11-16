defmodule ChromeProfiles do
  @moduledoc """
  Chrome Profiles: browser profiles, like Personal and Work, that a Chrome browser starts from, signed in to what its
  person signed in to.

  A profile is Chrome's whole user data dir, as Chrome hands it over: a gzipped tarball without caches. It's kept
  as a blob of an artifact this plugin makes for its person (`profiles`), named at random, encrypted with AES-256-GCM
  under a key made for that save. The key, the blob's name, the sites it was used on with when, and when it was last saved
  are an item in the person's own Vault, `Chrome profile: <name>`, which this plugin keeps (`keep/4`), each a secret
  field: so only the servers its person chose for their Vault open a profile, and the bucket holds only what's
  encrypted. Nothing answers with a profile or its key: they go only into the person's own browser, as
  `chrome.starting` is answered.

  Which browser is on which profile is a note in the plugin's own artifact, `browsers/<whose>/<session>`, written as
  `open` or `update` opens it: Chrome hands a browser that's starting the note's profile, and as it closes
  (`chrome.closing`) what it closed with is saved to it, unless the note says not to. An update's note says when it
  ends: the job `save_updates` saves and closes any past it.

  An update's note also holds its `state`, so every app and agent sees the same one. It changes only as `@steps`
  says, and every app watching is told as it does: `starting` (its container made, which may mean building or
  pulling its image), `opening` (the profile going in), `ready`, then `saving` or `discarding`, and the note is gone
  once it's closed. `failed`, with `why`, is one that couldn't start. Each call is one step from the state it finds,
  so a step that isn't one (Save while it's starting, Update while it's saving) is refused, and one a server left
  half done is carried on by the next call or `save_updates`.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @item "Chrome profile: "
  @update_minutes 15
  # Where a profile is handed over in a browser's container, as Chrome's hooks say.
  @tarball "/tmp/user-data.tar.gz"
  @profile %{type: "string", description: "the profile, by its name, like Personal or Work"}

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
  def uses, do: ["vault", "containers", "chrome"]

  @impl true
  def hooks, do: ["chrome.starting", "chrome.closing"]

  @impl true
  def jobs, do: [%{tool: "save_updates", every: 60}]

  @impl true
  def apps, do: [%{name: "Chrome Profiles", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "Your profiles: each one's name, the sites it was used on with when (ms since 1970), newest first, when it was last saved, and, while your person is updating it, its browser's session and when that ends. Never what's signed in itself.",
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
                  sites: %{
                    type: "array",
                    description: "The sites it was used on, newest first",
                    items: %{
                      type: "object",
                      properties: %{site: %{type: "string"}, used: %{type: "integer", description: "When, in milliseconds since 1970"}},
                      required: ["site", "used"]
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
                      state: %{type: "string", enum: @states},
                      why: %{type: ["string", "null"]}
                    },
                    required: ["session", "until", "state", "why"]
                  }
                },
                required: ["profile", "sites", "saved", "updating"]
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
        description: "Remove a profile and everything signed in on it, for good.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "open",
        description:
          "Open a Chrome browser on a profile: it starts as the profile was last saved, signed in to what it is. Then use Chrome's tools with its session. Chrome's `close` saves it back to the profile, unless `save` is false. Two browsers on one profile each start from its last save, and the last to close is what's kept.",
        inputs: %{
          type: "object",
          required: ["profile", "session", "description"],
          properties: %{
            profile: @profile,
            session: %{type: "string", pattern: "^[a-z0-9][a-z0-9_.-]*$", description: "the browser's name, which Chrome's tools take"},
            description: %{type: "string", description: "what it's for, in a few words"},
            url: %{type: "string", description: "the page to open"},
            save: %{type: "boolean", description: "save it back to the profile on close; default true. false for a throwaway"}
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
          "Open a profile's browser for your person, in the Chrome Profiles app on any device of theirs, to sign in to sites themselves, or anything else they'd do in their browser. Never sign in for them. It's saved when they tap Save there, or after #{@update_minutes} minutes.",
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
        description: "Save a profile your person is updating, and close its browser.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "discard",
        description: "Close the browser your person is updating a profile in, without saving it: the profile stays as it was last saved.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile}},
        outputs: %{type: "object", properties: %{profile: @profile}, required: ["profile"]}
      },
      %{
        name: "save_updates",
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
          do: {note["profile"], %{session: session, until: until, state: note["state"], why: note["why"]}}

    profiles =
      for name <- profiles(call) do
        sites = JSON.decode!(value(call, name, "sites"))
        saved = value(call, name, "saved")

        %{
          profile: name,
          sites: sites |> Enum.sort_by(fn {_site, used} -> -used end) |> Enum.map(fn {site, used} -> %{site: site, used: used} end),
          saved: if(saved == "", do: nil, else: String.to_integer(saved)),
          updating: updating[name]
        }
      end

    %{profiles: profiles}
  end

  def call("create", %{"name" => name}, call) do
    name = String.trim(name)
    if name == "", do: raise("A profile has a name")
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(name))), do: raise("You have a profile called #{name} already")
    vault(call).keep(call, @item <> name, %{"sites" => "{}", "saved" => ""})
    %{profile: name}
  end

  def call("rename", %{"profile" => from, "name" => to}, call) do
    from = profile!(call, from)
    to = String.trim(to)
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(to) and &1 != from)), do: raise("You have a profile called #{to} already")
    names = if value(call, from, "saved") == "", do: ~w(sites saved), else: ~w(key blob sites saved)
    fields = Map.new(names, &{&1, value(call, from, &1)})
    vault(call).keep(call, @item <> to, fields)
    vault(call).remove(call, @item <> from)
    for {session, %{"profile" => ^from} = note} <- notes(call), do: note(call, session, %{note | "profile" => to})
    %{profile: to, was: from}
  end

  def call("remove", %{"profile" => name}, call) do
    name = profile!(call, name)
    blob = if value(call, name, "saved") != "", do: value(call, name, "blob")
    vault(call).remove(call, @item <> name)
    if blob, do: Artifact.remove(saved(call), blob)
    %{profile: name}
  end

  def call("open", %{"profile" => name, "session" => session} = inputs, call) do
    name = profile!(call, name)
    open!(call, session, name)
    note(call, session, %{"profile" => name, "save" => Map.get(inputs, "save", true), "update" => nil})
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
        "It's open for your person, in the Chrome Profiles app on any device of theirs, where the profile says Updating. " <>
          "Ask them to open it there (notifications_send with app chrome-profiles reaches their devices) and sign in to what they want, in its own address bar and tabs. " <>
          "It's saved when they tap Save, or in #{@update_minutes} minutes. Don't sign in for them, and don't use its browser while they do."
    }
  end

  def call("save", %{"profile" => name}, call) do
    name = profile!(call, name)
    close_update(call, name, "save")
    %{profile: name}
  end

  def call("discard", %{"profile" => name}, call) do
    name = profile!(call, name)
    close_update(call, name, "discard")
    %{profile: name}
  end

  def call("save_updates", _inputs, call) do
    now = System.system_time(:millisecond)

    saved =
      for {session, %{"update" => until, "profile" => name}} <- notes(call), until && until < now do
        close_update(call, name, "expire")
        session
      end

    %{saved: saved}
  end

  # A browser on a profile is starting: the profile's last save, if it has one, goes into its container.
  @impl true
  def hook("chrome.starting", %{"session" => session, "container" => container}, call) do
    if match?(%{"state" => "starting"}, note(call, session)), do: step(call, session, "opening")

    with %{"profile" => name} <- note(call, session),
         true <- name in profiles(call),
         saved when saved != "" <- value(call, name, "saved") do
      {:ok, sealed, _tag} = Artifact.read(saved(call), value(call, name, "blob"))
      tarball = unseal(Base.decode64!(value(call, name, "key")), sealed)
      Call.tool(call, "containers", "write_file", %{"container" => container, "path" => @tarball, "data" => Base.encode64(tarball)})
      %{"user_data" => @tarball}
    else
      _ -> %{}
    end
  end

  # A browser on a profile is closing: what it closed with is saved to the profile, unless it was opened not to be.
  # A save that fails refuses the close, so the browser and what's in it stay.
  def hook("chrome.closing", %{"session" => session, "container" => container, "user_data" => tarball}, call) do
    with %{"profile" => name, "save" => true} <- note(call, session), true <- name in profiles(call) do
      {:blob, _type, data} = Call.tool(call, "containers", "read_file", %{"container" => container, "path" => tarball})
      save(call, name, data, sites(call, container))
    end

    forget(call, session)
    :ok
  rescue
    e -> {:refuse, "Couldn't save the profile: #{Exception.message(e)}"}
  end

  # Saves a profile: its tarball, encrypted under a new key, as a new blob, which the Vault item then names with its
  # key, and the last save's blob is removed. So the last to save is what's kept, and a save that fails halfway
  # leaves the last whole one.
  defp save(call, name, tarball, sites) do
    now = System.system_time(:millisecond)
    {key, blob} = {:crypto.strong_rand_bytes(32), Base.encode16(:crypto.strong_rand_bytes(16), case: :lower)}
    :ok = Artifact.write(saved(call), blob, seal(key, tarball))
    was = if value(call, name, "saved") != "", do: value(call, name, "blob")
    fields = %{"key" => Base.encode64(key), "blob" => blob, "sites" => JSON.encode!(sites), "saved" => Integer.to_string(now)}
    vault(call).keep(call, @item <> name, fields)
    if was, do: Artifact.remove(saved(call), was)
  end

  # The sites a profile was used on, from its History, newest first, at most 30: each one's registrable domain
  # (github.com, example.com.au) with when it was last visited, in ms since 1970. Chrome counts in microseconds
  # since 1601. A short list of second-level labels stands in for the public suffix list.
  @second ~w(com co org net gov edu ac id asn)
  defp sites(call, container) do
    query = "select url, last_visit_time / 1000 - 11644473600000 from urls where url like 'http%'"
    # Chrome leaves what it wrote last in History's write-ahead log, which a copy of both opens with.
    copy = "cp /root/.chromium/Default/History /tmp/history && cp /root/.chromium/Default/History-wal /tmp/history-wal"
    command = ~s{(#{copy}; sqlite3 -separator ' ' /tmp/history "#{query}"; rm -f /tmp/history*) 2>/dev/null}
    %{"stdout" => out} = Call.tool(call, "containers", "exec", %{"container" => container, "command" => command})

    for line <- String.split(out, "\n", trim: true), [url, at] <- [String.split(line, " ")], site = site(url), reduce: %{} do
      sites -> Map.update(sites, site, String.to_integer(at), &max(&1, String.to_integer(at)))
    end
    |> Enum.sort_by(fn {_site, at} -> -at end)
    |> Enum.take(30)
    |> Map.new()
  end

  defp site(url) do
    host = (URI.parse(url).host || "") |> String.downcase()
    labels = String.split(host, ".", trim: true)

    cond do
      host =~ ~r/^[\d.]+$/ or length(labels) < 2 -> if(host == "", do: nil, else: host)
      length(labels) > 2 and String.length(List.last(labels)) == 2 and Enum.at(labels, -2) in @second -> Enum.join(Enum.take(labels, -3), ".")
      true -> Enum.join(Enum.take(labels, -2), ".")
    end
  end

  @aad "chrome-profiles/1"
  defp seal(key, plain) do
    iv = :crypto.strong_rand_bytes(12)
    {sealed, tag} = :crypto.crypto_one_time_aead(:aes_256_gcm, key, iv, plain, @aad, true)
    iv <> tag <> sealed
  end

  defp unseal(key, <<iv::binary-12, tag::binary-16, sealed::binary>>) do
    case :crypto.crypto_one_time_aead(:aes_256_gcm, key, iv, sealed, @aad, tag, false) do
      :error -> raise "The profile's last save doesn't open with its key"
      plain -> plain
    end
  end

  # Where each of whoever this call is for's profiles is saved: an artifact made for them.
  defp saved(call), do: Call.make(call, "profiles")

  # The browser a person updates a profile in.
  defp update_session(name), do: "update-" <> (name |> String.downcase() |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-"))

  # Saves or discards an update (`event`), as a step to `saving` or `discarding`, then closes its browser.
  defp close_update(call, name, event) do
    session = update_session(name)
    if not match?(%{"update" => until} when until != nil, note(call, session)), do: raise("#{name} isn't being updated")
    %{"state" => state} = step(call, session, event)
    if state == "discarding", do: step(call, session, nil, %{"save" => false})

    # A browser that had stopped closes without `chrome.closing`, so its note goes here. One that didn't close is
    # ready again, saying why, and the next `save_updates` past its end closes it.
    try do
      Call.tool(call, "chrome", "close", %{"session" => session})
    rescue
      e ->
        if note(call, session), do: step(call, session, "fail", %{"why" => Exception.message(e)})
        reraise e, __STACKTRACE__
    end

    forget(call, session)
  end

  # Takes an update one step (`event`, by `@steps`) from the state its note is in, with `changes` to the note, and
  # gives the note. A `nil` event only changes the note.
  defp step(call, session, event, changes \\ %{}) do
    note = note(call, session)
    was = note && note["state"]

    state =
      cond do
        event == nil -> was
        to = @steps[{was, event}] -> to
        true -> raise "#{(note || changes)["profile"]} can't #{event} while it's #{was || "not being updated"}"
      end

    note = Map.merge(note || %{}, changes) |> Map.put("state", state)

    note =
      if state in ~w(starting ready saving discarding) and event != "fail", do: Map.put(note, "why", nil), else: note

    note(call, session, note)
    Call.changed(call)
    note
  end

  # A browser that's open already is only carried on with on the same profile: Chrome hands it a profile as it starts.
  # Whether it's running.
  defp open!(call, session, name) do
    running = Enum.find(Call.tool(call, "chrome", "list", %{})["browsers"], &(&1["session"] == session and &1["state"] == "running"))

    case {running, note(call, session)} do
      {nil, _} -> false
      {_, %{"profile" => ^name}} -> true
      _ -> raise "#{session} is open already, not on #{name}: Chrome's `close` closes it, or open another session"
    end
  end

  # The profiles' names, by their items in whoever this call is for's Vault, which this plugin keeps.
  defp profiles(call), do: for(%{name: @item <> name} <- vault(call).items(call), do: name)

  defp profile!(call, name) do
    Enum.find(profiles(call), &(String.downcase(&1) == String.downcase(String.trim(name)))) ||
      raise "You have no profile called #{name}: `list` shows yours, and `create` makes one"
  end

  defp value(call, name, field), do: vault(call).value(call, @item <> name, field)
  defp vault(call), do: Call.plugin(call, "vault")

  # Which profile each of whoever this call is for's browsers is on, by its session.
  defp notes(call) do
    prefix = "browsers/#{call.subject}/"
    names = Artifact.list(call.own, prefix)
    for {name, {:ok, data, _tag}} <- Enum.zip(names, Artifact.read_all(call.own, names)), do: {String.replace_prefix(name, prefix, ""), JSON.decode!(data)}
  end

  defp note(call, session) do
    case Artifact.read(call.own, at(call, session)) do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> nil
    end
  end

  defp note(call, session, note) do
    forget(call, session)
    Artifact.write(call.own, at(call, session), JSON.encode!(note))
  end

  defp forget(call, session), do: Artifact.remove(call.own, at(call, session))
  defp at(call, session), do: "browsers/#{call.subject}/#{session}"
end
