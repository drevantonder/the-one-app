defmodule ChromeProfiles do
  @moduledoc """
  Chrome Profiles: signed-in browser profiles, like Personal and Work, that a Chrome browser starts with.

  A profile is an item in its person's own Vault, `Chrome profile: <name>`, which this plugin keeps (`keep/4`) and
  only it is switched on for: the browser's storage state (Playwright's: cookies and local storage), the sites
  it was used on with when, and when it was last saved, each a secret field. Nothing answers with the storage state:
  it goes only to the person's own browser, as `chrome.starting` is answered.

  Which browser is on which profile is a note in the plugin's own artifact, `browsers/<whose>/<session>`, written
  as `open` or `sign_in` opens it: Chrome hands a browser that's starting the note's profile, and as it closes
  (`chrome.closing`) its storage state is saved to it, unless the note says not to. A sign-in's note says when it
  ends: the job `close_sign_ins` closes, and so saves, any past it.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Artifact, Call}

  @item "Chrome profile: "
  @sign_in_minutes 15
  @empty %{"cookies" => [], "origins" => []}
  @profile %{type: "string", description: "the profile, by its name, like Personal or Work"}

  @impl true
  def uses, do: ["vault", "database", "chrome"]

  @impl true
  def hooks, do: ["chrome.starting", "chrome.closing"]

  @impl true
  def jobs, do: [%{tool: "close_sign_ins", every: 300}]

  @impl true
  def apps, do: [%{name: "Chrome Profiles", page: "client/index.html", icon: "client/icon.svg"}]

  @impl true
  def tools do
    [
      %{
        name: "list",
        read_only: true,
        description:
          "Your profiles: each one's name, the sites it was used on with when (ms since 1970), newest first, when it was last saved, and the sign-in waiting on it, if one is. Never what's signed in itself.",
        inputs: %{type: "object", properties: %{}}
      },
      %{
        name: "create",
        description: "Create a profile, signed in to nothing yet.",
        inputs: %{
          type: "object",
          required: ["name"],
          properties: %{name: %{type: "string", minLength: 1, maxLength: 40, description: "like Personal or Work"}}
        }
      },
      %{
        name: "rename",
        description: "Rename a profile.",
        inputs: %{type: "object", required: ["profile", "name"], properties: %{profile: @profile, name: %{type: "string", minLength: 1, maxLength: 40}}}
      },
      %{
        name: "remove",
        description: "Remove a profile and everything signed in on it, for good.",
        inputs: %{type: "object", required: ["profile"], properties: %{profile: @profile}}
      },
      %{
        name: "open",
        description:
          "Open a Chrome browser on a profile: it starts signed in to what the profile is. Then use Chrome's tools with its session. Chrome's `close` saves what changed back to the profile, unless `save` is false. Two browsers on one profile each start from its last save, and the last to close is what's kept.",
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
        }
      },
      %{
        name: "sign_in",
        description:
          "Open a site on a profile for your person to sign in to themselves, in the Chrome Profiles app. Never sign in for them. It's saved to the profile when they tap Done there, or after #{@sign_in_minutes} minutes.",
        inputs: %{
          type: "object",
          required: ["profile", "url"],
          properties: %{profile: @profile, url: %{type: "string", description: "the site's sign-in page, or its home page"}}
        }
      },
      %{
        name: "close_sign_ins",
        description: "Close, and so save, the sign-ins past their #{@sign_in_minutes} minutes. It's run every 5 minutes.",
        inputs: %{type: "object", properties: %{}}
      }
    ]
  end

  @impl true
  def call("list", _inputs, call) do
    signing_in = for {session, %{"sign_in" => until} = note} <- notes(call), until, into: %{}, do: {note["profile"], %{session: session, site: note["site"], until: until}}

    for name <- profiles(call) do
      sites = JSON.decode!(value(call, name, "sites"))
      saved = value(call, name, "saved")

      %{
        profile: name,
        sites: sites |> Enum.sort_by(fn {_site, used} -> -used end) |> Enum.map(fn {site, used} -> %{site: site, used: used} end),
        saved: if(saved == "", do: nil, else: String.to_integer(saved)),
        signing_in: signing_in[name]
      }
    end
  end

  def call("create", %{"name" => name}, call) do
    name = String.trim(name)
    if name == "", do: raise("A profile has a name")
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(name))), do: raise("You have a profile called #{name} already")
    vault(call).keep(call, @item <> name, %{"storage_state" => JSON.encode!(@empty), "sites" => "{}", "saved" => ""})
    "Created #{name}. It's signed in to nothing yet: `sign_in` has your person sign in to a site on it."
  end

  def call("rename", %{"profile" => from, "name" => to}, call) do
    from = profile!(call, from)
    to = String.trim(to)
    if Enum.any?(profiles(call), &(String.downcase(&1) == String.downcase(to) and &1 != from)), do: raise("You have a profile called #{to} already")
    fields = Map.new(~w(storage_state sites saved), &{&1, value(call, from, &1)})
    vault(call).keep(call, @item <> to, fields)
    vault(call).remove(call, @item <> from)
    for {session, %{"profile" => ^from} = note} <- notes(call), do: note(call, session, %{note | "profile" => to})
    "Renamed #{from} to #{to}"
  end

  def call("remove", %{"profile" => name}, call) do
    name = profile!(call, name)
    vault(call).remove(call, @item <> name)
    "Removed #{name}"
  end

  def call("open", %{"profile" => name, "session" => session} = inputs, call) do
    name = profile!(call, name)
    open!(call, session, name)
    note(call, session, %{"profile" => name, "save" => Map.get(inputs, "save", true), "site" => site(inputs["url"]), "sign_in" => nil})
    opened = Call.tool(call, "chrome", "open", Map.take(inputs, ~w(session description url)))
    "On #{name}. #{opened}"
  end

  def call("sign_in", %{"profile" => name, "url" => url}, call) do
    name = profile!(call, name)
    site = site(url) || raise("Give the site's address, like https://github.com/login")
    session = "sign-in-" <> (name |> String.downcase() |> String.replace(~r/[^a-z0-9]+/, "-") |> String.trim("-"))
    open!(call, session, name)
    until = System.system_time(:millisecond) + @sign_in_minutes * 60_000
    note(call, session, %{"profile" => name, "save" => true, "site" => site, "sign_in" => until})
    Call.tool(call, "chrome", "open", %{"session" => session, "description" => "Signing in to #{site} on #{name}", "url" => url})

    "#{site} is open on #{name} for your person to sign in to, in the Chrome Profiles app on any device of theirs, where the profile says Signing in. " <>
      "Ask them to open it (notifications_send with app chrome-profiles reaches their devices). It's saved when they tap Done, or in #{@sign_in_minutes} minutes. " <>
      "Don't sign in for them, and don't use the browser #{session} while they do."
  end

  def call("close_sign_ins", _inputs, call) do
    now = System.system_time(:millisecond)

    for {session, %{"sign_in" => until}} <- notes(call), until && until < now do
      try do
        Call.tool(call, "chrome", "close", %{"session" => session})
      rescue
        # Its browser is gone already, so it closed without saying so: there's nothing to save.
        _ -> forget(call, session)
      end

      session
    end
  end

  # A browser on a profile is starting: it's handed the profile's storage state.
  @impl true
  def hook("chrome.starting", %{"session" => session}, call) do
    with %{"profile" => name} <- note(call, session), true <- name in profiles(call) do
      %{"storage_state" => JSON.decode!(value(call, name, "storage_state"))}
    else
      _ -> %{}
    end
  end

  # A browser on a profile closed: what it closed with is saved to the profile, unless it was opened not to be.
  def hook("chrome.closing", %{"session" => session, "storage_state" => state}, call) do
    with %{"profile" => name, "save" => true} = note <- note(call, session), true <- name in profiles(call) do
      save(call, name, state, note["site"])
    end

    forget(call, session)
    :ok
  end

  # Saves a profile's storage state, and which sites it was used on: those whose cookies or local storage changed,
  # and the one it was opened at, are used now, and a site it holds nothing for any more is dropped.
  defp save(call, name, state, opened) do
    now = System.system_time(:millisecond)
    was = JSON.decode!(value(call, name, "storage_state"))
    sites = JSON.decode!(value(call, name, "sites"))
    held = held(state)
    changed = for {site, kept} <- held, kept != held(was)[site], do: site
    used = Enum.reject([opened | changed], &is_nil/1)
    sites = sites |> Map.take(Map.keys(held)) |> Map.merge(Map.new(used, &{&1, now}))
    fields = %{"storage_state" => JSON.encode!(state), "sites" => JSON.encode!(sites), "saved" => Integer.to_string(now)}
    vault(call).keep(call, @item <> name, fields)
  end

  # What a storage state holds for each site: its cookies and its origins' local storage, sorted to compare.
  defp held(state) do
    cookies = for c <- state["cookies"] || [], do: {site(c["domain"]), {c["name"], c["value"], c["domain"], c["path"]}}
    storage = for o <- state["origins"] || [], s <- o["localStorage"] || [], do: {site(o["origin"]), {o["origin"], s["name"], s["value"]}}
    Enum.group_by(cookies ++ storage, &elem(&1, 0), &elem(&1, 1)) |> Map.new(fn {site, kept} -> {site, Enum.sort(kept)} end)
  end

  # A host's site, as a person names it: its registrable domain (github.com, example.com.au), from a URL, an origin
  # or a cookie's domain. A short list of second-level labels stands in for the public suffix list.
  @second ~w(com co org net gov edu ac id asn)
  defp site(nil), do: nil

  defp site(address) do
    host = (URI.parse(address).host || address) |> String.trim_leading(".") |> String.downcase()
    labels = String.split(host, ".", trim: true)

    cond do
      host =~ ~r/^[\d.]+$/ or length(labels) < 2 -> if(host == "", do: nil, else: host)
      length(labels) > 2 and String.length(List.last(labels)) == 2 and Enum.at(labels, -2) in @second -> Enum.join(Enum.take(labels, -3), ".")
      true -> Enum.join(Enum.take(labels, -2), ".")
    end
  end

  # A browser that's open already is only carried on with on the same profile: Chrome hands it a profile as it starts.
  defp open!(call, session, name) do
    running = Enum.find(Call.tool(call, "chrome", "list", %{}), &(&1["session"] == session and &1["state"] == "running"))

    case {running, note(call, session)} do
      {nil, _} -> :ok
      {_, %{"profile" => ^name}} -> :ok
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
    case Artifact.read(call.own, key(call, session)) do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> nil
    end
  end

  defp note(call, session, note) do
    forget(call, session)
    Artifact.write(call.own, key(call, session), JSON.encode!(note))
  end

  defp forget(call, session), do: Artifact.remove(call.own, key(call, session))
  defp key(call, session), do: "browsers/#{call.subject}/#{session}"
end
