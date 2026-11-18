defmodule Vault.Items do
  @moduledoc """
  An item: its fields (a secret one never read back), its websites, who has access (`Vault.Access`), what kind it is,
  and its status, as the app that keeps it says. Every write of an item that may make it goes through `write!/5`.

  Its record:

      %{"id", "name", "kind", "type", "app", "hosts", "access", "oauth", "fields", "contents", "status", "added", "changed"}

  - `kind` is stored once, as it's made, never worked out again: `password`, which a person or an agent keeps; `account`,
    a sign-in or a secret an app keeps for itself; or `app_item`, an item of a type an app registered (`vault.types`),
    `type` naming it by its app's artifact and its name: `%{"app" => "network/apps/chrome-profiles", "name" => "Chrome
    profile"}`.
  - `app` is the app that keeps an account or an app item, which alone says its status; nil for a password.
  - `status` is nil when it's ready, else `%{"state" => "busy" | "needs_action", "message", "action", "by"}`.
  """

  alias OneCore.Apps
  alias Vault.Store

  @kinds ~w(password account app_item)

  # An item's status, which only the app that keeps it moves (`status!/5`): ready, which shows nothing; busy, something's
  # under way; or needs_action, its person has to act. Any moves to any other, and to itself with new words:
  #
  #   ready <-> busy <-> needs_action <-> ready
  @status_moves %{"ready" => ~w(ready busy needs_action), "busy" => ~w(ready busy needs_action), "needs_action" => ~w(ready busy needs_action)}

  @doc "The kinds an item may be."
  def kinds, do: @kinds

  @doc """
  The one write of an item that may make it: finds `name` and gives its record to `change`, which gives what it's to
  be, or makes it first from `made` where there's none and `made` says how (`password/1`, `kept_by/3`). `by` is
  whoever writes, which must have access to an item that's there: `:person`, or `{:app, artifact}`.
  """
  def write!(call, db, name, made, by, change) do
    found = Store.find(call, db, name)

    item =
      cond do
        found -> Vault.Access.may!(current(call, db, found, names(call)), by)
        made -> Store.create(call, db, name, made)
        true -> raise "There's no item named #{name} in the Vault."
      end

    Store.change!(db, item, change)
  end

  @doc "A new password's record: what a person or an agent keeps, with access for nobody until they say."
  def password(hosts), do: blank("password", nil, nil, hosts, [])

  @doc """
  A new record for what `app` keeps: an item of its type where the name is of one it registered, or else an account,
  with access for it alone.
  """
  def kept_by(call, app, name, hosts \\ []) do
    case Enum.find(types(call), &(&1.artifact == app and String.starts_with?(name, &1.prefix))) do
      nil -> blank("account", nil, app, hosts, [app])
      type -> blank("app_item", %{"app" => app, "name" => type.name}, app, hosts, [app])
    end
  end

  defp blank(kind, type, app, hosts, access) do
    %{
      "kind" => kind,
      "type" => type,
      "app" => app,
      "hosts" => hosts!(hosts),
      "access" => access,
      "oauth" => nil,
      "fields" => [],
      "contents" => %{},
      "status" => nil,
      "added" => nil
    }
  end

  @doc "The item named `name`, as it's kept now, or nil."
  def find(call, db, name) do
    if found = Store.find(call, db, name), do: current(call, db, found, names(call))
  end

  def find!(call, db, name), do: find(call, db, name) || raise("There's no item named #{name} in the Vault.")

  @doc "Every item, as each is kept now."
  def all(call, db) do
    names = names(call)
    for item <- Store.everything(call, db), do: current(call, db, item, names)
  end

  # An item as it's kept now: upgraded, and with no access for an app that's no longer installed (`Vault.Access.prune/4`).
  defp current(call, db, item, names), do: Vault.Access.prune(call, db, upgraded(call, db, item), names)

  # A record kept before items had a kind, access and a status is kept as they are now, once: its kind is what its
  # type and its sign-in made it, and its app the one of its type, or the first that used it. Where an app couldn't
  # say its types just now, it's read so and kept as it was, for a call when every app can.
  defp upgraded(_call, _db, %{"kind" => _} = item), do: item

  defp upgraded(call, db, item) do
    uses = item["uses"] || []
    {types, whole?} = collected(call)
    type = Enum.find(types, &(&1.artifact in uses and String.starts_with?(item["name"], &1.prefix)))

    {kind, type, app} =
      cond do
        type -> {"app_item", %{"app" => type.artifact, "name" => type.name}, type.artifact}
        item["oauth"] -> {"account", nil, List.first(uses)}
        true -> {"password", nil, nil}
      end

    status =
      case item["state"] do
        %{"state" => "action_required"} = state -> %{state | "state" => "needs_action"}
        state -> state
      end

    upgrade = fn record ->
      record
      |> Map.drop(["uses", "agents", "state"])
      |> Map.merge(%{
        "kind" => kind,
        "type" => type,
        "app" => app,
        "access" => if(record["agents"], do: ["agents"], else: []) ++ (record["uses"] || []),
        "status" => status
      })
    end

    if whole?, do: Store.change!(db, item, upgrade), else: upgrade.(item)
  end

  @doc "An item's fields' values, by name."
  def fields(item), do: Map.new(item["fields"], &{&1["name"], &1["value"]})

  @doc "Puts a field in a record, in place of one by its name or after the rest."
  def put_field(item, name, value, secret) do
    field = %{"name" => field!(name), "value" => value, "secret" => secret == true}

    Map.update!(item, "fields", fn fields ->
      if Enum.any?(fields, &(&1["name"] == name)),
        do: Enum.map(fields, &if(&1["name"] == name, do: field, else: &1)),
        else: fields ++ [field]
    end)
  end

  @doc "Adds websites to a record's: a write never takes one away, which only its person does."
  def add_hosts(item, hosts), do: Map.update!(item, "hosts", &Enum.uniq(&1 ++ hosts!(hosts)))

  @doc """
  Moves an item's status, as the app that keeps it says (`@status_moves`): `:ready`, which keeps nothing; `:busy`,
  with a `message`; or `:needs_action`, with a `message` and the one `action` that fixes it.
  """
  def status!(call, db, item, app, state, message \\ nil, action \\ nil) do
    if item["app"] != app,
      do: raise("Only the app that keeps #{item["name"]} says how it is, and #{Path.basename(app)} doesn't.")

    said = said!(state, message, action, app_name(call, app))
    from = if item["status"], do: item["status"]["state"], else: "ready"
    to = if said, do: said["state"], else: "ready"
    if to not in @status_moves[from], do: raise("An item doesn't go from #{from} to #{to}.")
    if item["status"] != said, do: Store.change!(db, item, &Map.put(&1, "status", said)), else: item
  end

  # What an app said of its item's status, checked, as an item keeps it: nothing for `ready`, since fine is silent.
  defp said!(state, message, action, app) do
    case to_string(state) do
      "ready" ->
        nil

      "busy" when is_binary(message) ->
        %{"state" => "busy", "message" => message, "by" => app}

      "needs_action" when is_binary(message) and is_map(action) ->
        action = JSON.decode!(JSON.encode!(action))

        if not is_binary(action["label"]) or not (is_binary(action["tool"]) or is_binary(action["page"])),
          do: raise("An action has a label, and either a tool of the app's or a page of its client's.")

        %{"state" => "needs_action", "message" => message, "action" => action, "by" => app}

      _ ->
        raise "An item is ready, busy with a message, or needs_action with why and an action."
    end
  end

  def field!(name) do
    if is_binary(name) and name =~ ~r/^[A-Za-z0-9_]+$/,
      do: name,
      else: raise("A field's name is letters, digits and _.")
  end

  @doc "A site is a host's name, with its port if it has one: never a path, so a secret goes to all of it or none."
  def hosts!(hosts) do
    for host <- hosts do
      if is_binary(host) and host =~ ~r/^[a-z0-9.-]+(:\d+)?$/,
        do: host,
        else: raise("A site is a host's name, like api.example.com.")
    end
  end

  @doc """
  The item types of the apps whoever this runs for has (`vault.types`), with each app's name, artifact, title and
  icon, and whether it's trusted, which decides where its component is drawn. A type that doesn't say what one must
  is left out.
  """
  def types(call), do: elem(collected(call), 0)

  # The types, and whether every app that hooks into `vault.types` and runs here said its own.
  defp collected(call) do
    # Asked by the hook's whole name, since an app keeping an item calls this in its own call, not the Vault's.
    {:ok, parts, failed} = OneCore.Hooks.collect("vault.types", %{}, {call.actor, call.subject})
    running = Apps.running(call.subject)

    types =
      for {app, part} <- Enum.sort(parts),
        version = running[app],
        version != nil,
        type <- List.wrap(JSON.decode!(JSON.encode!(part))),
        type?(type) do
      %{
        id: "#{app}/#{type["name"]}",
        app: app,
        artifact: version.artifact,
        title: version.title,
        icon: version.icon,
        trusted: Map.get(version, :trusted, true),
        name: type["name"],
        prefix: type["prefix"],
        section: type["section"],
        component: type["component"],
        create: type["create"]
      }
    end

    {types, failed == %{} and Enum.all?(Map.keys(parts), &Map.has_key?(running, &1))}
  end

  defp type?(%{"name" => name, "prefix" => prefix, "section" => section})
       when is_binary(name) and is_binary(prefix) and prefix != "" and is_binary(section),
       do: true

  defp type?(_type), do: false

  @doc "The apps' names as whoever this runs for finds them, by their artifacts."
  def names(call), do: Map.new(Apps.running(call.subject), fn {name, version} -> {version.artifact, name} end)
  def named(names, app), do: Map.get(names, app, Path.basename(app))
  def app_name(call, app), do: named(names(call), app)

  @doc "What a person calls an app, by its artifact."
  def title(call, app),
    do: Enum.find_value(Apps.running(call.subject), Path.basename(app), fn {_name, version} -> version.artifact == app && version.title end)

  @doc "An item as `list` gives it: never a secret field's value."
  def listed(item, db, names, types) do
    fields =
      for field <- item["fields"] do
        if field["secret"],
          do: %{name: field["name"], secret: true},
          else: %{name: field["name"], secret: false, value: field["value"]}
      end

    type = item["type"] && Enum.find(types, &(&1.artifact == item["type"]["app"] and &1.name == item["type"]["name"]))
    apps = Enum.uniq(for who <- item["access"], who != "agents", do: named(names, who))

    %{
      name: item["name"],
      artifact: Store.artifact(db, item["id"]),
      kind: item["kind"],
      type: type && type.id,
      status: item["status"],
      fields: fields,
      hosts: item["hosts"],
      access: if("agents" in item["access"], do: ["agents"], else: []) ++ Enum.sort(apps),
      sign_in: item["oauth"] != nil,
      added: item["added"],
      changed: item["changed"]
    }
  end
end
