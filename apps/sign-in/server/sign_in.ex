defmodule SignIn do
  @moduledoc """
  Signing in: whose each device is. People sign in at the network's address, where this app's edge function
  (`edge/`) keeps their passkeys and what they let act as them, and writes each device into this app's own
  artifact for the network as `devices/<device>`: the user whose it is. One Core asks whose a device is, the
  `device` hook, and it's answered from that blob, so no call asks anything of the edge. What each person goes
  by is there too, `people/<name>`, which `people` tells whoever may see who's in the network.

  What a person does with their devices in the app is asked of the edge function, which keeps them. A network
  with an address that isn't being built is joined by invitation, its first owner's included: until they've joined,
  the server's log says where, each time it's told who they are. A new link ends the one before.
  """
  @behaviour OneCore.App

  # Its data is the network's people and devices, which its edge function keeps for everyone.
  @impl true
  def own, do: :installed

  # One Core asks it whose a device is, and every client signs in through it.
  @impl true
  def icon, do: "client/security.svg"

  @impl true
  def requires, do: %{"core" => ">= 0.24.20"}

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are what people sign in with.
  @impl true
  def sensitive, do: true

  require Logger
  alias OneCore.{Artifact, Call, Edge}

  @impl true
  def entrypoints do
    [
      %{name: "Users & Groups", page: "client/users.html", icon: "client/users.svg", category: "Admin"},
      %{
        name: "Devices",
        page: "client/index.html",
        icon: "client/icon.svg",
        category: "Devices",
        subject: "user",
        badge: "waiting"
      },
      %{
        name: "Sign-In & Security",
        page: "client/security.html",
        subject: "user"
      },
      %{
        name: "Connect Your AI",
        page: "client/connect.html",
        icon: "client/connect.svg",
        category: "AI",
        subject: "user"
      }
    ]
  end

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    [
      %{
        name: "invite",
        description:
          "Makes a link someone joins the network by, as the user you name. It ends the one made for them before",
        inputs: %{
          type: "object",
          properties: %{name: %{type: "string", description: "Their name: lowercase letters, digits and hyphens"}},
          required: ["name"]
        },
        outputs: %{
          type: "object",
          properties: %{
            who: %{type: "string", description: "users/<name>"},
            link: %{type: "string", description: "Where they join, within a week. It's shown once, so pass it on now"}
          },
          required: ["who", "link"]
        }
      },
      %{
        name: "everyone",
        read_only: true,
        description:
          "Lists everyone in the network and everyone invited to it: the name each goes by, and whether they've joined",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            people: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  who: %{type: "string", description: "users/<name>"},
                  called: %{type: ["string", "null"], description: "The name they go by"},
                  email: %{type: ["string", "null"]},
                  joined: %{type: "boolean", description: "Whether they've joined, or are only invited"}
                },
                required: ["who", "called", "email", "joined"]
              }
            }
          },
          required: ["people"]
        }
      },
      %{
        name: "remove_person",
        description:
          "Takes someone out of the network: their devices, assistants and passkeys stop working, and what they made stays",
        inputs: %{
          type: "object",
          properties: %{who: %{type: "string", pattern: "^users/[a-z][a-z0-9-]*$", description: "Who: users/<name>"}},
          required: ["who"]
        },
        outputs: %{type: "object", properties: %{who: %{type: "string"}}, required: ["who"]}
      },
      %{
        name: "add_passkey",
        description:
          "Makes a link that you make another passkey by, in a browser, within a quarter of an hour. Open it, don't pass it on",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            link: %{type: "string", description: "Where to make it, in a browser, within a quarter of an hour"}
          },
          required: ["link"]
        }
      },
      %{
        name: "encrypt_for_passkeys",
        description: """
        Lets your passkeys unlock your key on a new device: from a device that holds it, encrypts it to each passkey
        whose PRF output derives a key and that doesn't unlock it yet. A passkey says that key as it's used to sign in.
        """,
        inputs: %{
          type: "object",
          properties: %{
            keys: %{
              type: "string",
              format: "passkey-keys",
              passkeys: "devices",
              description: "Your key, encrypted to each passkey, which this device fills in"
            }
          }
        },
        outputs: %{
          type: "object",
          properties: %{passkeys: %{type: "integer", description: "How many more unlock it"}},
          required: ["passkeys"]
        }
      },
      %{
        name: "unlock_with_passkey",
        description: """
        Makes a link that lets a device of yours that's asking for your key hold it, once you use a passkey that
        unlocks it there, in a browser. Without a device, using the passkey there lets it unlock your key: a device
        that holds it encrypts it to the passkey next. Open it, don't pass it on.
        """,
        inputs: %{
          type: "object",
          properties: %{device: %{type: "string", description: "The device's key, as `encryption_key` lists it asking"}}
        },
        outputs: %{type: "object", properties: %{link: %{type: "string"}}, required: ["link"]}
      },
      %{
        name: "devices",
        read_only: true,
        description: "Lists your devices, assistants and browsers, and your passkeys",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            devices: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string", description: "What remove_device takes"},
                  kind: %{type: "string", description: "app, assistant or browser"},
                  name: %{type: "string"},
                  added: %{type: "string", description: "When, ISO 8601"},
                  connected: %{type: "boolean", description: "Whether an app is connected now"},
                  last_seen: %{type: "string", description: "When an app that isn't connected was last, ISO 8601"}
                },
                required: ["id", "kind", "name", "added"]
              }
            },
            passkeys: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  id: %{type: "string", description: "What remove_passkey takes"},
                  name: %{type: "string"},
                  added: %{type: "string", description: "When, ISO 8601"},
                  credential: %{type: "string", description: "Its WebAuthn credential ID"},
                  prf_key: %{
                    type: ["string", "null"],
                    description:
                      "The key its PRF output derives, which your key is encrypted to so it unlocks it on a new device: empty if it has no PRF, null until it's next used"
                  },
                  unlocks: %{type: "boolean", description: "Whether it unlocks your key on a new device"}
                },
                required: ["id", "name", "added", "credential", "prf_key", "unlocks"]
              }
            }
          },
          required: ["devices", "passkeys"]
        }
      },
      %{
        name: "people",
        read_only: true,
        description:
          "Says who people in the network are: the name each goes by, and their email where signing in has one",
        inputs: %{
          type: "object",
          properties: %{
            users: %{
              type: "array",
              items: %{type: "string", pattern: "^users/[a-z][a-z0-9-]*$"},
              description: "Who: users/<name>. You, if left out"
            }
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            people: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  who: %{type: "string", description: "users/<name>"},
                  called: %{type: ["string", "null"], description: "The name they go by"},
                  email: %{type: ["string", "null"], description: "Their email, where signing in has one"}
                },
                required: ["who", "called", "email"]
              }
            }
          },
          required: ["people"]
        }
      },
      %{
        name: "name",
        description: "Changes the name you go by in the network",
        inputs: %{
          type: "object",
          properties: %{called: %{type: "string", minLength: 1, description: "Your name, as people know you"}},
          required: ["called"]
        },
        outputs: %{type: "object", properties: %{called: %{type: "string"}}, required: ["called"]}
      },
      %{
        name: "waiting",
        read_only: true,
        description:
          "Lists the devices waiting to sign in as you: each named you, and shows a number that one of the three here is",
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            count: %{type: "integer", description: "How many are waiting"},
            waiting: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{
                  request: %{type: "string", description: "What approve and deny take"},
                  name: %{type: "string", description: "What the device calls itself"},
                  at: %{type: "string", description: "When it named you, ISO 8601"},
                  place: %{type: ["string", "null"], description: "Roughly where it asked from: a city"},
                  ends: %{type: "string", description: "When it stops waiting, ISO 8601"},
                  numbers: %{type: "array", items: %{type: "integer"}, description: "One of them is the number it shows"}
                },
                required: ["request", "name", "at", "place", "ends", "numbers"]
              }
            }
          },
          required: ["count", "waiting"]
        }
      },
      %{
        name: "approve",
        description:
          "Approves a device as yours: by the code it shows, or one that's waiting by the number it shows. A wrong number blocks it",
        inputs: %{
          type: "object",
          properties: %{
            code: %{type: "string", description: "The code the device shows, as WDJB-MJHT"},
            request: %{type: "string", description: "A device waiting to sign in as you, as `waiting` lists it"},
            number: %{type: "integer", description: "With `request`: the number the device shows"}
          }
        },
        outputs: %{
          type: "object",
          properties: %{approved: %{type: "string", description: "The device's name, which is yours now"}},
          required: ["approved"]
        }
      },
      %{
        name: "deny",
        description: "Blocks a device waiting to sign in as you, which you didn't ask for",
        inputs: %{
          type: "object",
          properties: %{request: %{type: "string", description: "As `waiting` lists it"}},
          required: ["request"]
        },
        outputs: %{type: "object", properties: %{denied: %{type: "string", description: "Its name"}}, required: ["denied"]}
      },
      %{
        name: "notify",
        by_link: true,
        description:
          "Notifies someone that a device is waiting to sign in as them. The network's edge calls this as the device names them",
        inputs: %{
          type: "object",
          properties: %{
            who: %{type: "string", pattern: "^users/[a-z][a-z0-9-]*$"},
            request: %{type: "string"}
          },
          required: ["who", "request"]
        },
        outputs: %{type: "object", properties: %{notified: %{type: "boolean"}}, required: ["notified"]}
      },
      %{
        name: "remove_device",
        description: "Removes one of your devices, assistants or browsers. It's signed out from then on",
        inputs: %{
          type: "object",
          properties: %{device: %{type: "string", description: "The device's id, as `devices` lists it"}},
          required: ["device"]
        },
        outputs: %{
          type: "object",
          properties: %{removed: %{type: "string", description: "Its name"}},
          required: ["removed"]
        }
      },
      %{
        name: "remove_passkey",
        description: "Removes one of your passkeys",
        inputs: %{
          type: "object",
          properties: %{passkey: %{type: "string", description: "The passkey's id, as `devices` lists it"}},
          required: ["passkey"]
        },
        outputs: %{
          type: "object",
          properties: %{removed: %{type: "string", description: "Its name"}},
          required: ["removed"]
        }
      }
    ]
  end

  @impl true
  def offers do
    [
      %{
        name: "device_removed",
        kind: :observe,
        description:
          "One of a person's devices, assistants or browsers was removed with `remove_device`: its `device` key, " <>
            "as the `device` hook knows it, and `who` it was, users/<name>. An app that keeps anything for the " <>
            "device, as where its push service reaches it, forgets it: it's nobody's from then on."
      }
    ]
  end

  @impl true
  def hooks, do: ["device", "owner"]

  @impl true
  def hook("device", %{"device" => device}, call) do
    with true <- device =~ ~r/^[0-9a-f]{64}$/,
         {:ok, who, _tag} <- Artifact.read(call.own, "devices/" <> device) do
      %{"who" => who}
    else
      # A device it doesn't know is nobody's. A read that failed raises: that's not knowing, not nobody's.
      not_known when not_known in [false, :absent] -> %{}
    end
  end

  # Only a network that has an address and isn't being built has people sign in. One being built acts as whoever a
  # client says, so its first owner joins nowhere. One who has joined is given no link to join by again.
  def hook("owner", %{"who" => "users/" <> name}, _call) do
    signs_in = Application.get_env(:one_core, :address) && !Application.get_env(:one_core, :development, false)

    with true <- signs_in,
         %{"link" => link} <- ask(%{ask: "owner", who: name}),
         do: Logger.info(joins(name, link))

    :ok
  end

  @impl true
  # Who invites is whoever may change what this app keeps for the network: its owners, and whoever they made
  # an editor of it.
  def call("invite", %{"name" => name}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    %{"link" => link} = ask(%{ask: "invite", who: name})
    %{who: "users/#{name}", link: link}
  end

  def call("everyone", _inputs, call) do
    if not Call.member?(call, "network"), do: raise("Only a member of the network sees who's in it.")
    %{people: ask(%{ask: "everyone"})["people"]}
  end

  # Whoever invites takes out, and nobody takes themselves out.
  def call("remove_person", %{"who" => who}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    if who == call.actor, do: raise("You can't take yourself out of the network.")
    ask(%{ask: "remove_person", who: who})
    Call.tool(call, "permissions", "forget", %{"who" => who})
    %{who: who}
  end

  def call("add_passkey", _inputs, call), do: %{link: ask(%{ask: "passkey", who: person(call)})["link"]}

  def call("devices", _inputs, call), do: ask(%{ask: "devices", who: person(call)})

  def call("encrypt_for_passkeys", %{"keys" => keys}, call) do
    case JSON.decode(keys) do
      {:ok, %{} = keys} -> %{passkeys: ask(%{ask: "encrypt_for_passkeys", who: person(call), keys: keys})["passkeys"]}
      _ -> raise "Those aren't your key encrypted to your passkeys. A device that holds it fills them in."
    end
  end

  def call("encrypt_for_passkeys", _inputs, _call),
    do: raise("A device that holds your key encrypts it to your passkeys: in the app there, or with `one` on it.")

  def call("unlock_with_passkey", inputs, _call) do
    device = inputs["device"] || ""
    if not (device =~ ~r/^([0-9a-f]{64})?$/), do: raise("A device is named by its key.")
    %{link: address() <> "/sign-in/#unlock=" <> device}
  end

  # Whoever may see who's in the network sees who each of them is: a member of it, as the apps that validate
  # `reach` say. Everyone sees who they are themselves.
  def call("people", inputs, call) do
    users = Map.get(inputs, "users", [person(call)])

    if users -- [call.actor] != [] and not Call.member?(call, "network"),
      do: raise("Only a member of the network sees who's in it.")

    people =
      for "users/" <> name = user <- users do
        case Artifact.read(call.own, "people/" <> name) do
          {:ok, stored, _tag} ->
            Map.merge(%{"called" => nil, "email" => nil}, Map.put(JSON.decode!(stored), "who", user))

          :absent ->
            %{"who" => user, "called" => nil, "email" => nil}
        end
      end

    %{people: people}
  end

  def call("name", %{"called" => called}, call) do
    %{"called" => called} = ask(%{ask: "called", who: person(call), called: called})
    %{called: called}
  end

  def call("waiting", _inputs, call) do
    %{"waiting" => waiting} = ask(%{ask: "waiting", who: person(call)})
    %{count: length(waiting), waiting: waiting}
  end

  def call("approve", %{"request" => request, "number" => number}, call) do
    %{"approved" => name} = ask(%{ask: "approve", who: person(call), request: request, number: number})
    Call.changed(call)
    %{approved: name}
  end

  def call("approve", %{"code" => code}, call) do
    %{"approved" => name} = ask(%{ask: "approve", who: person(call), code: code})
    %{approved: name}
  end

  def call("approve", _inputs, _call), do: raise("Say the code the device shows, or the request and the number it shows.")

  def call("deny", %{"request" => request}, call) do
    %{"denied" => name} = ask(%{ask: "deny", who: person(call), request: request})
    Call.changed(call)
    %{denied: name}
  end

  # The edge says a device named someone, as the holder of a link it makes of them and the request. It's believed
  # only as far as the edge, asked as this server, lists that request as waiting for them: then they're notified,
  # and their Devices shows it.
  def call("notify", %{"who" => who, "request" => request}, %Call{actor: actor} = call) do
    if actor != OneCore.User.link("#{who}/#{request}"), do: raise("Only the network's edge says a device named someone.")
    %{"waiting" => waiting} = ask(%{ask: "waiting", who: who})

    case Enum.find(waiting, &(&1["request"] == request)) do
      nil ->
        %{notified: false}

      %{"name" => name} ->
        call = %{call | actor: nil, subject: who}
        Call.changed(call)

        Call.tool(call, "notifications", "send", %{
          "title" => "#{name} wants to sign in as you",
          "text" => "Open Devices to let it in.",
          "to" => who,
          "app" => call.app,
          "page" => "client/index.html#waiting=#{request}"
        })

        %{notified: true}
    end
  end

  def call("remove_device", %{"device" => device}, call) do
    %{"removed" => name, "device" => key} = ask(%{ask: "remove_device", who: person(call), device: device})
    Call.run_hook(call, "device_removed", %{"device" => key, "who" => person(call)})
    %{removed: name}
  end

  def call("remove_passkey", %{"passkey" => passkey}, call) do
    %{"removed" => name} = ask(%{ask: "remove_passkey", who: person(call), passkey: passkey})
    %{removed: name}
  end

  @doc """
  What the device whose key is `device` is called where its person let it in, as `devices` lists it for them, or
  `nil` for one that's nobody's. It's for an app that shows a person's device to whoever else may use it, by the name
  its person knows it by, as Computer Use names a phone.

      Call.app(call, "sign-in").device_name(call, device)
  """
  def device_name(_call, device) do
    with "users/" <> _ = who <- OneCore.Hooks.whose(device),
         %{"devices" => devices} <- ask(%{ask: "devices", who: who}),
         do: Enum.find_value(devices, &(&1["id"] == device && &1["name"])),
         else: (_ -> nil)
  end

  defp joins(name, link), do: "users/#{name} joins at #{link} within a week. It's shown once, so pass it on now."

  # Devices are a person's, whoever a call runs for.
  defp person(%Call{actor: "users/" <> _ = by}), do: by
  defp person(_call), do: raise("This is a person's to ask, and nobody is asking.")

  # Asks this app's edge function, as this server: it answers only the network's own.
  defp address do
    Application.get_env(:one_core, :address) ||
      raise "People sign in at a network's address, and this network has none."
  end

  defp ask(what) do
    # A network being built acts as whoever a client says, who joined nowhere, so the edge makes them as it's asked.
    body = JSON.encode!(Map.put(what, :building, Application.get_env(:one_core, :development, false)))

    request = [method: :post, url: address() <> "/sign-in/ask", body: body, headers: Edge.signed("/sign-in/ask", body)]

    case Edge.send!(request ++ [retry: false]) do
      %{status: 200, body: %{"error" => why}} -> raise why
      %{status: 200, body: response} -> response
      %{status: status} -> raise "The network's edge answered #{status}"
    end
  end
end
