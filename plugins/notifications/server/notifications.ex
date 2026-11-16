defmodule Notifications do
  @moduledoc """
  Notifications: what a plugin or an agent tells a person, kept in their inbox and shown on their devices. It's
  installed for the network, so one database keeps everyone's, a row for each person a notification went to.

  A running app on a computer hears that something changed and asks `inbox` for what's new, which it shows as the
  system's own notification: one that isn't running shows nothing, and its person sees the inbox when they open
  it. A phone or a browser registers where its push service reaches it (`register`), with its notification key,
  and this plugin's edge function (`edge/`) sends there, since only it holds the push services'
  keys. What it's handed for each device is the notification encrypted to that key (`Cipher.seal/3`), so
  neither it nor the push service reads it: the device opens it before it shows.
  """
  @behaviour OneCore.Plugin

  # Every client registers with it, and it's how a person's devices are told anything.
  @impl true
  def essential, do: true

  require Logger
  alias OneCore.{Call, Edge}

  @schema [
    """
    create table notifications (
      id integer primary key, user text not null, app text, title text not null, text text, at text not null,
      read integer not null default 0
    )
    """,
    "create index notifications_by_user on notifications (user, id)",
    "create table quiet (user text not null, app text not null, primary key (user, app)) without rowid",
    """
    create table devices (
      token text primary key, user text not null, via text not null, name text not null, at text not null
    )
    """,
    "create index devices_by_user on devices (user)",
    # A device registers its notification key with its token, which what it's sent is encrypted to. One that
    # registered before that registers again as its app starts.
    "drop table devices",
    """
    create table devices (
      token text primary key, user text not null, via text not null, name text not null, at text not null,
      key text not null
    )
    """,
    "create index devices_by_user on devices (user)"
  ]

  # The most of a person's notifications an inbox shows, newest first.
  @shown 100

  @impl true
  def uses, do: ["database", "encryption"]

  @impl true
  def apps do
    [
      %{name: "Notifications", page: "client/index.html", icon: "client/icon.svg", settings: true}
    ]
  end

  @doc """
  Notifies whoever `call` runs for, as the plugin making the call, so a tap opens its app and its person can
  silence it. `notification` is what `send` takes: `"title"`, and `"text"` and `"to"` if it has them. Gives what
  `send` answers.

      notifications = OneCore.Call.plugin(call, "notifications")
      notifications.send(call, %{"title" => "Claim approved", "text" => "Lunch with Sam, $42"})
  """
  def send(%Call{plugin: plugin} = call, notification),
    do: Call.tool(call, "notifications", "send", Map.put(notification, "app", plugin))

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}
    app = %{type: "string", description: "The plugin whose app it is, by the name it's added as"}
    id = %{type: "integer", description: "The notification's id, as `inbox` lists it. All of them, if left out"}

    [
      %{
        name: "send",
        description:
          "Notifies a person, everyone in a group or everyone in the network, on their devices and in their inbox",
        inputs: %{
          type: "object",
          properties: %{
            title: %{type: "string", minLength: 1, maxLength: 120, description: "One short line"},
            text: %{type: "string", maxLength: 1000, description: "A sentence or two under it"},
            to: %{
              type: "string",
              description: "users/<name>, groups/<name> or network. Whoever you're asking for, if left out"
            },
            app: Map.put(app, :description, "The plugin whose app a tap opens, and that its person can silence")
          },
          required: ["title"]
        }
      },
      %{
        name: "inbox",
        read_only: true,
        description: "Lists your notifications, newest first, with how many are unread",
        inputs: %{
          type: "object",
          properties: %{after: %{type: "integer", description: "Only those after this id, to hear what's new"}}
        }
      },
      %{name: "read", description: "Marks a notification read", inputs: %{type: "object", properties: %{id: id}}},
      %{
        name: "clear",
        description: "Clears a notification from your inbox",
        inputs: %{type: "object", properties: %{id: id}}
      },
      %{
        name: "apps",
        read_only: true,
        description: "Lists the apps that have notified you, and whether each is allowed",
        inputs: none
      },
      %{
        name: "allow",
        description: "Allows or mutes an app's notifications. A muted app sends you nothing",
        inputs: %{
          type: "object",
          properties: %{app: app, allow: %{type: "boolean", description: "true to allow it, false to mute it"}},
          required: ["app", "allow"]
        }
      },
      %{
        name: "register",
        description: "Has a phone or a browser of yours woken by its push service when you're notified",
        inputs: %{
          type: "object",
          properties: %{
            via: %{type: "string", enum: ~w(apns fcm webpush), description: "Its push service"},
            token: %{
              type: "string",
              minLength: 1,
              maxLength: 2000,
              description: "Where it reaches the device: an APNs or FCM token, or a browser's subscription as JSON"
            },
            name: %{type: "string", maxLength: 80, description: "What the device is called"},
            key: %{
              type: "string",
              pattern: "^[0-9a-f]{64}$",
              description:
                "The device's notification key, which what it's sent is encrypted to, so nothing on the way reads it"
            }
          },
          required: ["via", "token", "name", "key"]
        }
      },
      %{
        name: "unregister",
        description: "Stops waking a device of yours",
        inputs: %{
          type: "object",
          properties: %{token: %{type: "string", description: "Its token, as `register` was given it"}},
          required: ["token"]
        }
      },
      %{name: "devices", read_only: true, description: "Lists your devices that are woken", inputs: none}
    ]
  end

  @impl true
  def call("send", %{"title" => title} = inputs, call) do
    to = Map.get(inputs, "to", call.subject)
    app = inputs["app"]
    users = users(call, to)
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    {sent, quiet, devices} =
      transaction(call, fn db ->
        quiet = if app, do: query(db, "select user from quiet where app = ?", [app]), else: []
        quiet = Enum.filter(users, &(%{"user" => &1} in quiet))
        sent = users -- quiet

        for user <- sent do
          query(db, "insert into notifications (user, app, title, text, at) values (?, ?, ?, ?, ?)", [
            user,
            app,
            title,
            inputs["text"],
            at
          ])
        end

        {sent, quiet,
         query(db, "select token, via, name, key from devices where user in (select value from json_each(?))", [
           JSON.encode!(sent)
         ])}
      end)

    pushed = push(call, devices, %{title: title, text: inputs["text"], app: app})

    Enum.join(
      [if(sent == [], do: "Nobody was notified.", else: "Notified #{listed(sent)}.")] ++
        if(quiet == [], do: [], else: ["#{listed(quiet)} muted #{app}."]) ++ List.wrap(pushed),
      " "
    )
  end

  def call("inbox", inputs, call) do
    user = person(call)

    transaction(call, fn db ->
      [%{"unread" => unread}] =
        query(db, "select count(*) as unread from notifications where user = ? and read = 0", [user])

      notifications =
        query(
          db,
          """
          select id, app, title, text, at, read from notifications where user = ? and id > ?
          order by id desc limit ?
          """,
          [user, Map.get(inputs, "after", 0), @shown]
        )

      names = names(user)
      shown = Enum.map(notifications, &Map.merge(&1, %{"read" => &1["read"] == 1, "from" => names[&1["app"]]}))
      %{unread: unread, notifications: shown}
    end)
  end

  def call("read", inputs, call) do
    changed(call, inputs, "update notifications set read = 1", "No notification is read")
    "Read."
  end

  def call("clear", inputs, call) do
    changed(call, inputs, "delete from notifications", "No notification is cleared")
    "Cleared."
  end

  def call("apps", _inputs, call) do
    user = person(call)
    names = names(user)

    transaction(call, fn db ->
      query(
        db,
        """
        select app, not exists (select 1 from quiet where quiet.user = ? and quiet.app = apps.app) as allowed from (
          select app from notifications where user = ? and app is not null union select app from quiet where user = ?
        ) as apps order by app
        """,
        [user, user, user]
      )
      |> Enum.map(&Map.merge(&1, %{"allowed" => &1["allowed"] == 1, "name" => names[&1["app"]] || &1["app"]}))
    end)
  end

  def call("allow", %{"app" => app, "allow" => allow}, call) do
    user = person(call)

    transaction(call, fn db ->
      if allow,
        do: query(db, "delete from quiet where user = ? and app = ?", [user, app]),
        else: query(db, "insert or ignore into quiet (user, app) values (?, ?)", [user, app])
    end)

    if allow, do: "#{app} is allowed.", else: "#{app} is muted."
  end

  def call("register", %{"via" => via, "token" => token, "name" => name, "key" => key}, call) do
    user = person(call)
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    transaction(call, fn db ->
      query(db, "insert or replace into devices (token, user, via, name, at, key) values (?, ?, ?, ?, ?, ?)", [
        token,
        user,
        via,
        name,
        at,
        key
      ])
    end)

    "#{name} is woken when you're notified."
  end

  def call("unregister", %{"token" => token}, call) do
    user = person(call)

    removed = "delete from devices where token = ? and user = ? returning name"

    case transaction(call, &query(&1, removed, [token, user])) do
      [%{"name" => name}] -> "#{name} isn't woken any more."
      [] -> raise "No device of yours has that token. `devices` lists them."
    end
  end

  def call("devices", _inputs, call) do
    user = person(call)
    transaction(call, &query(&1, "select name, via, at from devices where user = ? order by at", [user]))
  end

  # Marks one of a person's notifications, or all of them, with `sql`, which names the table and what it does.
  defp changed(call, inputs, sql, none) do
    user = person(call)

    transaction(call, fn db ->
      case inputs do
        %{"id" => id} ->
          if query(db, sql <> " where user = ? and id = ? returning id", [user, id]) == [],
            do: raise("#{none}: you have none with id #{id}. `inbox` lists them.")

        _ ->
          query(db, sql <> " where user = ?", [user])
      end
    end)
  end

  # What a person sees each plugin's app called, by the plugin's name, of the ones installed for them. One with no app
  # is called by its name.
  defp names(user) do
    Map.new(OneCore.Plugins.running(user), fn {plugin, version} ->
      {plugin, Enum.find_value(version.apps || [], plugin, & &1[:name])}
    end)
  end

  # Who a notification goes to: a user, or the users in a group or the network, whom relations says.
  defp users(_call, "users/" <> _ = user), do: [user]

  defp users(call, "groups/" <> _ = to), do: Call.tool(call, "relations", "members", %{"of" => to})
  defp users(call, "network"), do: Call.tool(call, "relations", "members", %{"of" => "network"})

  defp users(_call, to), do: raise("#{to} isn't a user, a group or the network.")

  # A person's inbox is theirs whoever the call runs for: a group or the network they asked for.
  defp person(%Call{actor: "users/" <> _ = by}), do: by
  defp person(_call), do: raise("An inbox is a person's, and nobody is asking.")

  defp listed([one]), do: one
  defp listed(many), do: Enum.join(Enum.drop(many, -1), ", ") <> " and " <> List.last(many)

  # Has this plugin's edge function wake the phones and browsers it went to, each sent the
  # notification encrypted to it, and forgets the ones whose push service says they're gone. With no address there's nothing to send through, and that's said only if
  # some device was to be woken.
  defp push(_call, [], _notification), do: nil

  defp push(call, devices, notification) do
    with address when is_binary(address) <- Application.get_env(:one_core, :address),
         said = JSON.encode!(notification),
         sent = Enum.map(devices, &%{token: &1["token"], via: &1["via"], sealed: sealed(&1["key"], said)}),
         {:ok, %{"gone" => gone, "failed" => failed}} <- ask(address, %{devices: sent}) do
      if gone != [] do
        forget = "delete from devices where token in (select value from json_each(?))"
        transaction(call, &query(&1, forget, [JSON.encode!(gone)]))
      end

      names = Map.new(devices, &{&1["token"], &1["name"]})

      if failed != [],
        do:
          Enum.map_join(failed, " ", fn %{"token" => token, "why" => why} ->
            "#{names[token]} wasn't woken: #{why}."
          end)
    else
      nil -> "Phones and browsers weren't woken: this network has no address."
      {:error, why} -> "Phones and browsers weren't woken: #{why}"
    end
  end

  defp sealed(key, said) do
    {:ok, sealed} = Cipher.seal(key, "one/notifications/1/notification\n" <> key, said)
    Base.encode64(sealed)
  end

  # Asks this plugin's edge function, signed as this server, which it answers only for a server
  # acting for the network.
  defp ask(address, what) do
    body = JSON.encode!(what)
    path = "/notifications/send"

    case Req.post(address <> path, body: body, headers: Edge.signed(path, body), retry: false) do
      {:ok, %{status: 200, body: answer}} -> {:ok, answer}
      {:ok, %{status: status, body: %{"error" => why}}} -> {:error, "the network's edge answered #{status}: #{why}"}
      {:ok, %{status: status}} -> {:error, "the network's edge answered #{status}"}
      {:error, error} -> {:error, Exception.message(error)}
    end
  end

  defp transaction(call, fun) do
    database = Call.plugin(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values), do: database.query(db, sql, values)
end
