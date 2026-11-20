defmodule Notifications do
  @moduledoc """
  Notifications: what an app or an agent tells a person, kept in their inbox and shown on their devices. It's
  installed for the network, so one database keeps everyone's, a row for each person a notification went to.

  A running app on a computer hears that something changed and asks `inbox` for what's new, which it shows as the
  system's own notification: one that isn't running shows nothing, and its person sees the inbox when they open
  it. A phone or a browser registers where its push service reaches it (`register`), with its notification key,
  and this app's edge function (`edge/`) sends there, since only it holds the push services'
  keys. What it's handed for each device is the notification encrypted to that key (`Cipher.seal/3`), so
  neither it nor the push service reads it: the device opens it before it shows.

  A phone is woken the same way with nothing to show when a server asks its app something while it isn't running
  (the hook `wake`), as Computer Use does of an Android phone's screen.
  """
  @behaviour OneCore.App

  # Every client registers with it, and it's how a person's devices are told anything.
  @impl true
  def icon, do: "client/icon.svg"

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
    "create index devices_by_user on devices (user)",
    # A device registers with its own key too, which is what Sign-In says when it removes the device, so that what
    # reaches it is forgotten with it. One that registered before registers again as its app starts.
    "drop table devices",
    """
    create table devices (
      token text primary key, user text not null, via text not null, name text not null, at text not null,
      key text not null, device text not null
    )
    """,
    "create index devices_by_user on devices (user)",
    "create index devices_by_device on devices (device)",
    # Where in its app a tap opens, where it isn't the app's first page.
    "alter table notifications add column page text",
    # Or its section in Settings, which is the app's own too.
    "alter table notifications add column section text"
  ]

  # The most of a person's notifications an inbox shows, newest first.
  @shown 100

  # Its data is everyone's together, each person's inbox, quiet apps and devices kept apart by user.
  @impl true
  def own, do: :installed

  @impl true
  def requires do
    %{
      "core" => ">= 0.32.167",
      "database" => "to keep your inbox",
      "encryption" => "to encrypt what's sent to your devices"
    }
  end

  @impl true
  def hooks, do: ["sign-in.device_removed", "wake", "settings.sections"]

  # Which apps may notify you, and whether this device is woken, are Settings' Notifications; the inbox stays the app.
  @impl true
  def hook("settings.sections", _about, _call),
    do: [%{in: "settings", name: "Notifications", component: "settings", icon: "client/icon.svg", group: "general", order: 1}]

  # A removed device is nobody's, so nothing is sent where its push service reaches it.
  def hook("sign-in.device_removed", %{"device" => device, "who" => who}, call) do
    transaction(call, &query(&1, "delete from devices where device = ? and user = ?", [device, who]))
    :ok
  end

  # Wakes a device whose app a server is about to ask something, through each push token it registered as its
  # person's: one registered under its key by anyone else is never woken.
  def hook("wake", %{"device" => device}, call) do
    with "users/" <> _ = user <- OneCore.Hooks.whose(device),
         [_ | _] = devices <- transaction(call, &query(&1, "select token, via from devices where device = ? and user = ?", [device, user])),
         address when is_binary(address) <- Application.get_env(:one_core, :address),
         {:ok, %{"gone" => gone, "failed" => failed}} <- ask(address, %{devices: devices}) do
      forget(call, gone)
      %{"woken" => length(failed) + length(gone) < length(devices)}
    else
      _ -> nil
    end
  end

  @impl true
  def entrypoints do
    [
      %{page: "client/index.html", subject: "user", runtime_version: 4}
    ]
  end

  @doc """
  Notifies whoever `call` runs for, as the app making the call (`send` takes that from the call, never from what's
  asked), so a tap opens its app and its person can silence it. `notification` is what `send` takes: `"title"`, and
  `"text"` and `"to"` if it has them. Gives what `send` answers.

      notifications = OneCore.Call.app(call, "notifications")
      notifications.send(call, %{"title" => "Claim approved", "text" => "Lunch with Sam, $42"})
  """
  def send(%Call{} = call, notification), do: Call.tool(call, "notifications", "send", notification)

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}
    app = %{type: "string", description: "The app whose app it is, by the name it's added as"}
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
              description:
                "users/<name>, groups/<name> or network: you, or someone you share a group with. Whoever you're asking for, if left out"
            },
            page: %{
              type: "string",
              description:
                "Where an app notifies, one of its own pages a tap opens instead of its first, and what follows # there: client/index.html#waiting"
            },
            section: %{
              type: "string",
              description:
                "Where an app notifies, or one of its own sections in Settings a tap opens, by its component (settings.sections), and what follows # there: devices#waiting=7"
            }
          },
          required: ["title"]
        },
        outputs: %{
          type: "object",
          properties: %{
            sent: %{type: "array", items: %{type: "string"}, description: "The users notified"},
            muted: %{type: "array", items: %{type: "string"}, description: "The users who muted its app, and weren't"},
            unwoken: %{
              type: "array",
              description: "The phones and browsers that weren't woken, and why",
              items: %{
                type: "object",
                properties: %{device: %{type: "string"}, why: %{type: "string"}},
                required: ["device", "why"]
              }
            }
          },
          required: ["sent", "muted", "unwoken"]
        }
      },
      %{
        name: "inbox",
        read_only: true,
        description: "Lists your notifications, newest first, with how many are unread",
        inputs: %{
          type: "object",
          properties: %{after: %{type: "integer", description: "Only those after this id, to hear what's new"}}
        },
        outputs: %{
          type: "object",
          properties: %{
            unread: %{type: "integer"},
            notifications: %{
              type: "array",
              description: "Newest first",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "integer"},
                  app: %{type: ["string", "null"], description: "The app whose app a tap opens"},
                  page: %{type: ["string", "null"], description: "Its page a tap opens, where it isn't its first"},
                  opens: %{type: ["string", "null"], description: "What a tap opens, as the UI library's `open` takes it: <app>/<page>#…, or none"},
                  from: %{type: ["string", "null"], description: "What its app is called"},
                  title: %{type: "string"},
                  text: %{type: ["string", "null"]},
                  at: %{type: "string", description: "When, ISO 8601"},
                  read: %{type: "boolean"}
                },
                required: ["id", "app", "page", "opens", "from", "title", "text", "at", "read"]
              }
            }
          },
          required: ["unread", "notifications"]
        }
      },
      %{
        name: "read",
        description: "Marks a notification read",
        inputs: %{type: "object", properties: %{id: id}},
        outputs: %{type: "object", properties: %{read: %{type: "integer", description: "How many were marked read"}}, required: ["read"]}
      },
      %{
        name: "clear",
        description: "Clears a notification from your inbox",
        inputs: %{type: "object", properties: %{id: id}},
        outputs: %{type: "object", properties: %{cleared: %{type: "integer", description: "How many were cleared"}}, required: ["cleared"]}
      },
      %{
        name: "apps",
        read_only: true,
        description: "Lists the apps that have notified you, and whether each is allowed",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            apps: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  app: app,
                  name: %{type: "string", description: "What its app is called"},
                  allowed: %{type: "boolean"}
                },
                required: ["app", "name", "allowed"]
              }
            }
          },
          required: ["apps"]
        }
      },
      %{
        name: "allow",
        description: "Allows or mutes an app's notifications. A muted app sends you nothing",
        inputs: %{
          type: "object",
          properties: %{app: app, allow: %{type: "boolean", description: "true to allow it, false to mute it"}},
          required: ["app", "allow"]
        },
        outputs: %{type: "object", properties: %{app: app, allowed: %{type: "boolean"}}, required: ["app", "allowed"]}
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
            name: %{
              type: "string",
              maxLength: 80,
              format: "device-name",
              description: "What the device is called. This device's, if left out"
            },
            device: %{type: "string", format: "device", description: "The device's key. This device's, if left out"},
            key: %{
              type: "string",
              pattern: "^[0-9a-f]{64}$",
              description:
                "The device's notification key, which what it's sent is encrypted to, so nothing on the way reads it"
            }
          },
          required: ["via", "token", "name", "key", "device"]
        },
        outputs: %{type: "object", properties: %{device: %{type: "string", description: "What it's called"}}, required: ["device"]}
      },
      %{
        name: "unregister",
        description: "Stops waking a device of yours",
        inputs: %{
          type: "object",
          properties: %{token: %{type: "string", description: "Its token, as `register` was given it"}},
          required: ["token"]
        },
        outputs: %{type: "object", properties: %{device: %{type: "string", description: "What it's called"}}, required: ["device"]}
      },
      %{
        name: "devices",
        read_only: true,
        description: "Lists your devices that are woken",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            devices: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  via: %{type: "string", enum: ~w(apns fcm webpush)},
                  at: %{type: "string", description: "When it was registered, ISO 8601"}
                },
                required: ["name", "via", "at"]
              }
            }
          },
          required: ["devices"]
        }
      }
    ]
  end

  @impl true
  def call("send", %{"title" => title} = inputs, call) do
    to = Map.get(inputs, "to", call.subject)
    app = app(call)
    users = call |> users(to) |> told!(call)
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    {sent, quiet, devices} =
      transaction(call, fn db ->
        quiet = if app, do: query(db, "select user from quiet where app = ?", [app]), else: []
        quiet = Enum.filter(users, &(%{"user" => &1} in quiet))
        sent = users -- quiet

        for user <- sent do
          query(db, "insert into notifications (user, app, page, section, title, text, at) values (?, ?, ?, ?, ?, ?, ?)", [
            user,
            app,
            app && inputs["page"],
            app && inputs["section"],
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

    # Said as Notifications' change: one sent from a job, a service or another app's call is otherwise told as
    # nobody's, or that app's, and a device asks its inbox again only for Notifications'.
    if sent != [], do: Call.changed(call)
    # A phone opens what it's told to as a link to an app's page (`open` in the UI library), its page too.
    opens = opens(app, app && inputs["page"], app && inputs["section"])
    %{sent: sent, muted: quiet, unwoken: push(call, devices, %{title: title, text: inputs["text"], app: opens})}
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
          select id, app, page, section, title, text, at, read from notifications where user = ? and id > ?
          order by id desc limit ?
          """,
          [user, Map.get(inputs, "after", 0), @shown]
        )

      names = names(user)
      shown =
        for notification <- notifications do
          {section, notification} = Map.pop(notification, "section")
          opens = opens(notification["app"], notification["page"], section)
          Map.merge(notification, %{"read" => notification["read"] == 1, "from" => names[notification["app"]], "opens" => opens})
        end

      %{unread: unread, notifications: shown}
    end)
  end

  # What a tap opens, as the UI library's `open` takes it: the app's section in Settings (`settings.sections`), with
  # what follows # there after the section's own, its page, or its first. A notification opens only its own app's.
  defp opens(nil, _page, _section), do: nil

  defp opens(app, _page, section) when is_binary(section) do
    [component | rest] = String.split(section, "#", parts: 2)
    "settings/client/index.html#" <> Enum.join(["#{app}/#{component}" | rest], "&")
  end

  defp opens(app, page, _section) when is_binary(page), do: "#{app}/#{page}"
  defp opens(app, _page, _section), do: app

  def call("read", inputs, call) do
    %{read: changed(call, inputs, "update notifications set read = 1", "No notification is read")}
  end

  def call("clear", inputs, call) do
    %{cleared: changed(call, inputs, "delete from notifications", "No notification is cleared")}
  end

  def call("apps", _inputs, call) do
    user = person(call)
    names = names(user)

    transaction(call, fn db ->
      apps =
        query(
          db,
          """
          select app, not exists (select 1 from quiet where quiet.user = ? and quiet.app = apps.app) as allowed from (
          select app from notifications where user = ? and app is not null union select app from quiet where user = ?
        ) as apps order by app
        """,
          [user, user, user]
        )

      %{apps: Enum.map(apps, &Map.merge(&1, %{"allowed" => &1["allowed"] == 1, "name" => names[&1["app"]] || &1["app"]}))}
    end)
  end

  def call("allow", %{"app" => app, "allow" => allow}, call) do
    user = person(call)

    transaction(call, fn db ->
      if allow,
        do: query(db, "delete from quiet where user = ? and app = ?", [user, app]),
        else: query(db, "insert or ignore into quiet (user, app) values (?, ?)", [user, app])
    end)

    %{app: app, allowed: allow}
  end

  def call("register", %{"via" => via, "token" => token, "name" => name, "key" => key, "device" => device}, call) do
    user = person(call)
    at = DateTime.to_iso8601(DateTime.utc_now(:second))

    transaction(call, fn db ->
      query(db, "insert or replace into devices (token, user, via, name, at, key, device) values (?, ?, ?, ?, ?, ?, ?)", [
        token,
        user,
        via,
        name,
        at,
        key,
        device
      ])
    end)

    %{device: name}
  end

  def call("unregister", %{"token" => token}, call) do
    user = person(call)

    removed = "delete from devices where token = ? and user = ? returning name"

    case transaction(call, &query(&1, removed, [token, user])) do
      [%{"name" => name}] -> %{device: name}
      [] -> raise "No device of yours has that token.\n\n`devices` lists them."
    end
  end

  def call("devices", _inputs, call) do
    user = person(call)
    %{devices: transaction(call, &query(&1, "select name, via, at from devices where user = ? order by at", [user]))}
  end

  # Marks one of a person's notifications, or all of them, with `sql`, which names the table and what it does.
  defp changed(call, inputs, sql, none) do
    user = person(call)

    transaction(call, fn db ->
      case inputs do
        %{"id" => id} ->
          if query(db, sql <> " where user = ? and id = ? returning id", [user, id]) == [],
            do: raise("#{none}: you have none with id #{id}.\n\n`inbox` lists them.")

          1

        _ ->
          length(query(db, sql <> " where user = ? returning id", [user]))
      end
    end)
  end

  # What a person calls each app installed for them, its title, by its name.
  defp names(user), do: Map.new(OneCore.Apps.running(user), fn {app, version} -> {app, version.title} end)

  # Who a notification goes to: a user, or the users in a group or the network, whom permissions says.
  defp users(_call, "users/" <> _ = user), do: [user]

  defp users(call, "groups/" <> _ = to), do: Call.tool(call, "permissions", "members", %{"of" => to})["members"]
  defp users(call, "network"), do: Call.tool(call, "permissions", "members", %{"of" => "network"})["members"]

  defp users(_call, to), do: raise("#{to} isn't a user, a group or the network.")

  # The app that called, by the name it's installed as, and never one that's asked for: what a notification opens is
  # only the app that sent it. Whoever called it from outside, a person or an agent, sends it as nobody's.
  defp app(%Call{caller: nil}), do: nil

  defp app(call) do
    Enum.find_value(OneCore.Apps.running(call.subject), fn {name, version} -> version.artifact == call.caller and name end)
  end

  # Whoever a call may notify is the user asking, whoever it runs for, the users in a group with the user it runs for
  # or in the group it runs for, and everyone for the network's owner and for an app installed for the network, which
  # no one is asking. Anyone else isn't theirs to tell.
  defp told!(users, call) do
    case known(call) do
      :everyone ->
        users

      known ->
        case users -- known do
          [] -> users
          others -> raise Call.Refused, "You can notify only yourself and people you share a group with, not #{Enum.join(others, ", ")}."
        end
    end
  end

  defp known(%Call{actor: nil, subject: "network"}), do: :everyone

  defp known(%Call{actor: actor, subject: subject} = call) do
    own = Enum.filter([actor, subject], &match?("users/" <> _, &1))
    together = fn %{"members" => members} -> for "users/" <> _ = user <- members, do: user end

    cond do
      String.starts_with?(subject, "groups/") ->
        own ++ together.(Call.tool(call, "permissions", "members", %{"of" => subject}))

      own == [] ->
        []

      Call.tool(call, "permissions", "check", %{"artifact" => "network", "relation" => "owner"})["may"] ->
        :everyone

      true ->
        for group <- Call.tool(call, "permissions", "groups", %{})["groups"], reduce: own, do: (all -> all ++ together.(group))
    end
  end

  # A person's inbox is theirs whoever the call runs for: a group or the network they asked for.
  defp person(%Call{actor: "users/" <> _ = by}), do: by
  defp person(_call), do: raise("An inbox is a person's, and nobody is asking.")

  # Has this app's edge function wake the phones and browsers it went to, each sent the notification encrypted to
  # it, and forgets the ones whose push service says they're gone. Gives the ones that weren't woken, and why: with no
  # address, there's nothing to send through.
  defp push(_call, [], _notification), do: []

  defp push(call, devices, notification) do
    with address when is_binary(address) <- Application.get_env(:one_core, :address),
         json = JSON.encode!(notification),
         sent = Enum.map(devices, &%{token: &1["token"], via: &1["via"], sealed: sealed(&1["key"], json)}),
         {:ok, %{"gone" => gone, "failed" => failed}} <- ask(address, %{devices: sent}) do
      forget(call, gone)

      names = Map.new(devices, &{&1["token"], &1["name"]})
      for %{"token" => token, "why" => why} <- failed, do: %{device: names[token], why: why}
    else
      nil -> for device <- devices, do: %{device: device["name"], why: "this network has no address"}
      {:error, why} -> for device <- devices, do: %{device: device["name"], why: why}
    end
  end

  # Forgets the devices whose push service says their tokens are gone.
  defp forget(_call, []), do: :ok

  defp forget(call, gone) do
    forget = "delete from devices where token in (select value from json_each(?))"
    transaction(call, &query(&1, forget, [JSON.encode!(gone)]))
  end

  defp sealed(key, json) do
    {:ok, sealed} = Cipher.seal(key, "one/notifications/1/notification\n" <> key, json)
    Base.encode64(sealed)
  end

  # Asks this app's edge function, signed as this server, which it answers only for a server
  # acting for the network.
  defp ask(address, what) do
    body = JSON.encode!(what)
    path = "/notifications/send"

    case Req.post(address <> path, body: body, headers: Edge.signed(path, body), retry: false) do
      {:ok, %{status: 200, body: response}} -> {:ok, response}
      {:ok, %{status: status, body: %{"error" => why}}} -> {:error, "the network's edge answered #{status}: #{why}"}
      {:ok, %{status: status}} -> {:error, "the network's edge answered #{status}"}
      {:error, error} -> {:error, Exception.message(error)}
    end
  end

  defp transaction(call, fun) do
    database = Call.app(call, "database")
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values), do: database.query(db, sql, values)
end
