defmodule Vault do
  @moduledoc """
  The Vault: keys, sign-ins and accounts, used without being seen. Each is an item with fields, some of them
  secret, the sites its secrets are sent to, and what may use it: the plugins its person switched on for it, and
  their agents if they said so.

  Someone's items are one encrypted database (`Encryption`, `Database`), in an artifact made for them, so only the
  servers they chose open it. A secret gets here encrypted on its person's device for those servers, and leaves
  only inside a request to one of its item's sites, or to the code of a plugin switched on for it. No tool here
  answers with one, and nothing a tool raises says one: what handles them raises only words of its own
  (`quietly/2`).

  A plugin uses an account through the Vault's tools, which act for the plugin that calls them (its `caller`), so
  no secret reaches its code: `fetch` makes a web request with an item's fields filled in, `sign_in` starts an
  OAuth sign-in whose tokens become an item (`Vault.OAuth`), which `fetch` then keeps fresh, `items` lists what it
  may use, and `stop_using` switches one off for it. The rest of the tools are its person's, and refuse a plugin.

  A plugin trusted with secrets themselves, whose service hands one to a program, calls the functions: `value/3`
  gives one field, `keep/4` keeps secrets the plugin made itself, `remove/2` removes one, and `items/1` lists what
  it may use.
  """
  @behaviour OneCore.Plugin

  # A network has one Vault, where every plugin keeps its keys and outside accounts.
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
    """
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
            "either `account: true`, for items that are accounts, which the Vault lists under Accounts by the plugin's " <>
            "`accounts`, `sign_in` and `sign_out` tools, or `plural`, the heading of their own section (Chrome " <>
            "Profiles). `component` names a Solid component the plugin's client exports (client/package.json's " <>
            "`exports`, as ./<component>), which draws one item, given `item` and `subject`, beside the list; and " <>
            "`create`, a tool of the plugin's that makes one from its `name`."
      }
    ]
  end

  @impl true
  def apps, do: [%{name: "Vault", page: "client/index.html", icon: "client/icon.svg", category: "access", subject: "user"}]

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
                  type: %{type: ["string", "null"], description: "Its type's id, as `types` lists them, if it's of one"},
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
                required: ["name", "type", "fields", "hosts", "plugins", "agents", "sign_in"]
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
                  account: %{type: "boolean", description: "Whether its items are accounts"},
                  plural: %{type: ["string", "null"], description: "Its section's heading"},
                  component: %{type: ["string", "null"], description: "What the plugin's client exports that draws one"},
                  create: %{type: ["string", "null"], description: "The plugin's tool that makes one, from its name"},
                  sign_in: %{type: ["string", "null"], description: "The page the plugin signs an account in on, if it has one"}
                },
                required: ~w(id plugin title icon trusted name prefix account plural component create sign_in)
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
        description: "Deletes an item, or with `field`, one of its fields.",
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            field: %{type: "string", description: "Only this field. The whole item, if left out"}
          },
          required: ["item"]
        },
        outputs: %{
          type: "object",
          properties: %{item: item, field: %{type: ["string", "null"], description: "The field deleted, or null for the item"}},
          required: ["item", "field"]
        }
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
                  sign_in: %{type: ["string", "null"], description: "The service whose sign-in made it, if one did"}
                },
                required: ["name", "sign_in"]
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
    artifact = reached(call)

    quietly("Reading #{field}", fn ->
      transaction(call, artifact, fn db ->
        item = allowed!(db, item, {:plugin, call.who})
        fields(db, item)[field] || raise "#{item["name"]} has no field named #{field}."
      end)
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
    artifact = reached(call)

    quietly("Keeping #{item}", fn ->
      transaction(call, artifact, fn db ->
        stored =
          if item(db, item) do
            allowed!(db, item, {:plugin, call.who})
          else
            created = new(db, item, hosts)
            query(db, "insert into uses (item, plugin) values (?, ?)", [created["id"], call.who])
            created
          end

        for {field, value} <- fields, do: keep(db, stored, field, value, true)
        :ok
      end)
    end)
  end

  @doc """
  Removes the item `item`, for the plugin that calls this, which its person switched on for it or which it made
  with `keep/4`: as when an account it holds is signed out. One that isn't there is nothing to remove.
  """
  def remove(%Call{} = call, item) do
    artifact = reached(call)

    quietly("Removing #{item}", fn ->
      blobs =
        transaction(call, artifact, fn db ->
          if item(db, item) do
            %{"id" => id} = allowed!(db, item, {:plugin, call.who})
            deleted(db, id)
          else
            []
          end
        end)

      if blobs != [], do: Enum.each(blobs, &Artifact.remove(contents!(call), &1))
      :ok
    end)
  end

  @doc """
  Renames the item `item` to `to`, for the plugin that calls this, which it's switched on for. Its fields, its
  contents and whatever it's switched on for go with it.
  """
  def rename(%Call{} = call, item, to) do
    artifact = reached(call)

    quietly("Renaming #{item}", fn ->
      transaction(call, artifact, fn db ->
        %{"id" => id} = stored = allowed!(db, item, {:plugin, call.who})
        if String.trim(to) == "", do: raise("An item has a name.")
        if match?(%{"id" => other} when other != id, item(db, to)), do: raise("There's an item named #{to} already.")
        query(db, "update items set name = ? where id = ?", [to, id])
        touch(db, stored)
        :ok
      end)
    end)
  end

  @doc """
  The fields of the item `item`, by their names, for the plugin that calls this, which it's switched on for: as
  `value/3` gives each.
  """
  def values(%Call{} = call, item) do
    artifact = reached(call)

    quietly("Reading #{item}", fn ->
      transaction(call, artifact, fn db -> fields(db, allowed!(db, item, {:plugin, call.who})) end)
    end)
  end

  @doc """
  Keeps the contents `name` of the item `item`, for the plugin that calls this, which it's switched on for: what's
  too large for a field, like a Chrome profile's hundreds of MB, kept as 1Password keeps a file. They're a blob of the
  Vault's in the bucket, sealed under a key made for that blob alone, which only the item holds.

  `send` is given where they go, `%{"link" => link, "key" => key}`: a short-lived link that writes one blob, and the
  key, base64. It hands them to whatever holds the bytes, which seals them a chunk at a time and sends them, as
  Containers' `upload` does, so they never pass through a tool's result whatever their size, and gives how many bytes
  it sent. The item holds them once it has, and only then do the contents they replace go, so a write that fails
  leaves the last one whole. Gives the size.
  """
  def write_contents(%Call{} = call, item, name, send) when is_function(send, 1) do
    artifact = reached(call)
    {blob, key} = {Base.encode16(:crypto.strong_rand_bytes(16), case: :lower), Base.encode64(:crypto.strong_rand_bytes(32))}

    quietly("Keeping #{name} of #{item}", fn ->
      transaction(call, artifact, &allowed!(&1, item, {:plugin, call.who}))
      contents = contents!(call)
      size = send.(%{"link" => link(contents, blob, :write), "key" => key})

      replaced =
        transaction(call, artifact, fn db ->
          %{"id" => id} = stored = allowed!(db, item, {:plugin, call.who})
          was = query(db, "select blob from contents where item = ? and name = ?", [id, name])

          upsert =
            "insert into contents (item, name, blob, key, size, at) values (?, ?, ?, ?, ?, ?) on conflict (item, name) do update set blob = excluded.blob, key = excluded.key, size = excluded.size, at = excluded.at"

          query(db, upsert, [id, name, blob, key, size, now()])
          touch(db, stored)
          for %{"blob" => old} <- was, do: old
        end)

      for old <- replaced, do: Artifact.remove(contents, old)
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
    artifact = reached(call)

    quietly("Reading #{name} of #{item}", fn ->
      stored =
        transaction(call, artifact, fn db ->
          %{"id" => id} = allowed!(db, item, {:plugin, call.who})
          List.first(query(db, "select blob, key, size from contents where item = ? and name = ?", [id, name]))
        end)

      if stored,
        do: receive.(%{"link" => link(contents!(call), stored["blob"], :read), "key" => stored["key"], "size" => stored["size"]})
    end)
  end

  @doc """
  The items switched on for the plugin that calls this, each with the service whose sign-in made it, if one did:
  `[%{name: "Google a@example.com", sign_in: "Google"}]`, by name. None before anything is.
  """
  def items(%Call{} = call) do
    if Call.reaches?(call, theirs(call), "editor"),
      do: transaction(call, reached(call), &used_by(&1, call.who)),
      else: []
  end

  defp used_by(db, plugin) do
    used =
      "select items.name, items.oauth from items join uses on uses.item = items.id where uses.plugin = ? order by items.name collate nocase"

    for item <- query(db, used, [plugin]),
        do: %{name: item["name"], sign_in: item["oauth"] && JSON.decode!(item["oauth"])["name"]}
  end

  # The tools a plugin's code calls, which act for it. The rest are its person's.
  @plugins ~w(fetch items sign_in stop_using)

  @impl true
  def hooks, do: ["renamed"]

  # The items switched on for a renamed plugin are switched on for it by its new name, in the Vault of whoever it's
  # renamed for, if they have one.
  @impl true
  def hook("renamed", %{"was" => was, "is" => is}, %Call{subject: "users/" <> _} = call) do
    if Artifact.list(items_artifact(call), "") != [] do
      transaction(call, own(call), fn db ->
        query(db, "update or ignore uses set plugin = ? where plugin = ?", [is, was])
        query(db, "delete from uses where plugin = ?", [was])
      end)
    end
  end

  def hook("renamed", _about, _call), do: :ok

  @impl true
  def call(tool, _inputs, %Call{caller: caller}) when caller != nil and tool not in @plugins,
    do: raise("vault_#{tool} is its person's, not a plugin's. A plugin calls #{Enum.join(@plugins, ", ")}.")

  def call(tool, _inputs, %Call{caller: nil}) when tool in ~w(items sign_in stop_using),
    do: raise("vault_#{tool} is for a plugin's code, and acts for the plugin that calls it.")

  def call("items", _inputs, call), do: %{items: transaction(call, own(call), &used_by(&1, call.caller))}

  def call("stop_using", %{"item" => name}, call) do
    transaction(call, own(call), fn db ->
      query(db, "delete from uses where item = ? and plugin = ?", [item!(db, name)["id"], call.caller])
    end)

    %{item: name}
  end

  def call("sign_in", %{"service" => given} = inputs, call) do
    address = address()
    artifact = own(call)

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

      client =
        transaction(call, artifact, fn db ->
          item = item!(db, service.client)
          client = fields(db, item)

          client["client_id"] ||
            raise "#{service.client} has no client_id. Keep the one #{service.name} gave you there."

          # Its client_secret goes to the token address, as any secret goes only to one of its item's sites.
          if client["client_secret"] && not OAuth.to?(service.token, JSON.decode!(item["hosts"])),
            do:
              raise(
                "#{service.client}'s secrets go only to its sites, and #{service.name}'s token address isn't one. Its person adds it, in the Vault."
              )

          stored = JSON.encode!(Map.take(service, [:name, :token, :hosts, :client]))
          query(db, "delete from sign_ins where at < ?", [now() - @sign_in_waits])

          query(db, "insert into sign_ins (state, verifier, service, plugin, item, at) values (?, ?, ?, ?, ?, ?)", [
            state,
            verifier,
            stored,
            call.caller,
            inputs["item"] || service.name,
            now()
          ])

          client
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
    names = names(call)
    types = types(call)

    use_it =
      Enum.sort(
        for {name, version} <- Plugins.running(call.subject),
            "vault" in version.uses or Enum.any?(version.permissions, &(&1 == "vault" or &1 =~ ~r/^vault_/)),
            do: name
      )

    transaction(call, own(call), fn db ->
      items =
        for item <- query(db, "select * from items order by name collate nocase") do
          fields =
            for field <-
                  query(
                    db,
                    "select name, secret, case secret when 0 then value end as shown from fields where item = ? order by rowid",
                    [item["id"]]
                  ) do
              if field["secret"] == 1,
                do: %{name: field["name"], secret: true},
                else: %{name: field["name"], secret: false, value: field["shown"]}
            end

          plugins =
            for %{"plugin" => plugin} <- query(db, "select plugin from uses where item = ?", [item["id"]]),
                do: named(names, plugin)

          # An item is of a plugin's type when it's switched on for that plugin and named as the type says.
          type = Enum.find(types, &(&1.plugin in plugins and String.starts_with?(item["name"], &1.prefix)))

          %{
            name: item["name"],
            type: type && type.id,
            fields: fields,
            hosts: JSON.decode!(item["hosts"]),
            plugins: Enum.sort(plugins),
            agents: item["agents"] == 1,
            sign_in: item["oauth"] != nil,
            changed: item["changed"]
          }
        end

      waiting =
        "select id, title, fields, hosts, plugin, why from requests where state = 'waiting' and at > ? order by at"

      requests =
        for request <- query(db, waiting, [now() - @request_waits]) do
          %{
            id: request["id"],
            title: request["title"],
            fields: JSON.decode!(request["fields"]),
            hosts: JSON.decode!(request["hosts"]),
            plugin: request["plugin"] && named(names, request["plugin"]),
            why: request["why"]
          }
        end

      %{items: items, requests: requests, plugins: use_it, types: types}
    end)
  end

  def call("set", %{"item" => name, "field" => field, "value" => value} = inputs, call) do
    artifact = own(call)

    quietly("Keeping #{field}", fn ->
      value = encryption(call).secret(call, value)

      transaction(call, artifact, fn db ->
        item = item(db, name) || new(db, name, Map.get(inputs, "hosts", []))

        if hosts = inputs["hosts"],
          do: query(db, "update items set hosts = ? where id = ?", [JSON.encode!(hosts!(hosts)), item["id"]])

        keep(db, item, field, value, Map.get(inputs, "secret", true))
        %{item: name, field: field}
      end)
    end)
  end

  def call("remove", %{"item" => name} = inputs, call) do
    removed =
      transaction(call, own(call), fn db ->
        item = item!(db, name)

        case inputs["field"] do
          nil ->
            plugins =
              for %{"plugin" => plugin} <- query(db, "select plugin from uses where item = ?", [item["id"]]), do: plugin

            blobs = deleted(db, item["id"])
            {nil, Enum.reject(plugins, &used?(db, &1)), blobs}

          field ->
            removed = query(db, "delete from fields where item = ? and name = ? returning name", [item["id"], field])
            if removed == [], do: raise("#{name} has no field named #{field}.")
            touch(db, item)
            {field, [], []}
        end
      end)

    {field, unused, blobs} = removed
    for blob <- blobs, do: Artifact.remove(Call.make(mine!(call), "contents"), blob)
    for plugin <- unused, do: share(call, "unshare", Artifact.name(items_artifact(call)), plugin)
    %{item: name, field: field}
  end

  # A plugin switched on for an item is an editor of the Vault's artifact too, which is what lets its code reach it
  # at all, and goes when no item is switched on for it.
  def call("use", %{"item" => name, "plugin" => plugin} = inputs, call) do
    artifact = own(call)

    who =
      Plugins.plugin(call.subject, plugin) || Plugins.plugin("network", plugin) ||
        raise "There's no plugin named #{plugin}."

    if Map.get(inputs, "on", true) do
      transaction(call, artifact, fn db ->
        query(db, "insert or ignore into uses (item, plugin) values (?, ?)", [item!(db, name)["id"], who])
      end)

      share(call, "share", Artifact.name(artifact), who)
      %{item: name, plugin: plugin, on: true}
    else
      unused =
        transaction(call, artifact, fn db ->
          query(db, "delete from uses where item = ? and plugin = ?", [item!(db, name)["id"], who])
          not used?(db, who)
        end)

      if unused, do: share(call, "unshare", Artifact.name(artifact), who)
      %{item: name, plugin: plugin, on: false}
    end
  end

  def call("agents", %{"item" => name} = inputs, call) do
    on = Map.get(inputs, "on", true)

    transaction(call, own(call), fn db ->
      query(db, "update items set agents = ? where id = ?", [on, item!(db, name)["id"]])
    end)

    %{item: name, on: on}
  end

  def call("request", %{"title" => title, "fields" => fields} = inputs, call) do
    artifact = own(call)

    plugin =
      if name = inputs["plugin"],
        do:
          Plugins.plugin(call.subject, name) || Plugins.plugin("network", name) ||
            raise("There's no plugin named #{name}.")

    if fields == %{}, do: raise("A request names the fields it needs.")
    fields = for {name, about} <- fields, do: %{name: field!(name), secret: Map.get(about, "secret", true)}
    id = random(9)

    transaction(call, artifact, fn db ->
      query(
        db,
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
    transaction(call, own(call), fn db ->
      case query(db, "select title, state, at from requests where id = ?", [id]) do
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
    artifact = own(call)

    quietly("Answering the request", fn ->
      values = if given = inputs["values"], do: JSON.decode!(encryption(call).secret(call, given))

      result =
        transaction(call, artifact, fn db ->
          request =
            case query(db, "select * from requests where id = ? and state = 'waiting' and at > ?", [
                   id,
                   now() - @request_waits
                 ]) do
              [request] -> request
              [] -> raise "That request isn't waiting any more."
            end

          cond do
            inputs["decline"] == true ->
              query(db, "update requests set state = 'declined' where id = ?", [id])
              {%{state: "declined"}, nil}

            name = inputs["item"] ->
              {allow(db, request, item!(db, name)), request["plugin"]}

            is_map(values) ->
              wanted = JSON.decode!(request["fields"])
              missing = for %{"name" => name} <- wanted, values[name] in [nil, ""], do: name
              if missing != [], do: raise("It needs #{Enum.join(missing, ", ")} too.")
              item = item(db, request["title"]) || new(db, request["title"], JSON.decode!(request["hosts"]))
              for %{"name" => name, "secret" => secret} <- wanted, do: keep(db, item, name, values[name], secret)
              {allow(db, request, item), request["plugin"]}

            true ->
              raise "Answer with one of your items, with values, or by declining."
          end
        end)

      {message, plugin} = result
      if plugin, do: share(call, "share", Artifact.name(artifact), plugin)
      message
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
      artifact = own(call)

      quietly("Finishing the sign-in", fn ->
        sign_in =
          transaction(call, artifact, fn db ->
            case query(db, "select * from sign_ins where state = ? and at > ?", [state, now() - @sign_in_waits]) do
              [sign_in] -> Map.put(sign_in, "client", fields(db, item!(db, JSON.decode!(sign_in["service"])["client"])))
              [] -> raise "This sign-in isn't waiting any more. Start it again."
            end
          end)

        service = JSON.decode!(sign_in["service"])
        if why = inputs["error"], do: raise("#{service["name"]} didn't sign you in: #{why}.")
        code = inputs["code"] || raise "#{service["name"]} sent no code back."
        tokens = OAuth.exchange(service, sign_in["client"], code, sign_in["verifier"], address() <> "/vault/callback")
        name = signed_in(sign_in["item"], service, tokens)

        transaction(call, artifact, fn db ->
          if query(db, "delete from sign_ins where state = ? returning state", [state]) != [] do
            item = item(db, name) || new(db, name, service["hosts"])
            keep_tokens(db, item, service, tokens)
            query(db, "insert or ignore into uses (item, plugin) values (?, ?)", [item["id"], sign_in["plugin"]])
          end
        end)

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
  defp fetched(call, artifact, name, request, by) do
    quietly("The request", fn ->
      read = fn ->
        transaction(call, artifact, fn db ->
          item = allowed!(db, name, by)

          {item, fields(db, item),
           for(
             %{"name" => field} <- query(db, "select name from fields where item = ? and secret = 1", [item["id"]]),
             do: field
           )}
        end)
      end

      {item, fields, secret} = read.()
      oauth = item["oauth"] && JSON.decode!(item["oauth"])
      fields = if oauth && oauth["expires"] <= now() + 60, do: renewed(call, artifact, item, fields), else: fields
      response = OAuth.request(request, fields, secret, JSON.decode!(item["hosts"]))

      if oauth && response.status == 401,
        do: OAuth.request(request, renewed(call, artifact, item, fields), secret, JSON.decode!(item["hosts"])),
        else: response
    end)
  end

  # An item's fields with its tokens renewed, unless another call renewed them meanwhile: one renewal at a time, on
  # the server that writes the Vault.
  defp renewed(call, artifact, item, had) do
    :global.trans({{__MODULE__, Artifact.name(artifact), item["id"]}, self()}, fn ->
      {oauth, fields, client} =
        transaction(call, artifact, fn db ->
          [%{"oauth" => oauth}] = query(db, "select oauth from items where id = ?", [item["id"]])
          oauth = JSON.decode!(oauth)
          {oauth, fields(db, item), fields(db, item!(db, oauth["client"]))}
        end)

      if fields["access_token"] != had["access_token"] do
        fields
      else
        tokens =
          OAuth.renew(
            oauth,
            client,
            fields["refresh_token"] || raise("#{item["name"]} has no way to renew its sign-in. Sign in again.")
          )

        transaction(call, artifact, fn db ->
          keep_tokens(db, item, oauth, tokens)
          fields(db, item)
        end)
      end
    end)
  end

  defp keep_tokens(db, item, service, tokens) do
    for field <- ~w(access_token refresh_token), value = tokens[field], do: keep(db, item, field, value, true)

    oauth = %{
      name: service["name"],
      token: service["token"],
      client: service["client"],
      expires: now() + (tokens["expires_in"] || 3600)
    }

    query(db, "update items set oauth = ? where id = ?", [JSON.encode!(oauth), item["id"]])
  end

  defp allow(db, request, item) do
    if plugin = request["plugin"],
      do: query(db, "insert or ignore into uses (item, plugin) values (?, ?)", [item["id"], plugin]),
      else: query(db, "update items set agents = 1 where id = ?", [item["id"]])

    query(db, "update requests set state = 'set' where id = ?", [request["id"]])
    %{state: "set"}
  end

  # The item named `name`, if `by` may use it: a plugin switched on for it, or agents its person let.
  defp allowed!(db, name, by) do
    item = item!(db, name)

    case by do
      :agents ->
        if item["agents"] != 1,
          do: raise("#{item["name"]} isn't one agents may use. Ask its person, with vault_request.")

      {:plugin, plugin} ->
        if query(db, "select 1 from uses where item = ? and plugin = ?", [item["id"], plugin]) == [],
          do:
            raise(
              "#{item["name"]} isn't switched on for #{Path.basename(plugin)}. Its person switches it on, in the Vault."
            )
    end

    item
  end

  defp item(db, name), do: List.first(query(db, "select * from items where name = ? collate nocase", [name]))
  defp item!(db, name), do: item(db, name) || raise("There's no item named #{name} in the Vault.")

  defp new(db, name, hosts) do
    if String.trim(name) == "", do: raise("An item has a name.")

    [item] =
      query(db, "insert into items (id, name, hosts, changed) values (?, ?, ?, ?) returning *", [
        random(9),
        name,
        JSON.encode!(hosts!(hosts)),
        now()
      ])

    item
  end

  defp keep(db, item, field, value, secret) do
    sql =
      "insert into fields (item, name, value, secret) values (?, ?, ?, ?) on conflict (item, name) do update set value = excluded.value, secret = excluded.secret"

    query(db, sql, [item["id"], field!(field), value, secret == true])
    touch(db, item)
  end

  defp touch(db, item), do: query(db, "update items set changed = ? where id = ?", [now(), item["id"]])

  defp fields(db, item),
    do: Map.new(query(db, "select name, value from fields where item = ?", [item["id"]]), &{&1["name"], &1["value"]})

  defp used?(db, plugin), do: query(db, "select 1 from uses where plugin = ? limit 1", [plugin]) != []

  # Deletes an item and what's kept of it, and gives the blobs its contents were, for whoever deletes it to remove.
  defp deleted(db, id) do
    blobs = for %{"blob" => blob} <- query(db, "select blob from contents where item = ?", [id]), do: blob
    for table <- ~w(fields uses contents), do: query(db, "delete from #{table} where item = ?", [id])
    query(db, "delete from items where id = ?", [id])
    blobs
  end

  # A short-lived link to one blob of an item's contents, as what's handed on takes it: JSON's keys.
  defp link(contents, blob, mode), do: contents |> Artifact.link(blob, mode) |> JSON.encode!() |> JSON.decode!()

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
  # plugin's name, title and icon, whether its plugin is trusted, which decides where its component is drawn, and the
  # page its plugin signs an account in on, if it has one. A type that doesn't say what one must is left out.
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
        account: type["account"] == true,
        plural: type["plural"],
        component: type["component"],
        create: type["create"],
        sign_in: Enum.find_value(version.tools, &(&1.name == "sign_in" and &1[:ui] && "#{plugin}/#{&1[:ui]}"))
      }
    end
  end

  defp type?(%{"name" => name, "prefix" => prefix} = type) when is_binary(name) and is_binary(prefix) and prefix != "",
    do: type["account"] == true or is_binary(type["plural"])

  defp type?(_type), do: false

  # The plugins' names as whoever this runs for finds them, by their artifacts.
  defp names(call), do: Map.new(Plugins.running(call.subject), fn {name, version} -> {version.artifact, name} end)
  defp named(names, plugin), do: Map.get(names, plugin, Path.basename(plugin))

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

  # The artifact that the items of whoever a call of the Vault's own tools is for are in.
  defp items_artifact(call), do: Call.make(mine!(call), "items")

  # Their Vault, opened: on a server they chose, which this call ends up on.
  defp own(call), do: encryption(call).encrypted(call, items_artifact(call))

  # The same artifact by its name, for a plugin that uses the Vault, which finds it added as `vault`.
  defp theirs(call), do: "#{mine!(call).subject}/made/vault/items"

  # It reaches the Vault as an editor, which a tuple says once anything of the Vault's is switched on for it.
  defp reached(call), do: encryption(call).encrypted(call, Call.reach(call, theirs(call), "editor"))

  # Where the contents of someone's items are, each a blob sealed under a key its item holds: an artifact made for
  # them, which a plugin reaches as an editor as it does their Vault. The bucket holds only what's sealed.
  defp contents(call), do: "#{mine!(call).subject}/made/vault/contents"

  # That artifact, reached by a plugin switched on for one of their items, which a tuple then lets it.
  defp contents!(call) do
    share(call, "share", contents(call), call.who)
    Call.reach(call, contents(call), "editor")
  end

  defp encryption(call), do: Call.plugin(call, "encryption")

  # A transaction on someone's Vault. The database is theirs, so this server's copy of it is kept by its name.
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
