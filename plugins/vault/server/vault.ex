defmodule Vault do
  @moduledoc """
  The Vault: the one place for everything someone has given access to, used without being seen: keys, passwords,
  sign-ins, accounts and Chrome profiles. Each is an item with fields, some of them secret, the sites its secrets are
  sent to, what may use it (the plugins its person switched on for it, and their agents if they said so), and which of
  three states its plugin says it's in. Other plugins register the kinds of item they keep (`vault.types`).

  Each item is an artifact of its own, `<owner>/made/vault/<id>`, encrypted with their Vault's keys (`Encryption`), so
  only the servers they chose open it, and deleted, restored and purged by soft delete as anything a plugin made is:
  the Vault stores and encrypts, and keeps no lifecycle of its own. Their Vault's database (`Database`), encrypted the
  same, says which item each name is, and keeps what's been asked for and the sign-ins under way. A secret gets here
  encrypted on its person's device for those servers, and leaves only inside a request to one of its item's sites, or
  to the code of a plugin switched on for it. No tool here answers with one, and nothing a tool raises says one: what
  handles them raises only words of its own (`quietly/2`).

  A plugin uses an account through the Vault's tools, which act for the plugin that calls them (its `caller`), so
  no secret reaches its code: `fetch` makes a web request with an item's fields filled in, `sign_in` starts an
  OAuth sign-in whose tokens become an item (`Vault.OAuth`), which `fetch` then keeps fresh, `items` lists what it
  may use, `state` says which state one is in, and `stop_using` switches one off for it. The rest of the tools are its
  person's, and refuse a plugin.

  A plugin trusted with secrets themselves, whose service hands one to a program, calls the functions: `value/3`
  gives one field, `keep/4` keeps secrets the plugin made itself, `state/5` says an item's state, `remove/2` deletes
  one through soft delete, `items/1` lists what it may use, and `write_contents/4` and `read_contents/4` keep and read
  what's too large for a field.
  """
  @behaviour OneCore.Plugin

  # A network has one Vault, where every plugin keeps its keys and outside accounts.
  @impl true
  def icon, do: "client/icon.svg"

  @impl true
  def essential, do: true

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are people's keys and sign-ins.
  @impl true
  def sensitive, do: true

  alias OneCore.{Artifact, Call, Plugins, Server, User}
  alias OneCore.Plugins.Calls
  alias Vault.OAuth

  @schema [
    """
    create table items (
      id text primary key, name text not null unique, hosts text not null, agents integer not null default 0,
      oauth text, changed integer not null
    ) without rowid
    """,
    "create table fields (item text not null, name text not null, value text not null, secret integer not null, unique (item, name))",
    "create table uses (item text not null, plugin text not null, primary key (item, plugin)) without rowid",
    """
    create table requests (
      id text primary key, title text not null, fields text not null, hosts text not null, plugin text, why text,
      state text not null, at integer not null
    ) without rowid
    """,
    """
    create table sign_ins (
      state text primary key, verifier text not null, service text not null, plugin text not null,
      item text not null, at integer not null
    ) without rowid
    """,
    """
    create table contents (
      item text not null, name text not null, blob text not null, key text not null, size integer not null,
      at integer not null, primary key (item, name)
    ) without rowid
    """,
    # Which item each name is, now that each item is an artifact of its own. The tables before it are what was kept
    # before that, which each item was copied from (`moved/2`).
    "create table artifacts (id text primary key, name text not null)"
  ]

  # How long a request waits for its person, and a sign-in for its browser, in seconds.
  @request_waits 7 * 86_400
  @sign_in_waits 600
  # The most of an answer's body that a tool answers with, in bytes.
  @most 256 * 1024

  @impl true
  def uses, do: ["encryption", "database"]

  # The kinds of item other plugins keep here, which they register by answering `vault.types` (SKILL.md).
  @impl true
  def offers do
    [
      %{
        name: "types",
        kind: :collect,
        description:
          "The kinds of item a plugin keeps in its people's Vaults, which the Vault lists and opens as the plugin says. " <>
            "It's about nothing. A plugin with item types answers with its list of them, each a map: `name`, one item " <>
            "of it as a person reads it (Chrome profile); `prefix`, what its items' names start with, for the items " <>
            "switched on for the plugin (\"Chrome profile: \"), what follows being the item as a person reads it; and " <>
            "`section`, the heading it's listed under (Chrome Profiles, or Accounts, which several plugins' types share). " <>
            "`component` names a Solid component the plugin's client exports (client/package.json's `exports`, as " <>
            "./<component>), which draws one item beside the list, given `item` and `subject`; and `create` is how one " <>
            "is added, an action as `state/5` takes one: `%{tool: \"create\", name: true}` asks its person for a name " <>
            "first."
      }
    ]
  end

  @impl true
  def apps, do: [%{name: "Vault", page: "client/index.html", category: "access", subject: "user"}]

  @impl true
  def tools do
    item = %{type: "string", description: "The item's name"}

    hosts = %{
      type: "array",
      items: %{type: "string"},
      description: "The sites its secrets are sent to, like api.example.com"
    }

    secret = %{type: "string", format: "secret", unlocked_on: "unlocked_on"}

    [
      %{
        name: "list",
        read_only: true,
        description: """
        Lists the Vault's items: each one's fields (a secret one never with its value), the sites its secrets are
        sent to, the plugins switched on for it and whether agents may use it. With them, the requests still
        waiting for their person, and the plugins that use the Vault.
        """,
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            items: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  artifact: %{type: "string", description: "The artifact it is, which soft delete deletes and restores it by"},
                  type: %{type: ["string", "null"], description: "Its type's id, as `types` lists them, if it's of one"},
                  state: %{
                    type: ["object", "null"],
                    description: "Null when it's ready; else busy with a message, or action_required with why and an action, as its plugin says"
                  },
                  fields: %{
                    type: "array",
                    items: %{
                      type: "object",
                      properties: %{
                        name: %{type: "string"},
                        secret: %{type: "boolean"},
                        value: %{type: ["string", "null"], description: "Only a field that isn't secret's"}
                      },
                      required: ["name", "secret"]
                    }
                  },
                  hosts: %{type: "array", items: %{type: "string"}, description: "The sites its secrets are sent to"},
                  plugins: %{type: "array", items: %{type: "string"}, description: "The plugins switched on for it"},
                  agents: %{type: "boolean", description: "Whether agents may use it, with fetch"},
                  sign_in: %{type: "boolean", description: "Whether a sign-in made it"},
                  changed: %{type: ["integer", "null"], description: "When it last changed, in milliseconds since 1970"}
                },
                required: ["name", "artifact", "type", "state", "fields", "hosts", "plugins", "agents", "sign_in"]
              }
            },
            requests: %{
              type: "array",
              description: "The requests still waiting for their person",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string"},
                  title: %{type: "string"},
                  fields: %{type: "array", items: %{type: "object"}},
                  hosts: %{type: "array", items: %{type: "string"}},
                  plugin: %{type: ["string", "null"]},
                  why: %{type: ["string", "null"]}
                },
                required: ["id", "title", "fields", "hosts", "plugin", "why"]
              }
            },
            plugins: %{type: "array", items: %{type: "string"}, description: "The plugins that use the Vault"},
            types: %{
              type: "array",
              description: "The kinds of item plugins keep here (`vault.types`)",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string"},
                  plugin: %{type: "string", description: "The plugin's name, which its tools start with"},
                  title: %{type: ["string", "null"], description: "The plugin's name, as a person reads it"},
                  icon: %{type: ["string", "null"], description: "The plugin's icon, a data: address"},
                  trusted: %{type: "boolean", description: "Whether its component is drawn in the Vault's page, or in a frame of its own"},
                  name: %{type: "string", description: "One item of it, as a person reads it"},
                  prefix: %{type: "string", description: "What its items' names start with"},
                  section: %{type: "string", description: "The heading it's listed under"},
                  component: %{type: ["string", "null"], description: "What the plugin's client exports that draws one"},
                  create: %{type: ["object", "null"], description: "How one is added: a tool of the plugin's, or a page of its"}
                },
                required: ~w(id plugin title icon trusted name prefix section component create)
              }
            }
          },
          required: ["items", "requests", "plugins", "types"]
        }
      },
      %{
        name: "set",
        description: """
        Keeps a key, token or password someone pasted (or any field) in an item, making the item if there's none by
        that name. Nobody reads a secret back, not you either. When the person has the value, use `request`
        instead: never ask them to paste it into the chat.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            field: %{type: "string", description: "The field's name, like token or password"},
            value: Map.put(secret, :description, "The field's value. Your device encrypts it before it's sent"),
            secret: %{type: "boolean", description: "Never shown again. True if left out: say false for a username"},
            hosts: hosts
          },
          required: ["item", "field", "value"]
        },
        outputs: %{type: "object", properties: %{item: item, field: %{type: "string"}}, required: ["item", "field"]}
      },
      %{
        name: "remove",
        description:
          "Takes a field out of an item. Deleting a whole item is soft delete's, as for anything an app made: `soft-delete_delete` with the item's artifact, as `list` gives it, so it can be restored for 30 days.",
        inputs: %{
          type: "object",
          properties: %{item: item, field: %{type: "string", description: "The field to take out"}},
          required: ["item", "field"]
        },
        outputs: %{type: "object", properties: %{item: item, field: %{type: "string"}}, required: ["item", "field"]}
      },
      %{
        name: "use",
        description: """
        Switches a plugin on for an item, so its code may use the item's fields, or with `on` false, off. That
        trusts the plugin with the secret. Only the person lets agents use one: ask them, with `request`.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            plugin: %{type: "string", description: "The plugin's name, like gmail"},
            on: %{type: "boolean", description: "False switches it off"}
          },
          required: ["item", "plugin"]
        },
        outputs: %{
          type: "object",
          properties: %{item: item, plugin: %{type: "string"}, on: %{type: "boolean"}},
          required: ["item", "plugin", "on"]
        }
      },
      %{
        name: "agents",
        description: """
        Lets agents use an item with `fetch`, or with `on` false, no longer. This is the person's to decide, in the
        Vault's app or by answering a request: an agent asks with `request`, and doesn't call this for itself.
        """,
        inputs: %{
          type: "object",
          properties: %{item: item, on: %{type: "boolean", description: "False stops it"}},
          required: ["item"]
        },
        outputs: %{type: "object", properties: %{item: item, on: %{type: "boolean"}}, required: ["item", "on"]}
      },
      %{
        name: "request",
        description: """
        Asks the person for a key, token, password or sign-in, for a plugin or for you to use, instead of handling
        its value yourself. Answers with a link to give them: they pick one of their items or type a new one on
        their own device, and you never see it. Tell them, and check `status` when they say it's done.
        """,
        inputs: %{
          type: "object",
          properties: %{
            title: %{type: "string", description: "The item's name, like GitHub token"},
            fields: %{
              type: "object",
              description: "The fields it needs, by name: {\"token\": {\"secret\": true}}",
              additionalProperties: %{type: "object", properties: %{secret: %{type: "boolean"}}}
            },
            hosts: hosts,
            plugin: %{type: "string", description: "The plugin that will use it. Left out, it's for you, with `fetch`"},
            why: %{type: "string", description: "One line the person sees: what it's for, and where they find it"}
          },
          required: ["title", "fields"]
        },
        outputs: %{
          type: "object",
          properties: %{
            id: %{type: "string", description: "The request's id, for status"},
            link: %{type: ["string", "null"], description: "The link to give the person, where the network has an address"},
            message: %{type: "string", description: "What to tell them"}
          },
          required: ["id", "link", "message"]
        }
      },
      %{
        name: "status",
        read_only: true,
        description: "Says how a request went: waiting, set, declined or expired. Never a value.",
        inputs: %{
          type: "object",
          properties: %{id: %{type: "string", description: "The request's id"}},
          required: ["id"]
        },
        outputs: %{
          type: "object",
          properties: %{title: %{type: "string"}, state: %{type: "string", enum: ~w(waiting set declined expired)}},
          required: ["title", "state"]
        }
      },
      %{
        name: "answer",
        description: """
        Answers a request, as its person does on their own device: with one of their items, with new values, or by
        declining. Allowing it switches the item on for whoever asked.
        """,
        inputs: %{
          type: "object",
          properties: %{
            id: %{type: "string", description: "The request's id"},
            item: Map.put(item, :description, "One of their items to use for it"),
            values: Map.put(secret, :description, "New values, as JSON by field name. The device encrypts them"),
            decline: %{type: "boolean", description: "True declines it"}
          },
          required: ["id"]
        },
        outputs: %{type: "object", properties: %{state: %{type: "string", enum: ~w(set declined)}}, required: ["state"]}
      },
      %{
        name: "fetch",
        description: """
        Makes a web request with an item's fields filled in, without seeing them: write {{<field>}} where a value
        goes, in the url, a header or the body. Only to one of the item's sites, and only an item its person lets
        agents use, or, called by a plugin, one switched on for that plugin. Answers status, headers and body, with
        any secret the site echoed blotted out: body_base64 where it isn't text.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            url: %{type: "string", description: "The URL, with {{<field>}} where a value goes"},
            method: %{type: "string", description: "GET if left out"},
            headers: %{
              type: "object",
              additionalProperties: %{type: "string"},
              description: "Like {\"authorization\": \"Bearer {{token}}\"}"
            },
            body: %{type: "string", description: "The request's body, with {{<field>}} where a value goes"},
            timeout: %{type: "integer", description: "How many seconds it may take to answer. 15 if left out"}
          },
          required: ["item", "url"]
        },
        outputs: %{
          type: "object",
          properties: %{
            status: %{type: "integer"},
            headers: %{type: "object", additionalProperties: %{type: "string"}},
            body: %{type: "string", description: "The body, where it's text"},
            body_base64: %{type: "string", description: "Or the body in base64, where it isn't"},
            cut: %{type: ["string", "null"], description: "How much an agent's answer was cut short by, if it was"}
          },
          required: ["status", "headers", "cut"]
        }
      },
      %{
        name: "items",
        read_only: true,
        description: """
        For a plugin's code: the items switched on for the plugin that calls this, each with the service whose
        sign-in made it, if one did: [{"name": "Google a@example.com", "sign_in": "Google"}].
        """,
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            items: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  sign_in: %{type: ["string", "null"], description: "The service whose sign-in made it, if one did"},
                  artifact: %{type: "string", description: "The artifact it is, as soft delete knows it"},
                  state: %{type: ["object", "null"], description: "Null when it's ready, else what `state` last said of it"}
                },
                required: ["name", "sign_in", "artifact", "state"]
              }
            }
          },
          required: ["items"]
        }
      },
      %{
        name: "sign_in",
        description: """
        For a plugin's code: starts an OAuth sign-in to an outside service for the plugin that calls this, and
        answers the link its person opens in their browser. The tokens become the item `item`, switched on for the
        plugin. `client` is the item holding the client_id the service gave, and its client_secret, which goes only
        to its sites, so the service's token address is one of them.
        """,
        inputs: %{
          type: "object",
          properties: %{
            service: %{
              type: "object",
              properties: %{
                name: %{type: "string", description: "The service's name, like Google"},
                authorize: %{type: "string", description: "Where it signs people in"},
                token: %{type: "string", description: "Where it gives tokens"},
                scopes: %{type: "array", items: %{type: "string"}},
                hosts: Map.put(hosts, :description, "The sites its tokens are sent to"),
                client: %{type: "string", description: "The item holding the client_id it gave"},
                params: %{
                  type: "object",
                  additionalProperties: %{type: "string"},
                  description: "More to ask its sign-in page with"
                }
              },
              required: ["name", "authorize", "token", "scopes", "hosts", "client"]
            },
            item: %{
              type: "string",
              description:
                "The item its tokens become, the service's name if left out. {{<claim>}} is filled in from who signed in, as its OpenID Connect ID token says: Google {{email}}"
            }
          },
          required: ["service"]
        },
        outputs: %{
          type: "object",
          properties: %{url: %{type: "string", description: "The link its person opens in their browser"}},
          required: ["url"]
        }
      },
      %{
        name: "stop_using",
        description: """
        For a plugin's code: switches an item off for the plugin that calls this, as signing out of it does.
        Whatever else it's switched on for still uses it.
        """,
        inputs: %{type: "object", properties: %{item: item}, required: ["item"]},
        outputs: %{type: "object", properties: %{item: item}, required: ["item"]}
      },
      %{
        name: "state",
        description: """
        For a plugin's code: says which of three states an item switched on for the plugin that calls this is in, for
        its person to see in the Vault: ready, which shows nothing; busy, with a `message` (Saving…); or
        action_required, with `message`, why, and `action`, the one thing that fixes it: a tool of the plugin's, called
        as its person, or a page of its client's.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            state: %{type: "string", enum: ~w(ready busy action_required)},
            message: %{type: "string", description: "What's under way, or why its person has to act"},
            action: %{
              type: "object",
              description: "For action_required: {label, tool, inputs} or {label, page}",
              properties: %{
                label: %{type: "string", description: "The button's words, like Sign In"},
                tool: %{type: "string", description: "A tool of the plugin's, like sign_in"},
                inputs: %{type: "object"},
                page: %{type: "string", description: "Or a page of its client's, like client/sign-in.html#+61400000000"}
              },
              required: ["label"]
            }
          },
          required: ["item", "state"]
        },
        outputs: %{
          type: "object",
          properties: %{item: item, state: %{type: "string"}},
          required: ["item", "state"]
        }
      },
      %{
        name: "grant",
        description: """
        Chooses the servers that open your Vault: they hold its key, and run what uses your secrets. `servers` is
        all of them from now on, so one left out no longer opens it. Only a device that holds your key does this
        (`encryption_approve`), and a Vault no server opens keeps no secret.
        """,
        inputs: %{
          type: "object",
          properties: %{
            servers: %{
              type: "array",
              items: %{type: "string"},
              description: "The servers' keys, as `unlocked_on` lists them"
            },
            keys: %{
              type: "string",
              format: "artifact-keys",
              unlocked_on: "unlocked_on",
              description: "Your device fills this in"
            }
          },
          required: ["servers"]
        },
        outputs: %{
          type: "object",
          properties: %{servers: %{type: "array", items: %{type: "string"}, description: "The servers that open it now"}},
          required: ["servers"]
        }
      },
      %{
        name: "unlocked_on",
        read_only: true,
        description: "Says which servers open your Vault, and which may be chosen to.",
        inputs: %{type: "object", properties: %{}},
        outputs: %{
          type: "object",
          properties: %{
            artifact: %{type: "string"},
            epoch: %{type: "integer"},
            servers: %{type: "array", items: %{type: "string"}, description: "The servers that open it"},
            may: %{
              type: "array",
              description: "The servers that may be chosen to",
              items: %{
                type: "object",
                properties: %{key: %{type: "string"}, name: %{type: ["string", "null"]}},
                required: ["key", "name"]
              }
            }
          },
          required: ["artifact", "epoch", "servers", "may"]
        }
      },
      %{
        name: "oauth_callback",
        by_link: true,
        description:
          "Finishes a sign-in to an outside service. The network's edge calls this when the service sends its person back.",
        inputs: %{
          type: "object",
          properties: %{state: %{type: "string"}, code: %{type: "string"}, error: %{type: "string"}},
          required: ["state"]
        },
        outputs: %{
          type: "object",
          properties: %{service: %{type: "string"}, item: %{type: "string", description: "What it's kept as in the Vault"}},
          required: ["service", "item"]
        }
      }
    ]
  end

  @doc """
  One field of an item, for the plugin that calls this, which its person switched on for the item: to hand to a
  program a service brings. What's given is the secret itself, so it goes nowhere a person or an agent reads.
  """
  def value(%Call{} = call, item, field) do
    db = reached(call)

    quietly("Reading #{field}", fn ->
      found = allowed!(find!(call, db, item), {:plugin, call.who})
      fields(found)[field] || raise "#{found["name"]} has no field named #{field}."
    end)
  end

  @doc """
  Keeps `fields`, by their names, in the item `item`, for the plugin that calls this: secrets it made itself or an
  outside service handed it, such as the key a session it holds is encrypted with. Each is secret. An item there's
  none of is made, switched on for the plugin, with `hosts` its sites; one that's there is changed only if it's
  switched on for the plugin.
  """
  def keep(%Call{} = call, item, fields, hosts \\ []) when is_map(fields) do
    # The plugin reaches the Vault as an editor of it from here on, as a sign-in has it.
    share(call, "share", theirs(call), call.who)
    db = reached(call)

    quietly("Keeping #{item}", fn ->
      found =
        case find(call, db, item) do
          nil -> create(call, db, item, hosts, %{"uses" => [call.who]})
          found -> allowed!(found, {:plugin, call.who})
        end

      change!(db, found, fn record -> Enum.reduce(fields, record, fn {name, value}, record -> put_field(record, name, value, true) end) end)
      :ok
    end)
  end

  @doc """
  Deletes the item `item`, for the plugin that calls this, which its person switched on for it or which it made with
  `keep/4`, as anything a plugin made is deleted: by soft delete, which keeps it, contents and all, for 30 days, so it
  can be restored as it was. One that isn't there is nothing to delete.
  """
  def remove(%Call{} = call, item) do
    db = reached(call)

    quietly("Deleting #{item}", fn ->
      if found = find(call, db, item), do: delete!(call, db, allowed!(found, {:plugin, call.who}))
      :ok
    end)
  end

  @doc """
  Renames the item `item` to `to`, for the plugin that calls this, which it's switched on for. Its fields, its
  contents and whatever it's switched on for go with it.
  """
  def rename(%Call{} = call, item, to) do
    db = reached(call)

    quietly("Renaming #{item}", fn ->
      found = allowed!(find!(call, db, item), {:plugin, call.who})
      if String.trim(to) == "", do: raise("An item has a name.")

      transaction(call, db, fn d ->
        if taken(d, db, to, found["id"]) != [], do: raise("There's an item named #{to} already.")
        query(d, "update artifacts set name = ? where id = ?", [to, found["id"]])
      end)

      change!(db, found, &Map.put(&1, "name", to))
      :ok
    end)
  end

  @doc """
  The fields of the item `item`, by their names, for the plugin that calls this, which it's switched on for: as
  `value/3` gives each.
  """
  def values(%Call{} = call, item) do
    db = reached(call)
    quietly("Reading #{item}", fn -> fields(allowed!(find!(call, db, item), {:plugin, call.who})) end)
  end

  @doc """
  Says which of three states the item `item` is in, for the plugin that calls this, which it's switched on for, so
  its person sees it in the Vault: `:ready`, usable, which shows nothing; `:busy`, something's under way, said in
  `message` ("Saving…"), which shows a spinner and it; or `:action_required`, its person has to act, `message` saying
  why ("Signed out on your phone") and `action` the one thing that fixes it, which shows a dot, why and a button. The
  plugin keeps its own detailed states and says only which of these it's in. `action` is the plugin's own:
  `%{label: "Sign In", tool: "sign_in", inputs: %{account: "+614..."}}`, a tool of the plugin's, called as its person,
  whose `url`, if it gives one, opens in their browser; or `%{label: "Sign In", page: "client/sign-in.html#+614..."}`,
  a page of the plugin's, opened over the Vault. Whether its person is notified too is the plugin's to choose, with
  `notifications`.
  """
  def state(%Call{} = call, item, state, message \\ nil, action \\ nil) do
    db = reached(call)
    said = said!(state, message, action, plugin_name(call, call.who))

    quietly("Changing #{item}", fn ->
      found = allowed!(find!(call, db, item), {:plugin, call.who})
      if found["state"] != said, do: change!(db, found, &Map.put(&1, "state", said))
      Call.changed(call)
      :ok
    end)
  end

  @doc """
  Keeps the contents `name` of the item `item`, for the plugin that calls this, which it's switched on for: what's
  too large for a field, like a Chrome profile's hundreds of MB, kept as 1Password keeps a file. They're a blob of the
  item's, beside its record, sealed under a key made for that blob alone, which only the item holds, so soft delete
  deletes and restores them with it.

  `send` is given where they go, `%{"link" => link, "key" => key}`: a short-lived link that writes one blob, and the
  key, base64. It hands them to whatever holds the bytes, which seals them a chunk at a time and sends them, as
  Containers' `upload` does, so they never pass through a tool's result whatever their size, and gives how many bytes
  it sent. The item holds them once it has, and only then do the contents they replace go, so a write that fails
  leaves the last one whole. Gives the size.
  """
  def write_contents(%Call{} = call, item, name, send) when is_function(send, 1) do
    db = reached(call)
    {blob, key} = {Base.encode16(:crypto.strong_rand_bytes(16), case: :lower), Base.encode64(:crypto.strong_rand_bytes(32))}

    quietly("Keeping #{name} of #{item}", fn ->
      found = allowed!(find!(call, db, item), {:plugin, call.who})
      path = "#{found["id"]}/contents/#{blob}"
      size = send.(%{"link" => link(db, path, :write), "key" => key})
      kept = %{"blob" => path, "key" => key, "size" => size, "at" => now()}
      change!(db, found, &Map.update(&1, "contents", %{name => kept}, fn contents -> Map.put(contents, name, kept) end))
      was = found["contents"][name]
      if was && was["blob"] != path, do: Artifact.remove(root(db), was["blob"])
      size
    end)
  end

  @doc """
  Reads the contents `name` of the item `item`, for the plugin that calls this, which it's switched on for:
  `receive` is given where they are, `%{"link" => link, "key" => key, "size" => size}`, a short-lived link that reads
  the blob and its key, to hand to whatever takes the bytes, which fetches and opens them a chunk at a time, as
  Containers' `download` does. Gives what `receive` gives, or `nil` where the item has no such contents yet.
  """
  def read_contents(%Call{} = call, item, name, receive) when is_function(receive, 1) do
    db = reached(call)

    quietly("Reading #{name} of #{item}", fn ->
      found = allowed!(find!(call, db, item), {:plugin, call.who})

      if kept = found["contents"][name],
        do: receive.(%{"link" => link(db, kept["blob"], :read), "key" => kept["key"], "size" => kept["size"]})
    end)
  end

  @doc """
  The items switched on for the plugin that calls this, by name, each with the service whose sign-in made it, if one
  did, its artifact, which soft delete knows it by, and its state, as `state/5` last said it:
  `[%{name: "Google a@example.com", sign_in: "Google", artifact: "users/a/made/vault/x1", state: nil}]`. None before
  anything is.
  """
  def items(%Call{} = call) do
    if Call.reaches?(call, theirs(call), "editor") do
      db = reached(call)
      used_by(db, everything(call, db), call.who)
    else
      []
    end
  end

  defp used_by(db, items, plugin) do
    for item <- Enum.sort_by(items, &String.downcase(&1["name"])), plugin in item["uses"] do
      %{
        name: item["name"],
        sign_in: item["oauth"] && item["oauth"]["name"],
        artifact: artifact(db, item["id"]),
        state: item["state"]
      }
    end
  end

  # The tools a plugin's code calls, which act for it. The rest are its person's.
  @plugins ~w(fetch items sign_in stop_using state)

  @impl true
  def hooks, do: ["renamed"]

  # The items switched on for a renamed plugin are switched on for it by its new name, in the Vault of whoever it's
  # renamed for, if they have one.
  @impl true
  def hook("renamed", %{"was" => was, "is" => is}, %Call{subject: "users/" <> _} = call) do
    if Artifact.list(items_artifact(call), "") != [] do
      db = own(call)

      for item <- everything(call, db), was in item["uses"],
          do: change!(db, item, &Map.update!(&1, "uses", fn uses -> Enum.uniq(for use <- uses, do: if(use == was, do: is, else: use)) end))
    end

    :ok
  end

  def hook("renamed", _about, _call), do: :ok

  @impl true
  def call(tool, _inputs, %Call{caller: caller}) when caller != nil and tool not in @plugins,
    do: raise("vault_#{tool} is its person's, not a plugin's. A plugin calls #{Enum.join(@plugins, ", ")}.")

  def call(tool, _inputs, %Call{caller: nil}) when tool in ~w(items sign_in stop_using state),
    do: raise("vault_#{tool} is for a plugin's code, and acts for the plugin that calls it.")

  def call("items", _inputs, call) do
    db = own(call)
    %{items: used_by(db, everything(call, db), call.caller)}
  end

  def call("stop_using", %{"item" => name}, call) do
    db = own(call)
    change!(db, find!(call, db, name), &Map.update!(&1, "uses", fn uses -> uses -- [call.caller] end))
    %{item: name}
  end

  def call("state", %{"item" => name, "state" => state} = inputs, call) do
    db = own(call)
    said = said!(state, inputs["message"], inputs["action"], plugin_name(call, call.caller))
    found = allowed!(find!(call, db, name), {:plugin, call.caller})
    if found["state"] != said, do: change!(db, found, &Map.put(&1, "state", said))
    Call.changed(call)
    %{item: name, state: state}
  end

  def call("sign_in", %{"service" => given} = inputs, call) do
    address = address()
    db = own(call)

    service = %{
      name: given["name"],
      authorize: given["authorize"],
      token: given["token"],
      scopes: given["scopes"],
      hosts: hosts!(given["hosts"]),
      client: given["client"],
      params: Map.get(given, "params", %{})
    }

    quietly("Starting the sign-in", fn ->
      {state, verifier} = {"#{Base.url_encode64(call.subject, padding: false)}.#{random(24)}", random(48)}
      item = find!(call, db, service.client)
      client = fields(item)

      client["client_id"] ||
        raise "#{service.client} has no client_id. Keep the one #{service.name} gave you there."

      # Its client_secret goes to the token address, as any secret goes only to one of its item's sites.
      if client["client_secret"] && not OAuth.to?(service.token, item["hosts"]),
        do:
          raise(
            "#{service.client}'s secrets go only to its sites, and #{service.name}'s token address isn't one. Its person adds it, in the Vault."
          )

      stored = JSON.encode!(Map.take(service, [:name, :token, :hosts, :client]))

      transaction(call, db, fn d ->
        query(d, "delete from sign_ins where at < ?", [now() - @sign_in_waits])

        query(d, "insert into sign_ins (state, verifier, service, plugin, item, at) values (?, ?, ?, ?, ?, ?)", [
          state,
          verifier,
          stored,
          call.caller,
          inputs["item"] || service.name,
          now()
        ])
      end)

      %{url: OAuth.link(service, client["client_id"], address <> "/vault/callback", state, verifier)}
    end)
  end

  def call("unlocked_on", _inputs, call), do: encryption(call).unlocked_on(call, items_artifact(call))

  def call("grant", %{"servers" => _servers} = inputs, call) do
    keys =
      inputs["keys"] ||
        raise "Which servers open your Vault is chosen in the Vault's app on a trusted device.\n\nOr with `one vault grant` on it."

    %{servers: encryption(call).choose(call, items_artifact(call), keys)}
  end

  def call("list", _inputs, call) do
    db = own(call)
    names = names(call)
    types = types(call)

    use_it =
      Enum.sort(
        for {name, version} <- Plugins.running(call.subject),
            "vault" in version.uses or is_map_key(version.requires, "vault"),
            do: name
      )

    items =
      for item <- Enum.sort_by(everything(call, db), &String.downcase(&1["name"])) do
        fields =
          for field <- item["fields"] do
            if field["secret"],
              do: %{name: field["name"], secret: true},
              else: %{name: field["name"], secret: false, value: field["value"]}
          end

        plugins = for plugin <- item["uses"], do: named(names, plugin)

        # An item is of a plugin's type when it's switched on for that plugin and named as the type says.
        type = Enum.find(types, &(&1.plugin in plugins and String.starts_with?(item["name"], &1.prefix)))

        %{
          name: item["name"],
          artifact: artifact(db, item["id"]),
          type: type && type.id,
          state: item["state"],
          fields: fields,
          hosts: item["hosts"],
          plugins: Enum.sort(plugins),
          agents: item["agents"],
          sign_in: item["oauth"] != nil,
          changed: item["changed"]
        }
      end

    waiting =
      "select id, title, fields, hosts, plugin, why from requests where state = 'waiting' and at > ? order by at"

    requests =
      transaction(call, db, fn d ->
        for request <- query(d, waiting, [now() - @request_waits]) do
          %{
            id: request["id"],
            title: request["title"],
            fields: JSON.decode!(request["fields"]),
            hosts: JSON.decode!(request["hosts"]),
            plugin: request["plugin"] && named(names, request["plugin"]),
            why: request["why"]
          }
        end
      end)

    %{items: items, requests: requests, plugins: use_it, types: types}
  end

  def call("set", %{"item" => name, "field" => field, "value" => value} = inputs, call) do
    db = own(call)

    quietly("Keeping #{field}", fn ->
      value = encryption(call).secret(call, value)
      found = find(call, db, name) || create(call, db, name, Map.get(inputs, "hosts", []))

      change!(db, found, fn record ->
        record = if hosts = inputs["hosts"], do: Map.put(record, "hosts", hosts!(hosts)), else: record
        put_field(record, field, value, Map.get(inputs, "secret", true))
      end)

      %{item: name, field: field}
    end)
  end

  # Deleting an item is soft delete's, as for anything a plugin made: this takes out one of its fields.
  def call("remove", %{"item" => name, "field" => field}, call) do
    db = own(call)
    found = find!(call, db, name)
    if fields(found)[field] == nil, do: raise("#{name} has no field named #{field}.")
    change!(db, found, &Map.update!(&1, "fields", fn fields -> Enum.reject(fields, fn kept -> kept["name"] == field end) end))
    %{item: name, field: field}
  end

  # A plugin switched on for an item is an editor of the Vault's artifact too, which is what lets its code reach it
  # at all, and goes when no item is switched on for it.
  def call("use", %{"item" => name, "plugin" => plugin} = inputs, call) do
    db = own(call)

    who =
      Plugins.plugin(call.subject, plugin) || Plugins.plugin("network", plugin) ||
        raise "There's no plugin named #{plugin}."

    found = find!(call, db, name)

    if Map.get(inputs, "on", true) do
      change!(db, found, &Map.update!(&1, "uses", fn uses -> Enum.uniq(uses ++ [who]) end))
      share(call, "share", Artifact.name(db), who)
      %{item: name, plugin: plugin, on: true}
    else
      change!(db, found, &Map.update!(&1, "uses", fn uses -> uses -- [who] end))
      if not Enum.any?(everything(call, db), &(who in &1["uses"])), do: share(call, "unshare", Artifact.name(db), who)
      %{item: name, plugin: plugin, on: false}
    end
  end

  def call("agents", %{"item" => name} = inputs, call) do
    on = Map.get(inputs, "on", true)
    db = own(call)
    change!(db, find!(call, db, name), &Map.put(&1, "agents", on))
    %{item: name, on: on}
  end

  def call("request", %{"title" => title, "fields" => fields} = inputs, call) do
    db = own(call)

    plugin =
      if name = inputs["plugin"],
        do:
          Plugins.plugin(call.subject, name) || Plugins.plugin("network", name) ||
            raise("There's no plugin named #{name}.")

    if fields == %{}, do: raise("A request names the fields it needs.")
    fields = for {name, about} <- fields, do: %{name: field!(name), secret: Map.get(about, "secret", true)}
    id = random(9)

    transaction(call, db, fn d ->
      query(
        d,
        "insert into requests (id, title, fields, hosts, plugin, why, state, at) values (?, ?, ?, ?, ?, ?, 'waiting', ?)",
        [
          id,
          title,
          JSON.encode!(fields),
          JSON.encode!(hosts!(Map.get(inputs, "hosts", []))),
          plugin,
          inputs["why"],
          now()
        ]
      )
    end)

    link = if address = Application.get_env(:one_core, :address), do: "#{address}/vault/request?id=#{id}"
    where = if link, do: "Give them this link: #{link}. It", else: "Ask them to open Vault on a device of theirs: it"

    %{
      id: id,
      link: link,
      message: "#{where} shows the request, to answer on their own device. You never see what they type."
    }
  end

  def call("status", %{"id" => id}, call) do
    transaction(call, own(call), fn d ->
      case query(d, "select title, state, at from requests where id = ?", [id]) do
        [%{"title" => title, "state" => "waiting", "at" => at}] ->
          %{title: title, state: if(at > now() - @request_waits, do: "waiting", else: "expired")}

        [%{"title" => title, "state" => state}] ->
          %{title: title, state: state}

        [] ->
          raise "There's no request #{id}."
      end
    end)
  end

  def call("answer", %{"id" => id} = inputs, call) do
    db = own(call)

    quietly("Answering the request", fn ->
      values = if given = inputs["values"], do: JSON.decode!(encryption(call).secret(call, given))

      request =
        transaction(call, db, fn d ->
          case query(d, "select * from requests where id = ? and state = 'waiting' and at > ?", [id, now() - @request_waits]) do
            [request] -> request
            [] -> raise "That request isn't waiting any more."
          end
        end)

      answered =
        cond do
          inputs["decline"] == true ->
            nil

          name = inputs["item"] ->
            allow!(db, request, find!(call, db, name))

          is_map(values) ->
            wanted = JSON.decode!(request["fields"])
            missing = for %{"name" => name} <- wanted, values[name] in [nil, ""], do: name
            if missing != [], do: raise("It needs #{Enum.join(missing, ", ")} too.")
            found = find(call, db, request["title"]) || create(call, db, request["title"], JSON.decode!(request["hosts"]))
            filled = change!(db, found, fn record -> Enum.reduce(wanted, record, &put_field(&2, &1["name"], values[&1["name"]], &1["secret"])) end)
            allow!(db, request, filled)

          true ->
            raise "Answer with one of your items, with values, or by declining."
        end

      state = if inputs["decline"] == true, do: "declined", else: "set"
      transaction(call, db, &query(&1, "update requests set state = ? where id = ?", [state, id]))
      if answered && request["plugin"], do: share(call, "share", Artifact.name(db), request["plugin"])
      %{state: state}
    end)
  end

  def call("fetch", %{"item" => item, "url" => url} = inputs, call) do
    request = %{
      url: url,
      method: Map.get(inputs, "method", "GET"),
      headers: Map.get(inputs, "headers", %{}),
      body: inputs["body"],
      timeout: inputs["timeout"]
    }

    by = if call.caller, do: {:plugin, call.caller}, else: :agents
    response = fetched(call, own(call), item, request, by)

    # What an agent reads is cut short, and what a plugin's code reads is whole.
    {body, cut} =
      case {by, response.body} do
        {:agents, <<body::binary-size(@most), rest::binary>>} -> {body, "#{byte_size(rest)} more bytes were left out"}
        {_by, body} -> {body, nil}
      end

    body = if String.valid?(body), do: %{body: body}, else: %{body_base64: Base.encode64(body)}
    Map.merge(%{status: response.status, headers: response.headers, cut: cut}, body)
  end

  # Whoever the service sent back is nobody the network knows: what says whose sign-in this finishes is its
  # `state`, which only the browser that began it was given, and which starts with whose it is.
  # A server that passed this on has checked that, and asks for whoever's it is, by nobody.
  def call("oauth_callback", %{"state" => state} = inputs, %Call{actor: actor} = call) do
    with [whose, _random] <- String.split(state, "."),
         {:ok, owner} <- Base.url_decode64(whose, padding: false),
         true <- User.owner?(owner),
         true <- actor == User.link("#{owner}/#{state}") or (actor == nil and call.subject == owner) do
      call = %{call | subject: owner, actor: nil}
      db = own(call)

      quietly("Finishing the sign-in", fn ->
        sign_in =
          transaction(call, db, fn d ->
            case query(d, "select * from sign_ins where state = ? and at > ?", [state, now() - @sign_in_waits]) do
              [sign_in] -> sign_in
              [] -> raise "This sign-in isn't waiting any more. Start it again."
            end
          end)

        service = JSON.decode!(sign_in["service"])
        if why = inputs["error"], do: raise("#{service["name"]} didn't sign you in: #{why}.")
        code = inputs["code"] || raise "#{service["name"]} sent no code back."
        client = fields(find!(call, db, service["client"]))
        tokens = OAuth.exchange(service, client, code, sign_in["verifier"], address() <> "/vault/callback")
        name = signed_in(sign_in["item"], service, tokens)

        if transaction(call, db, &query(&1, "delete from sign_ins where state = ? returning state", [state])) != [] do
          found = find(call, db, name) || create(call, db, name, service["hosts"])

          # Signed in, it's ready: whatever it needed of its person is done.
          change!(db, found, fn record ->
            record
            |> kept_tokens(service, tokens)
            |> Map.update!("uses", &Enum.uniq(&1 ++ [sign_in["plugin"]]))
            |> Map.put("state", nil)
          end)

          Call.changed(call)
        end

        %{service: service["name"], item: name}
      end)
    else
      _ -> raise "This sign-in isn't one this network began."
    end
  end

  # The item a sign-in's tokens become: its name, with any {{<claim>}} filled in from who the service says signed in.
  defp signed_in(name, service, tokens) do
    claims = OAuth.claims(tokens["id_token"])

    Regex.replace(~r/\{\{([A-Za-z0-9_]+)\}\}/, name, fn _all, claim ->
      case claims[claim] do
        value when is_binary(value) and value != "" -> value
        _ -> raise "#{service["name"]} didn't say who signed in. Sign in again."
      end
    end)
  end

  # An item's fields filled into a request, for what may use it. Its tokens are renewed first if they've ended,
  # and once more if the site says they have.
  defp fetched(call, db, name, request, by) do
    quietly("The request", fn ->
      item = allowed!(find!(call, db, name), by)
      secret = for %{"name" => field, "secret" => true} <- item["fields"], do: field
      oauth = item["oauth"]
      fields = if oauth && oauth["expires"] <= now() + 60, do: renewed(call, db, item, fields(item)), else: fields(item)
      response = OAuth.request(request, fields, secret, item["hosts"])

      if oauth && response.status == 401,
        do: OAuth.request(request, renewed(call, db, item, fields), secret, item["hosts"]),
        else: response
    end)
  end

  # An item's fields with its tokens renewed, unless another call renewed them meanwhile: one renewal at a time.
  defp renewed(call, db, item, had) do
    :global.trans({{__MODULE__, artifact(db, item["id"])}, self()}, fn ->
      latest = record(db, item["id"]) || raise "There's no item named #{item["name"]} in the Vault."

      if fields(latest)["access_token"] != had["access_token"] do
        fields(latest)
      else
        oauth = latest["oauth"]
        client = fields(find!(call, db, oauth["client"]))

        tokens =
          OAuth.renew(
            oauth,
            client,
            fields(latest)["refresh_token"] || raise("#{item["name"]} has no way to renew its sign-in. Sign in again.")
          )

        fields(change!(db, latest, &kept_tokens(&1, oauth, tokens)))
      end
    end)
  end

  defp kept_tokens(record, service, tokens) do
    record =
      Enum.reduce(~w(access_token refresh_token), record, fn field, record ->
        if value = tokens[field], do: put_field(record, field, value, true), else: record
      end)

    oauth = %{
      "name" => service["name"],
      "token" => service["token"],
      "client" => service["client"],
      "expires" => now() + (tokens["expires_in"] || 3600)
    }

    Map.put(record, "oauth", oauth)
  end

  # A request allowed with an item: switched on for whichever plugin asked, or for agents if no plugin did.
  defp allow!(db, request, item) do
    if plugin = request["plugin"],
      do: change!(db, item, &Map.update!(&1, "uses", fn uses -> Enum.uniq(uses ++ [plugin]) end)),
      else: change!(db, item, &Map.put(&1, "agents", true))
  end

  # The item, if `by` may use it: a plugin switched on for it, or agents its person let.
  defp allowed!(item, by) do
    case by do
      :agents ->
        if item["agents"] != true,
          do: raise("#{item["name"]} isn't one agents may use. Ask its person, with vault_request.")

      {:plugin, plugin} ->
        if plugin not in item["uses"],
          do: raise("#{item["name"]} isn't switched on for #{Path.basename(plugin)}. Its person switches it on, in the Vault.")
    end

    item
  end

  # Someone's items, each an artifact of its own, `<owner>/made/vault/<id>`, so soft delete deletes, restores and
  # purges one as it does anything a plugin made, and the Vault keeps no lifecycle of its own: it stores and
  # encrypts. An item's record is the blob `<id>/item`, encrypted with the Vault's keys, as its database is, and
  # changed in one step; its contents are blobs beside it, `<id>/contents/<blob>`, each sealed under a key the record
  # holds. The database says which id each name is (`artifacts`), so finding one by name is one read, and one that
  # soft delete took aside isn't there to read until it's restored.

  # Where someone's items are: their Vault's database, with its keys and its reach, at the folder it's in.
  defp root(%Artifact{} = db), do: %Artifact{db | prefix: String.replace_suffix(db.prefix, "items/", "")}

  # An item's artifact, by its name, as soft delete knows it.
  defp artifact(db, id), do: root(db).prefix <> id

  defp record(db, id) do
    case Artifact.read(root(db), "#{id}/item") do
      {:ok, data, _tag} -> JSON.decode!(data)
      :absent -> nil
    end
  end

  # Changes an item's record in one step: `change` is given it as it is, and gives what it's to be.
  defp change!(db, item, change) do
    changed =
      Artifact.update(root(db), "#{item["id"]}/item", fn
        nil -> nil
        data -> data |> JSON.decode!() |> change.() |> Map.put("changed", now()) |> JSON.encode!()
      end)

    if changed, do: JSON.decode!(changed), else: raise("There's no item named #{item["name"]} in the Vault.")
  end

  # The item named `name`, any case: the first whose record is there.
  defp find(call, db, name) do
    ids =
      transaction(call, db, fn d ->
        moved(d, db)
        for %{"id" => id} <- query(d, "select id from artifacts where name = ? collate nocase", [name]), do: id
      end)

    Enum.find_value(ids, &record(db, &1))
  end

  defp find!(call, db, name), do: find(call, db, name) || raise("There's no item named #{name} in the Vault.")

  # Every item that's there.
  defp everything(call, db) do
    ids =
      transaction(call, db, fn d ->
        moved(d, db)
        for %{"id" => id} <- query(d, "select id from artifacts"), do: id
      end)

    for {:ok, data, _tag} <- Artifact.read_all(root(db), Enum.map(ids, &"#{&1}/item")), do: JSON.decode!(data)
  end

  # The ids of the items there are by the name `name`, other than `id`.
  defp taken(d, db, name, id \\ nil) do
    for %{"id" => other} <- query(d, "select id from artifacts where name = ? collate nocase", [name]),
        other != id,
        record(db, other),
        do: other
  end

  # A new item, made once its name is free: its record, with `record`'s parts.
  defp create(call, db, name, hosts, record \\ %{}) do
    if String.trim(name) == "", do: raise("An item has a name.")
    id = random(9)

    transaction(call, db, fn d ->
      if taken(d, db, name) != [], do: raise("There's an item named #{name} already.")
      query(d, "insert into artifacts (id, name) values (?, ?)", [id, name])
    end)

    item =
      Map.merge(
        %{
          "id" => id,
          "name" => name,
          "hosts" => hosts!(hosts),
          "agents" => false,
          "oauth" => nil,
          "fields" => [],
          "uses" => [],
          "contents" => %{},
          "state" => nil,
          "changed" => now()
        },
        record
      )

    :ok = Artifact.write(root(db), "#{id}/item", JSON.encode!(item))
    item
  end

  # The items the database held before each was an artifact of its own are copied into one, each once, under its
  # id, with what was kept of it, and listed once the copy is written; the database's copy stays as it was.
  defp moved(d, db) do
    for old <- query(d, "select * from items where id not in (select id from artifacts)") do
      fields =
        for field <- query(d, "select name, value, secret from fields where item = ? order by rowid", [old["id"]]),
            do: %{"name" => field["name"], "value" => field["value"], "secret" => field["secret"] == 1}

      contents =
        for kept <- query(d, "select * from contents where item = ?", [old["id"]]), into: %{} do
          {kept["name"], %{"blob" => "contents/#{kept["blob"]}", "key" => kept["key"], "size" => kept["size"], "at" => kept["at"]}}
        end

      record = %{
        "id" => old["id"],
        "name" => old["name"],
        "hosts" => JSON.decode!(old["hosts"]),
        "agents" => old["agents"] == 1,
        "oauth" => old["oauth"] && JSON.decode!(old["oauth"]),
        "fields" => fields,
        "uses" => for(%{"plugin" => plugin} <- query(d, "select plugin from uses where item = ?", [old["id"]]), do: plugin),
        "contents" => contents,
        "state" => nil,
        "changed" => old["changed"]
      }

      :ok = Artifact.write(root(db), "#{old["id"]}/item", JSON.encode!(record))
      query(d, "insert into artifacts (id, name) values (?, ?)", [old["id"], old["name"]])
    end

    :ok
  end

  # Deletes an item as anything a plugin made is deleted: by soft delete, which keeps it for 30 days.
  defp delete!(call, db, item), do: Call.tool(call, "soft-delete", "delete", %{"artifact" => artifact(db, item["id"])})

  defp fields(item), do: Map.new(item["fields"], &{&1["name"], &1["value"]})

  defp put_field(item, name, value, secret) do
    field = %{"name" => field!(name), "value" => value, "secret" => secret == true}

    Map.update!(item, "fields", fn fields ->
      if Enum.any?(fields, &(&1["name"] == name)),
        do: Enum.map(fields, &if(&1["name"] == name, do: field, else: &1)),
        else: fields ++ [field]
    end)
  end

  # What a plugin said of its item's state, checked, as an item keeps it: nothing for `ready`, since fine is silent.
  defp said!(state, message, action, plugin) do
    case to_string(state) do
      "ready" ->
        nil

      "busy" when is_binary(message) ->
        %{"state" => "busy", "message" => message, "by" => plugin}

      "action_required" when is_binary(message) and is_map(action) ->
        action = JSON.decode!(JSON.encode!(action))

        if not is_binary(action["label"]) or not (is_binary(action["tool"]) or is_binary(action["page"])),
          do: raise("An action has a label, and either a tool of the plugin's or a page of its client's.")

        %{"state" => "action_required", "message" => message, "action" => action, "by" => plugin}

      _ ->
        raise "An item is ready, busy with a message, or action_required with why and an action."
    end
  end

  # A short-lived link to one blob of an item's, as what's handed on takes it: JSON's keys.
  defp link(db, blob, mode), do: %{root(db) | codec: nil} |> Artifact.link(blob, mode) |> JSON.encode!() |> JSON.decode!()

  defp field!(name) do
    if is_binary(name) and name =~ ~r/^[A-Za-z0-9_]+$/,
      do: name,
      else: raise("A field's name is letters, digits and _.")
  end

  # A site is a host's name, with its port if it has one: never a path, so a secret goes to all of it or none.
  defp hosts!(hosts) do
    for host <- hosts do
      if is_binary(host) and host =~ ~r/^[a-z0-9.-]+(:\d+)?$/,
        do: host,
        else: raise("A site is a host's name, like api.example.com.")
    end
  end

  # The item types of the plugins whoever this runs for has (`vault.types`), each as `list` gives it, with its
  # plugin's name, title and icon, and whether its plugin is trusted, which decides where its component is drawn. A
  # type that doesn't say what one must is left out.
  defp types(call) do
    {:ok, parts, _failed} = Call.run_hook(call, "types", %{})
    running = Plugins.running(call.subject)

    for {plugin, part} <- Enum.sort(parts),
        version = running[plugin],
        version != nil,
        type <- List.wrap(JSON.decode!(JSON.encode!(part))),
        type?(type) do
      %{
        id: "#{plugin}/#{type["name"]}",
        plugin: plugin,
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
  end

  defp type?(%{"name" => name, "prefix" => prefix, "section" => section})
       when is_binary(name) and is_binary(prefix) and prefix != "" and is_binary(section),
       do: true

  defp type?(_type), do: false

  # The plugins' names as whoever this runs for finds them, by their artifacts.
  defp names(call), do: Map.new(Plugins.running(call.subject), fn {name, version} -> {version.artifact, name} end)
  defp named(names, plugin), do: Map.get(names, plugin, Path.basename(plugin))
  defp plugin_name(call, plugin), do: named(names(call), plugin)

  defp address do
    Application.get_env(:one_core, :address) ||
      raise "A sign-in to an outside service finishes at the network's address, and this network has none."
  end

  defp random(bytes), do: Base.url_encode64(:crypto.strong_rand_bytes(bytes), padding: false)
  defp now, do: System.os_time(:second)

  # A relation tuple is the permissions plugin's to write, as whoever asked.
  defp share(call, tool, artifact, who) do
    inputs = %{"artifact" => artifact, "relation" => "editor", "who" => who}

    case Calls.call("permissions", tool, inputs, {call.actor, call.subject}) do
      {:ok, _said} ->
        :ok

      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} ->
        if(tool == "share", do: raise(why), else: :ok)

      {:forwarded, _result} ->
        :ok

      {:error, why} ->
        if(tool == "share", do: raise(why), else: :ok)
    end
  end

  # Someone's Vault is theirs alone: no relation tuple has anyone else ask for a user, the network's owner included.
  defp mine!(%Call{subject: subject, actor: actor} = call) do
    if String.starts_with?(subject, "users/") and actor not in [subject, nil],
      do: raise("#{subject}'s Vault is theirs alone.")

    call
  end

  # The artifact of the database of whoever a call of the Vault's own tools is for: which item each name is, what's
  # been asked for, and the sign-ins under way.
  defp items_artifact(call), do: Call.make(mine!(call), "items")

  # Their Vault, opened: on a server they chose, which this call ends up on.
  defp own(call), do: encryption(call).encrypted(call, items_artifact(call))

  # The same artifact by its name, for a plugin that uses the Vault, which finds it added as `vault`.
  defp theirs(call), do: "#{mine!(call).subject}/made/vault/items"

  # It reaches the Vault as an editor, which a tuple says once anything of the Vault's is switched on for it.
  defp reached(call), do: encryption(call).encrypted(call, Call.reach(call, theirs(call), "editor"))

  defp encryption(call), do: Call.plugin(call, "encryption")

  # A transaction on someone's Vault's database. It's theirs, so this server's copy of it is kept by its name.
  defp transaction(call, artifact, fun) do
    database = Call.plugin(call, "database")
    call = %{call | own: artifact, folder: Path.join(Server.home(), Artifact.name(artifact))}
    database.transaction(call, @schema, &fun.({database, &1}))
  end

  defp query({database, db}, sql, values \\ []), do: database.query(db, sql, values)

  @doc false
  # Runs what handles secrets, so that nothing it raises says one: only what this plugin raised in its own words
  # is passed on, and anything else is said as what was being done. A call that another server ran ends as it does.
  def quietly(what, fun) do
    fun.()
  rescue
    error in RuntimeError -> reraise error, __STACKTRACE__
    error -> raise "#{what} didn't work (#{inspect(error.__struct__)})."
  end
end
