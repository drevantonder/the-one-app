defmodule Vault do
  @moduledoc """
  The Vault: the one place for everything someone has given access to, used without being seen: keys, passwords,
  sign-ins, accounts and Chrome profiles. Each is an item (`Vault.Items`), kept as an artifact of its own encrypted
  with their Vault's keys (`Vault.Store`), so only the servers they chose open it, and deleted, restored and purged by
  soft delete as anything an app made is. Who may use each is one list on it (`Vault.Access`); what's asked of its
  person waits in `Vault.Requests`; a sign-in to an outside service is `Vault.SignIns`; and a secret leaves only inside
  a web request to one of its item's websites (`Vault.Fetch`), or to the code of an app with access. No tool here
  answers with a secret, and nothing a tool raises says one (`Vault.Store.quietly/2`).

  This module is the app: its tools, which act for its person, or for the app that calls them (its `caller`), and
  the functions an app trusted with secrets calls (`value/3`, `keep/4`, `state/5` and the rest).
  """
  @behaviour OneCore.App

  # A network has one Vault, where every app keeps its keys and outside accounts.
  @impl true
  def icon, do: "client/icon.svg"

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are people's keys and sign-ins.
  @impl true
  def sensitive, do: true

  alias OneCore.{Artifact, Call}
  alias Vault.{Access, Fetch, Items, Requests, SignIns, Store}

  # How ready someone's Vault is, which `security` says and its screen shows, worked out each time from their key and
  # the servers they chose, never kept. Each state is the first thing missing, in the order they're done:
  #
  #   not_set_up --Set Up (a device makes the key)--> no_server --a server is chosen--> device_lacks_key
  #   device_lacks_key --this device is approved--> ready
  #
  # and back as they're undone: the last server switched off is no_server again, this device's key revoked is
  # device_lacks_key. A device that lacks the key still adds and uses items (a secret is encrypted for the servers,
  # not with the key); only choosing servers and approving devices need it.
  @readiness ~w(not_set_up no_server device_lacks_key ready)

  # The most of an answer's body that a tool answers with, in bytes.
  @most 256 * 1024

  @impl true
  def requires do
    %{
      "core" => ">= 0.32.167",
      "encryption" => "to keep your secrets from whoever runs the bucket",
      "database" => "to keep your items",
      "notifications" => "to tell you when an agent saves something to your Vault"
    }
  end

  # The kinds of item other apps keep here, which they register by answering `vault.types` (SKILL.md).
  @impl true
  def offers do
    [
      %{
        name: "types",
        kind: :collect,
        # An untrusted app's part is only data, drawn by its own page in a frame.
        untrusted: true,
        description:
          "The kinds of item an app keeps in its people's Vaults, which the Vault lists and opens as the app says. " <>
            "It's about nothing. An app with item types answers with its list of them, each a map: `name`, one item " <>
            "of it as a person reads it (Chrome profile); `prefix`, what its items' names start with, for the items " <>
            "switched on for the app (\"Chrome profile: \"), what follows being the item as a person reads it; and " <>
            "`section`, the heading it's listed under (Chrome Profiles, or Accounts, which several apps' types share). " <>
            "`component` names a Solid component the app's client exports (client/package.json's `exports`, as " <>
            "./<component>), which draws one item beside the list, given `item` and `subject`; and `create` is how one " <>
            "is added, an action as `state/5` takes one: `%{tool: \"create\", name: true}` asks its person for a name " <>
            "first."
      }
    ]
  end

  # Requests that waited too long are kept expired, and what ended long ago is let go (`Vault.Requests`).
  @impl true
  def jobs, do: [%{tool: "expire", every: 3600}]

  @impl true
  def hooks, do: ["settings.sections"]

  # The Vault is a section of Settings: your accounts, passwords and keys, and what apps keep here.
  @impl true
  def hook("settings.sections", _about, _call),
    do: [%{in: "settings", name: "Vault", component: "accounts", icon: "client/icon.svg", group: "network", order: 3}]

  @impl true
  def tools do
    item = %{type: "string", description: "The item's name"}

    hosts = %{
      type: "array",
      items: %{type: "string"},
      description: "The sites its secrets are sent to, like api.example.com"
    }

    secret = %{type: "string", format: "secret", unlocked_on: "security"}

    named_device = %{
      type: "object",
      properties: %{device: %{type: "string"}, name: %{type: "string"}},
      required: ["device", "name"]
    }

    [
      %{
        name: "list",
        read_only: true,
        description: """
        Lists the Vault's items: each one's fields (a secret one never with its value), the sites its secrets are
        sent to, the apps switched on for it and whether agents may use it. With them, the requests still
        waiting for their person, and the apps that use the Vault.
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
                  kind: %{type: "string", enum: Items.kinds(), description: "password, account (a sign-in, or what an app keeps for itself) or app_item (of an app's type)"},
                  type: %{type: ["string", "null"], description: "Its type's id, as `types` lists them, if it's an app_item"},
                  status: %{
                    type: ["object", "null"],
                    description: "Null when it's ready; else busy with a message, or needs_action with why and an action, as its app says"
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
                  access: %{type: "array", items: %{type: "string"}, description: "Who may use it: agents (with fetch), and the apps switched on for it"},
                  sign_in: %{type: "boolean", description: "Whether a sign-in made it"},
                  added: %{
                    type: ["object", "null"],
                    description: "If a save from a browser made it: `by`, the device of the agent that saved it, `from`, the site, and `at`, when"
                  },
                  changed: %{type: ["integer", "null"], description: "When it last changed, in milliseconds since 1970"}
                },
                required: ["name", "artifact", "kind", "type", "status", "fields", "hosts", "access", "sign_in"]
              }
            },
            requests: %{
              type: "array",
              description: "The requests kept: those waiting for their person, and those that ended in the last month",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string"},
                  title: %{type: "string"},
                  state: %{type: "string", enum: Requests.states()},
                  fields: %{type: "array", items: %{type: "object"}},
                  hosts: %{type: "array", items: %{type: "string"}},
                  app: %{
                    type: ["object", "null"],
                    description: "The app it's for, `name` and `title`, or null for agents",
                    properties: %{name: %{type: "string"}, title: %{type: "string"}}
                  },
                  by: %{type: ["string", "null"], description: "The agent that asked, by its name, where one did"},
                  why: %{type: ["string", "null"]}
                },
                required: ["id", "title", "state", "fields", "hosts", "app", "by", "why"]
              }
            },
            apps: %{
              type: "array",
              description: "The apps that use the Vault: `name`, which its tools start with, `title`, as a person reads it, and `icon`",
              items: %{
                type: "object",
                properties: %{name: %{type: "string"}, title: %{type: "string"}, icon: %{type: ["string", "null"], description: "ui://icons/<sha256>/icon.<ext>"}},
                required: ["name", "title"]
              }
            },
            types: %{
              type: "array",
              description: "The kinds of item apps keep here (`vault.types`)",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string"},
                  app: %{type: "string", description: "The app's name, which its tools start with"},
                  title: %{type: ["string", "null"], description: "The app's name, as a person reads it"},
                  icon: %{type: ["string", "null"], description: "The app's icon, a data: address"},
                  trusted: %{type: "boolean", description: "Whether its component is drawn in the Vault's page, or in a frame of its own"},
                  name: %{type: "string", description: "One item of it, as a person reads it"},
                  prefix: %{type: "string", description: "What its items' names start with"},
                  section: %{type: "string", description: "The heading it's listed under"},
                  component: %{type: ["string", "null"], description: "What the app's client exports that draws one"},
                  create: %{type: ["object", "null"], description: "How one is added: a tool of the app's, or a page of its"}
                },
                required: ~w(id app title icon trusted name prefix section component create)
              }
            }
          },
          required: ["items", "requests", "apps", "types"]
        }
      },
      %{
        name: "set",
        description: """
        Keeps a key, token or password someone pasted (or any field) in an item, making the item if there's none by
        that name. Nobody reads a secret back, not you either. When the person has the value, use `request`
        instead: never ask them to paste it into the chat. A secret is kept only as a plain tool on a device, which
        encrypts it: in code mode, or over the web, use `request` for one they pasted too. `hosts` are added to a
        new item's; adding sites to one that's there is its person's, in the Vault, or by allowing a `request`.
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
        name: "save_from_browser",
        description: """
        Saves a secret from a browser of yours (Chrome's or Chrome Profiles' `session`) into the Vault without you
        seeing it, as a password manager's Save does: what elements on the page hold, or what's in a file the browser
        downloaded, the whole of it or a value in its JSON. Into a new item, or one a save from a browser made. The
        person is notified, and sees who added it from where, with Undo. Answers each field's length and the first 8
        hex of its SHA-256, never its value. The download is deleted from the browser.
        """,
        inputs: %{
          type: "object",
          properties: %{
            session: %{type: "string", description: "The browser, by the session it was opened with"},
            item: item,
            fields: %{
              type: "object",
              description:
                "What each field is taken from, by its name: {\"client_secret\": {\"target\": \"e12\"}}, or with `download`, {\"client_id\": {\"json\": \"web.client_id\", \"secret\": false}, \"client_secret\": {\"json\": \"web.client_secret\"}}",
              minProperties: 1,
              additionalProperties: %{
                type: "object",
                properties: %{
                  target: %{type: "string", description: "The element that holds it: its ref from the snapshot, like e12, or a unique selector"},
                  json: %{type: "string", description: "With `download`, where it is in the file's JSON, like web.client_secret. The whole file if left out"},
                  secret: %{type: "boolean", description: "Never shown again. True if left out: say false for a client_id"}
                }
              }
            },
            download: %{type: "string", description: "A file the browser downloaded, by its name as Chrome saved it, to take the fields from"},
            hosts: hosts,
            by: %{type: "string", format: "device-name", description: "Your device fills this in"}
          },
          required: ["session", "item", "fields"]
        },
        outputs: %{
          type: "object",
          properties: %{
            item: item,
            site: %{type: "string", description: "The site it was saved from"},
            fields: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  name: %{type: "string"},
                  secret: %{type: "boolean"},
                  length: %{type: "integer", description: "How many characters it is"},
                  sha256: %{type: "string", description: "The first 8 hex of its SHA-256, to check it against"}
                },
                required: ["name", "secret", "length", "sha256"]
              }
            }
          },
          required: ["item", "site", "fields"]
        }
      },
      %{
        name: "request",
        description: """
        Asks the person for a key, token, password or sign-in, for an app or for you to use, instead of handling
        its value yourself. Answers with a link to give them: they pick one of their items or type a new one on
        their own device, and you never see it. Tell them; `list` says how it went once they say they've answered.
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
            app: %{type: "string", description: "The app that will use it. Left out, it's for you, with `fetch`"},
            why: %{type: "string", description: "One line the person sees: what it's for, and where they find it"}
          },
          required: ["title", "fields"]
        },
        outputs: %{
          type: "object",
          properties: %{
            id: %{type: "string", description: "The request's id, which `list` gives its state by"},
            link: %{type: ["string", "null"], description: "The link to give the person, where the network has an address"},
            message: %{type: "string", description: "What to tell them"}
          },
          required: ["id", "link", "message"]
        }
      },
      %{
        name: "answer",
        description: """
        Answers a request, as its person does on their own device: with one of their items, with new values, or by
        declining. Allowing it switches the item on for whoever asked, and lets its secrets go to `hosts` too. Only
        its person answers, in the Vault on their device, never an agent.
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
        outputs: %{type: "object", properties: %{state: %{type: "string", enum: ~w(allowed declined)}}, required: ["state"]}
      },
      %{
        name: "fetch",
        description: """
        Makes a web request with an item's fields filled in, without seeing them: write {{<field>}} where a value
        goes, in the url, a header or the body. Only to one of the item's sites, and only an item its person lets
        agents use, or, called by an app, one switched on for that app. Answers status, headers and body, with
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
            body_base64: %{type: "string", description: "Or a body that isn't text, in base64, as it's sent"},
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
        For an app's code: the items switched on for the app that calls this, each with the service whose
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
                  state: %{type: ["object", "null"], description: "Null when it's ready, else what `state` last said of it: busy or needs_action"}
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
        For an app's code: starts an OAuth sign-in to an outside service for the app that calls this, and
        answers the link its person opens in their browser. The tokens become the item `item`, switched on for the
        app. `client` is the item holding the client_id the service gave, and its client_secret, which goes only
        to its sites, so the service's token address is one of them. `hosts` are the token address's site, names
        under it, or the client's sites, and `item` a new one or one this sign-in made before.
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
            },
            apps: %{
              type: "array",
              items: %{type: "string"},
              description: "What it's for, as people know them, which the page they come back to names: Gmail, Google Calendar"
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
        name: "state",
        description: """
        For an app's code: says which of three states an item switched on for the app that calls this is in, for
        its person to see in the Vault: ready, which shows nothing; busy, with a `message` (Saving…); or
        needs_action, with `message`, why, and `action`, the one thing that fixes it: a tool of the app's, called
        as its person, or a page of its client's.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            state: %{type: "string", enum: ~w(ready busy needs_action)},
            message: %{type: "string", description: "What's under way, or why its person has to act"},
            action: %{
              type: "object",
              description: "For needs_action: {label, tool, inputs} or {label, page}",
              properties: %{
                label: %{type: "string", description: "The button's words, like Sign In"},
                tool: %{type: "string", description: "A tool of the app's, like sign_in"},
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
              description: "The servers' keys, as `security` lists them"
            },
            keys: %{
              type: "string",
              format: "artifact-keys",
              unlocked_on: "security",
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
        name: "names",
        read_only: true,
        description:
          "What items are called, by their artifacts, deleted ones too: an item's artifact is named by an id, as its name is encrypted. Recently Deleted asks it",
        inputs: %{
          type: "object",
          properties: %{artifacts: %{type: "array", items: %{type: "string"}, description: "Items' artifacts, as `list` gives them"}},
          required: ["artifacts"]
        },
        outputs: %{
          type: "object",
          properties: %{
            names: %{
              type: "array",
              description: "Each of them that's an item of this Vault",
              items: %{
                type: "object",
                properties: %{
                  artifact: %{type: "string"},
                  name: %{type: "string"},
                  kind: %{type: ["string", "null"], description: "What its app calls it, as a person reads it: Chrome profile"}
                },
                required: ["artifact", "name"]
              }
            }
          },
          required: ["names"]
        }
      },
      %{
        name: "access",
        description: """
        Gives an app or your agents access to an item, or with `on` false takes it away. An app with access uses the
        item through the Vault's tools without seeing it, or, one with a service, is handed the secret itself, which
        trusts its code with it. Giving access is its person's alone, in the Vault on their device: an agent asks with
        `request`, which they allow there, and is refused here. Taking it away is anyone's. Called by an app's code, it
        takes the app's own access away, as signing out does, and only that.
        """,
        inputs: %{
          type: "object",
          properties: %{
            item: item,
            who: %{type: "string", description: "An app, by its name, like gmail; or agents"},
            on: %{type: "boolean", description: "False takes it away"}
          },
          required: ["item", "who"]
        },
        outputs: %{
          type: "object",
          properties: %{item: item, who: %{type: "string"}, on: %{type: "boolean"}},
          required: ["item", "who", "on"]
        }
      },
      %{
        name: "remove_field",
        description:
          "Removes a field from an item. Deleting a whole item is soft delete's, as for anything an app made: `soft-delete_delete` with the item's artifact, as `list` gives it, so it can be restored for 30 days.",
        inputs: %{
          type: "object",
          properties: %{item: item, field: %{type: "string", description: "The field to remove"}},
          required: ["item", "field"]
        },
        outputs: %{type: "object", properties: %{item: item, field: %{type: "string"}}, required: ["item", "field"]}
      },
      %{
        name: "expire",
        description: "Marks requests that waited a week as expired, and lets go of those that ended a month ago. The clock calls it each hour",
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{expired: %{type: "integer"}}, required: ["expired"]}
      },
      %{
        name: "security",
        read_only: true,
        description: """
        Says everything about the key that keeps your Vault, in one answer: how ready it is, the devices that hold
        the key and those asking to, and which servers open the Vault and which may be chosen to. `readiness` is
        not_set_up (there's no key yet), no_server (no server opens it, so it keeps nothing), device_lacks_key (it
        works, but this device can't choose servers or approve devices) or ready.
        """,
        inputs: %{
          type: "object",
          properties: %{device: %{type: "string", format: "device", description: "Your device fills this in"}}
        },
        outputs: %{
          type: "object",
          properties: %{
            readiness: %{type: "string", enum: @readiness},
            owner: %{type: "string", description: "Whose Vault: a user, a group or the network"},
            holds: %{type: "boolean", description: "Whether this device holds the key"},
            device: %{type: ["string", "null"]},
            devices: %{type: "array", items: named_device, description: "The devices that hold the key"},
            asking: %{type: "array", items: named_device, description: "The devices asking to"},
            artifact: %{type: "string"},
            epoch: %{type: "integer"},
            servers: %{type: "array", items: %{type: "string"}, description: "The servers that open it"},
            signature: %{type: "string", description: "Its owner's signature of which servers open it, by the key `signer`"},
            signer: %{type: "string", description: "The public key its owner signs with, which their devices derive from their key"},
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
          required: ~w(readiness owner holds device devices asking artifact epoch servers may)
        }
      },
      %{
        name: "oauth_callback",
        by_link: true,
        description:
          "Finishes a sign-in to an outside service, or with `again`, starts it again. The network's edge calls this when the service sends its person back, and their page answers how it ended, and where it began.",
        inputs: %{
          type: "object",
          properties: %{
            state: %{type: "string"},
            code: %{type: "string"},
            error: %{type: "string"},
            again: %{type: "boolean", description: "Starts it again, once it ended without signing in"},
            open: %{type: "string", description: "Its link opened, in the browser this random value is kept in"},
            browser: %{type: "string", description: "The random value the browser it was opened in keeps"}
          },
          required: ["state"]
        },
        outputs: %{
          type: "object",
          properties: %{
            service: %{type: "string"},
            status: %{type: "string", enum: SignIns.states()},
            began: %{
              type: "object",
              description: "Where: `{screen: <app>}`, `{agent: <its name>}` or `{cli: true}`",
              properties: %{screen: %{type: "string"}, agent: %{type: "string"}, cli: %{type: "boolean"}}
            },
            apps: %{type: "array", items: %{type: "string"}, description: "What it's for"},
            item: %{type: "string", description: "What it's kept as in the Vault, once it's ok"},
            account: %{type: "string", description: "Who signed in, where the service says their email"},
            problem: %{type: "string", description: "Why it failed"},
            url: %{type: "string", description: "Where it's started again, with `again`, or where its link goes, with `open`"}
          },
          required: ["service", "status", "began", "apps"]
        }
      }
    ]
  end

  @doc """
  One field of an item, for the app that calls this, which has access to it: to hand to a program a service brings.
  What's given is the secret itself, so it goes nowhere a person or an agent reads.
  """
  def value(%Call{} = call, item, field) do
    db = Store.reached(call)

    Store.quietly("Reading #{field}", fn ->
      found = Access.may!(Items.find!(call, db, item), {:app, call.app_artifact})
      Items.fields(found)[field] || raise "#{found["name"]} has no field named #{field}."
    end)
  end

  @doc """
  The fields of the item `item`, by their names, for the app that calls this, which has access to it: as `value/3`
  gives each.
  """
  def values(%Call{} = call, item) do
    db = Store.reached(call)
    Store.quietly("Reading #{item}", fn -> Items.fields(Access.may!(Items.find!(call, db, item), {:app, call.app_artifact})) end)
  end

  @doc """
  Keeps `fields`, by their names, in the item `item`, for the app that calls this: secrets it made itself or an
  outside service handed it, such as the key a session it holds is encrypted with. Each is secret. An item there's
  none of is made, of the app's type where its name is of one (`vault.types`), else an account of the app's, with
  access for it and `hosts` its websites; one that's there is changed only if the app has access to it.
  """
  def keep(%Call{} = call, item, fields, hosts \\ []) when is_map(fields) do
    Access.reach(call)
    db = Store.reached(call)

    Store.quietly("Keeping #{item}", fn ->
      Items.write!(call, db, item, Items.kept_by(call, call.app_artifact, item, hosts), {:app, call.app_artifact}, fn record ->
        Enum.reduce(fields, record, fn {name, value}, record -> Items.put_field(record, name, value, true) end)
      end)

      :ok
    end)
  end

  @doc """
  Deletes the item `item`, for the app that calls this, which has access to it, as anything an app made is deleted:
  by soft delete, which keeps it, contents and all, for 30 days, so it can be restored as it was. One that isn't there
  is nothing to delete.
  """
  def remove(%Call{} = call, item) do
    db = Store.reached(call)

    Store.quietly("Deleting #{item}", fn ->
      if found = Items.find(call, db, item), do: delete!(call, db, Access.may!(found, {:app, call.app_artifact}))
      :ok
    end)
  end

  @doc """
  Renames the item `item` to `to`, for the app that calls this, which has access to it. Its fields, its contents and
  its access go with it.
  """
  def rename(%Call{} = call, item, to) do
    db = Store.reached(call)

    Store.quietly("Renaming #{item}", fn ->
      Store.rename(call, db, Access.may!(Items.find!(call, db, item), {:app, call.app_artifact}), to)
      :ok
    end)
  end

  @doc """
  Says how the item `item` is, for the app that calls this, which keeps it, so its person sees it in the Vault:
  `:ready`, usable, which shows nothing; `:busy`, something's under way, said in `message` ("Saving…"), which shows a
  spinner and it; or `:needs_action`, its person has to act, `message` saying why ("Signed out on your phone") and
  `action` the one thing that fixes it, which shows a dot, why and a button. The app keeps its own detailed states and
  says only which of these it's in (`Vault.Items`' status). `action` is the app's own: `%{label: "Sign In", tool:
  "sign_in", inputs: %{account: "+614..."}}`, a tool of the app's, called as its person, whose `url`, if it gives one,
  opens in their browser; or `%{label: "Sign In", page: "client/sign-in.html#+614..."}`, a page of the app's, opened
  over the Vault. Whether its person is notified too is the app's to choose, with `notifications`.
  """
  def state(%Call{} = call, item, state, message \\ nil, action \\ nil) do
    db = Store.reached(call)

    Store.quietly("Changing #{item}", fn ->
      Items.status!(call, db, Items.find!(call, db, item), call.app_artifact, state, message, action)
      Call.changed(call)
      :ok
    end)
  end

  @doc """
  Keeps the contents `name` of the item `item`, for the app that calls this, which has access to it: what's too large
  for a field, like a Chrome profile's hundreds of MB, kept as 1Password keeps a file. They're a blob of the item's,
  beside its record, sealed under a key made for that blob alone, which only the item holds, so soft delete deletes
  and restores them with it.

  `send` is given where they go, `%{"link" => link, "key" => key}`: a short-lived link that writes one blob, and the
  key, base64. It hands them to whatever holds the bytes, which seals them a chunk at a time and sends them, as
  Containers' `upload` does, so they never pass through a tool's result whatever their size, and gives how many bytes
  it sent. The item holds them once it has, and only then do the contents they replace go, so a write that fails
  leaves the last one whole. Gives the size.
  """
  def write_contents(%Call{} = call, item, name, send) when is_function(send, 1) do
    db = Store.reached(call)
    {blob, key} = {Base.encode16(:crypto.strong_rand_bytes(16), case: :lower), Base.encode64(:crypto.strong_rand_bytes(32))}

    Store.quietly("Keeping #{name} of #{item}", fn ->
      found = Access.may!(Items.find!(call, db, item), {:app, call.app_artifact})
      path = "#{found["id"]}/contents/#{blob}"
      size = send.(%{"link" => Store.link(db, path, :write), "key" => key})
      kept = %{"blob" => path, "key" => key, "size" => size, "at" => Store.now()}
      Store.change!(db, found, &Map.update(&1, "contents", %{name => kept}, fn contents -> Map.put(contents, name, kept) end))
      was = found["contents"][name]
      if was && was["blob"] != path, do: Artifact.remove(Store.root(db), was["blob"])
      size
    end)
  end

  @doc """
  Reads the contents `name` of the item `item`, for the app that calls this, which has access to it: `receive` is
  given where they are, `%{"link" => link, "key" => key, "size" => size}`, a short-lived link that reads the blob and
  its key, to hand to whatever takes the bytes, which fetches and opens them a chunk at a time, as Containers'
  `download` does. Gives what `receive` gives, or `nil` where the item has no such contents yet.
  """
  def read_contents(%Call{} = call, item, name, receive) when is_function(receive, 1) do
    db = Store.reached(call)

    Store.quietly("Reading #{name} of #{item}", fn ->
      found = Access.may!(Items.find!(call, db, item), {:app, call.app_artifact})

      if kept = found["contents"][name],
        do: receive.(%{"link" => Store.link(db, kept["blob"], :read), "key" => kept["key"], "size" => kept["size"]})
    end)
  end

  @doc """
  The items the app that calls this has access to, by name, each with the service whose sign-in made it, if one did,
  its artifact, which soft delete knows it by, and its status, as `state/5` last said it:
  `[%{name: "Google a@example.com", sign_in: "Google", artifact: "users/a/made/vault/x1", state: nil}]`. None before
  anything is.
  """
  def items(%Call{} = call) do
    if Call.reaches?(call, Store.theirs(call), "editor") do
      db = Store.reached(call)
      used_by(db, Items.all(call, db), call.app_artifact)
    else
      []
    end
  end

  defp used_by(db, items, app) do
    for item <- Enum.sort_by(items, &String.downcase(&1["name"])), app in item["access"] do
      %{name: item["name"], sign_in: item["oauth"] && item["oauth"]["name"], artifact: Store.artifact(db, item["id"]), state: item["status"]}
    end
  end

  # Deletes an item as anything an app made is deleted: by soft delete, which keeps it for 30 days.
  defp delete!(call, db, item), do: Call.tool(call, "soft-delete", "delete", %{"artifact" => Store.artifact(db, item["id"])})

  # The tools an app's code calls, which act for it. The rest are its person's.
  @apps ~w(fetch items sign_in state access)

  @impl true
  def call(tool, _inputs, %Call{caller: caller}) when caller != nil and tool not in @apps,
    do: raise("vault_#{tool} is its person's, not an app's. An app calls #{Enum.join(@apps, ", ")}.")

  def call(tool, _inputs, %Call{caller: nil}) when tool in ~w(items sign_in state),
    do: raise("vault_#{tool} is for an app's code, and acts for the app that calls it.")

  def call("items", _inputs, call) do
    db = Store.own(call)
    %{items: used_by(db, Items.all(call, db), call.caller)}
  end

  def call("state", %{"item" => name, "state" => state} = inputs, call) do
    db = Store.own(call)
    Items.status!(call, db, Items.find!(call, db, name), call.caller, state, inputs["message"], inputs["action"])
    Call.changed(call)
    %{item: name, state: state}
  end

  def call("sign_in", %{"service" => given} = inputs, call) do
    db = Store.own(call)
    service = SignIns.service(given)
    apps = inputs["apps"] || [Items.title(call, call.caller)]
    %{url: SignIns.start(call, db, service, call.caller, inputs["item"] || service.name, SignIns.began(call), apps)}
  end

  # From the database, which keeps each item's name while soft delete has it aside.
  def call("names", %{"artifacts" => artifacts}, call) do
    names = Store.names(call, Store.own(call))
    types = Items.types(call)
    # As a person reads it: "Personal", not "Chrome profile: Personal", of the kind its app calls it: "Chrome profile".
    shown = fn name ->
      case Enum.find(types, &String.starts_with?(name, &1.prefix)) do
        nil -> %{name: name, kind: nil}
        type -> %{name: String.replace_prefix(name, type.prefix, ""), kind: type.name}
      end
    end

    %{names: for({artifact, name} <- names, artifact in artifacts, do: Map.put(shown.(name), :artifact, artifact))}
  end

  def call("security", inputs, call) do
    opened = Store.encryption(call).unlocked_on(call, Store.items_artifact(call))
    key = Call.tool(call, "encryption", "key", Map.take(inputs, ["device"]))
    Map.merge(opened, %{readiness: readiness(key, opened.servers), owner: key["owner"], holds: key["slot"] != nil, device: key["device"], devices: key["devices"], asking: key["asking"]})
  end

  def call("grant", %{"servers" => _servers} = inputs, call) do
    keys =
      inputs["keys"] ||
        raise "Which servers open your Vault is chosen in the Vault's app on a trusted device.\n\nOr with `one vault grant` on it."

    %{servers: Store.encryption(call).choose(call, Store.items_artifact(call), keys)}
  end

  def call("list", _inputs, call) do
    db = Store.own(call)
    names = Items.names(call)
    types = Items.types(call)

    use_it =
      for({name, version} <- OneCore.Apps.running(call.subject), is_map_key(version.requires, "vault"), do: %{name: name, title: version.title || name, icon: OneCore.Apps.Icons.address(version.icon)})
      |> Enum.sort_by(&String.downcase(&1.title))

    items = for item <- Enum.sort_by(Items.all(call, db), &String.downcase(&1["name"])), do: Items.listed(item, db, names, types)
    %{items: items, requests: Requests.all(call, db, names), apps: use_it, types: Enum.map(types, &Map.delete(&1, :artifact))}
  end

  # A write of a field: into the item, or a new password where there's none. Websites given are added to its own,
  # never put in their place, and only by its person where it's there already.
  def call("set", %{"item" => name, "field" => field, "value" => value} = inputs, call) do
    db = Store.own(call)

    Store.quietly("Keeping #{field}", fn ->
      value = Store.encryption(call).secret(call, value)
      hosts = Map.get(inputs, "hosts", [])
      new_hosts!(call, Items.find(call, db, name), hosts)

      Items.write!(call, db, name, Items.password(hosts), :person, fn record ->
        record |> Items.add_hosts(hosts) |> Items.put_field(field, value, Map.get(inputs, "secret", true))
      end)

      %{item: name, field: field}
    end)
  end

  # A secret from a browser of theirs, which Chrome sends sealed under a key made here for it alone (its `upload`), so
  # it never passes through a tool's result. It goes only into a new item, or one a save from a browser made, which
  # its person sees was added, by whom and from where, and deletes as any item.
  def call("save_from_browser", %{"session" => session, "item" => name, "fields" => fields} = inputs, call) do
    db = Store.own(call)
    secret = Map.new(fields, fn {field, about} -> {Items.field!(field), Map.get(about, "secret", true)} end)

    Store.quietly("Saving from the browser", fn ->
      found = Items.find(call, db, name)

      if found && found["added"] == nil,
        do: raise("#{found["name"]} is in the Vault already, made by its person. Save to a new item, or ask them, with request.")

      new_hosts!(call, found, Map.get(inputs, "hosts", []))

      {blob, key} = {"saving/#{Store.random(12)}", :crypto.strong_rand_bytes(32)}
      taken = Map.new(fields, fn {field, about} -> {field, Map.take(about, ~w(target json))} end)
      upload = %{"session" => session, "link" => Store.link(db, blob, :write), "key" => Base.encode64(key), "fields" => taken}
      upload = if download = inputs["download"], do: Map.put(upload, "download", download), else: upload

      %{"site" => site, "values" => values} =
        try do
          Call.tool(call, "chrome", "upload", upload)
          {:ok, sealed, _tag} = Artifact.read(%{Store.root(db) | codec: nil}, blob)
          JSON.decode!(opened(sealed, key))
        after
          Artifact.remove(Store.root(db), blob)
        end

      if Enum.sort(Map.keys(values)) != Enum.sort(Map.keys(secret)), do: raise("The browser sent other fields than were asked for.")
      added = %{"by" => inputs["by"], "from" => site, "at" => Store.now()}
      hosts = Map.get(inputs, "hosts", [])

      saved =
        Items.write!(call, db, name, Items.password(hosts), :person, fn record ->
          record = record |> Items.add_hosts(hosts) |> Map.put("added", added)
          Enum.reduce(values, record, fn {field, value}, record -> Items.put_field(record, field, value, secret[field]) end)
        end)

      Call.changed(call)
      on = if inputs["by"], do: " on #{inputs["by"]}", else: ""

      Call.app(call, "notifications").send(call, %{
        "title" => "An agent#{on} saved #{saved["name"]} to your Vault",
        "text" => "From #{site}. Delete it in Vault if you didn't want it."
      })

      kept =
        for {field, value} <- Enum.sort(values) do
          sha256 = :sha256 |> :crypto.hash(value) |> Base.encode16(case: :lower) |> binary_part(0, 8)
          %{name: field, secret: secret[field], length: String.length(value), sha256: sha256}
        end

      %{item: saved["name"], site: site, fields: kept}
    end)
  end

  def call("remove_field", %{"item" => name, "field" => field}, call) do
    db = Store.own(call)
    found = Items.find!(call, db, name)
    if Items.fields(found)[field] == nil, do: raise("#{name} has no field named #{field}.")
    Store.change!(db, found, &Map.update!(&1, "fields", fn fields -> Enum.reject(fields, fn kept -> kept["name"] == field end) end))
    %{item: name, field: field}
  end

  # An app's own code may only take its own access away.
  def call("access", %{"item" => name} = inputs, %Call{caller: caller} = call) when caller != nil do
    if Map.get(inputs, "on", true), do: raise("An app is given access by its person, in the Vault, not by itself.")
    db = Store.own(call)
    Access.switch(call, db, Items.find!(call, db, name), caller, false)
    %{item: name, who: Items.app_name(call, caller), on: false}
  end

  # Taking access away is anyone's, and giving it is its person's.
  def call("access", %{"item" => name, "who" => who} = inputs, call) do
    db = Store.own(call)
    on = Map.get(inputs, "on", true)
    if on, do: Access.person!(call, "Letting #{who} use #{name}")
    Access.switch(call, db, Items.find!(call, db, name), Access.who!(call, who), on)
    %{item: name, who: who, on: on}
  end

  def call("request", %{"title" => title, "fields" => fields} = inputs, call) do
    db = Store.own(call)
    app = if name = inputs["app"], do: Access.who!(call, name)
    by = with %{agent: agent} <- SignIns.began(call), do: agent, else: (_ -> nil)
    id = Requests.create(call, db, title, fields, Map.get(inputs, "hosts", []), app, by, inputs["why"])
    link = if address = Application.get_env(:one_core, :address), do: "#{address}/vault/request?id=#{id}"
    where = if link, do: "Give them this link: #{link}. It", else: "Ask them to open Vault on a device of theirs: it"
    %{id: id, link: link, message: "#{where} shows the request, to answer on their own device. You never see what they type."}
  end

  def call("answer", %{"id" => id} = inputs, call) do
    Access.person!(call, "Answering a request")
    db = Store.own(call)

    Store.quietly("Answering the request", fn ->
      values = if given = inputs["values"], do: JSON.decode!(Store.encryption(call).secret(call, given))
      Requests.answer(call, db, id, Map.put(inputs, "values", values))
    end)
  end

  def call("expire", _inputs, call) do
    expired = Requests.expire(call, Store.own(call))
    if expired > 0, do: Call.changed(call)
    %{expired: expired}
  end

  def call("fetch", %{"item" => item, "url" => url} = inputs, call) do
    request = %{
      url: url,
      method: Map.get(inputs, "method", "GET"),
      headers: Map.get(inputs, "headers", %{}),
      body: inputs["body"],
      bytes: inputs["body_base64"] && Base.decode64!(inputs["body_base64"]),
      timeout: inputs["timeout"]
    }

    by = if call.caller, do: {:app, call.caller}, else: :agents
    response = fetched(call, Store.own(call), item, request, by)

    # What an agent reads is cut short, and what an app's code reads is whole.
    {body, cut} =
      case {by, response.body} do
        {:agents, <<body::binary-size(@most), rest::binary>>} -> {body, "#{byte_size(rest)} more bytes were left out"}
        {_by, body} -> {body, nil}
      end

    body = if String.valid?(body), do: %{body: body}, else: %{body_base64: Base.encode64(body)}
    Map.merge(%{status: response.status, headers: response.headers, cut: cut}, body)
  end

  def call("oauth_callback", inputs, call), do: SignIns.callback(call, inputs)

  # An item's fields filled into a request, for whoever may use it. Its tokens are renewed first if they've ended,
  # and once more if the site says they have.
  defp fetched(call, db, name, request, by) do
    Store.quietly("The request", fn ->
      item = Access.may!(Items.find!(call, db, name), by)
      secret = for %{"name" => field, "secret" => true} <- item["fields"], do: field
      oauth = item["oauth"]
      fields = if oauth && oauth["expires"] <= Store.now() + 60, do: SignIns.renewed(call, db, item, Items.fields(item)), else: Items.fields(item)
      response = Fetch.request(request, fields, secret, item["hosts"])

      if oauth && response.status == 401,
        do: Fetch.request(request, SignIns.renewed(call, db, item, fields), secret, item["hosts"]),
        else: response
    end)
  end

  # Sites an item that's there doesn't send its secrets to yet are its person's to add.
  defp new_hosts!(call, found, hosts) do
    new = if found, do: Items.hosts!(hosts) -- found["hosts"], else: []
    if new != [], do: Access.person!(call, "Sending #{found["name"]}'s secrets to #{Enum.join(new, ", ")}")
  end

  # The first thing someone's Vault lacks (`@readiness`), from their key and the servers that open it.
  defp readiness(key, servers) do
    cond do
      key["check"] == nil -> "not_set_up"
      servers == [] -> "no_server"
      key["slot"] == nil -> "device_lacks_key"
      true -> "ready"
    end
  end

  # What Chrome's `upload` sent, sealed as Containers seals a file (`Containers.Sealed`): a version byte, 1, then, a
  # secret being far less than its chunk of 1 MiB, one chunk sealed with AES-256-GCM under the nonce 0, as the last,
  # with its tag after it.
  defp opened(<<1, sealed::binary>>, key) when byte_size(sealed) >= 16 do
    {text, tag} = :erlang.split_binary(sealed, byte_size(sealed) - 16)

    case :crypto.crypto_one_time_aead(:aes_256_gcm, key, <<0::96>>, text, "last", tag, false) do
      :error -> raise "What the browser sent doesn't open with its key."
      plain -> plain
    end
  end

  defp opened(_sealed, _key), do: raise("What the browser sent isn't whole.")
end
