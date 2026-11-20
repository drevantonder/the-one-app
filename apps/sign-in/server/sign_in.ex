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
  def requires, do: %{"core" => ">= 0.32.167"}

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are what people sign in with.
  @impl true
  def sensitive, do: true

  require Logger
  alias OneCore.{Artifact, Builds, Call, Edge}

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    [
      %{
        name: "invite",
        description: """
        Makes a link someone joins the network by, as the user you name. It ends the one made for them before. Only the
        network's owner invites someone who has joined again, and never an owner
        """,
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
        description: """
        Lets you make another passkey, in a browser, within a quarter of an hour. Whoever opens its link makes a
        passkey of yours, so the link goes only to your own screen: an assistant's call notifies you instead
        """,
        inputs: none,
        outputs: %{
          type: "object",
          properties: %{
            link: %{type: "string", description: "Where to make it, given only to the app or web client on your own device"},
            message: %{type: "string", description: "What to tell your person, when you were given no link"}
          }
        }
      },
      %{
        name: "encrypt_for_passkeys",
        description: """
        Lets a passkey unlock your key on a new device, once your person has said it may: from a device that holds the
        key, encrypts it to that passkey's key, which its PRF output derives and it says as it's used to sign in.
        Whoever uses the passkey has your key from then on, so ask them first, naming it and when it was added
        """,
        inputs: %{
          type: "object",
          properties: %{
            passkey: %{type: "string", description: "The passkey your person allowed, by its id as `devices` lists it"},
            code: %{type: "string", description: "Its code as `devices` lists it, which your person checked is the one the sign-in page showed them"},
            keys: %{
              type: "string",
              format: "passkey-keys",
              passkeys: "devices",
              description: "Your key, encrypted to that passkey, which this device fills in"
            }
          },
          required: ["passkey"]
        },
        outputs: %{
          type: "object",
          properties: %{passkeys: %{type: "integer", description: "1 once it unlocks your key, 0 if it couldn't"}},
          required: ["passkeys"]
        }
      },
      %{
        name: "unlock_with_passkey",
        description: """
        Makes a link that lets a device of yours that's asking for your key hold it, once you use a passkey that
        unlocks it there, in a browser. Without a device, using the passkey there says the key it unlocks with, which
        a device that holds yours encrypts it to once you allow it there (`encrypt_for_passkeys`). Open it, don't pass
        it on.
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
                  unlocks: %{type: "boolean", description: "Whether it unlocks your key on a new device"},
                  code: %{
                    type: ["string", "null"],
                    description: "Its key's code, which the sign-in page showed as it was used: its person checks they're the same before it's allowed"
                  }
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
          "Lists the devices waiting to sign in as you: each named you, and shows a number its person types to let it in",
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
                  device: %{type: "string", description: "Its key, which `encryption_approve` takes to hand it yours"},
                  name: %{type: "string", description: "What the device calls itself"},
                  at: %{type: "string", description: "When it named you, ISO 8601"},
                  place: %{type: ["string", "null"], description: "Roughly where it asked from: a city"},
                  ends: %{type: "string", description: "When it stops waiting, ISO 8601"}
                },
                required: ["request", "device", "name", "at", "place", "ends"]
              }
            }
          },
          required: ["count", "waiting"]
        }
      },
      %{
        name: "approve",
        description:
          "Approves a device as yours: by the code it shows, or one that's waiting by the number its person reads off it. A wrong number blocks it",
        inputs: %{
          type: "object",
          properties: %{
            code: %{type: "string", description: "The code the device shows, as WDJB-MJHT"},
            request: %{type: "string", description: "A device waiting to sign in as you, as `waiting` lists it"},
            number: %{type: "integer", description: "With `request`: the number the device shows, as its person says it"}
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
  def hooks, do: ["device", "owner", "settings.sections"]

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

  # Its sections of Settings: whoever's looking (Sign In & Security under their name), Devices, with the devices waiting
  # to sign in as its red count, AI, and People & Groups, an admin thing, for the network's owners alone.
  def hook("settings.sections", _about, call) do
    people = %{in: "settings", name: "People & Groups", component: "people", icon: "client/users.svg", group: "network", order: 2}

    [
      %{in: "settings", name: "Sign In & Security", component: "passkeys", icon: "client/security.svg", group: "you", order: 0},
      %{in: "settings", name: "Devices", component: "devices", icon: "client/icon.svg", group: "network", order: 1, badge: "waiting"},
      %{in: "settings", name: "AI", component: "ai", icon: "client/connect.svg", group: "general", order: 3}
    ] ++ if(Call.owns?(%{call | subject: "network"}), do: [people], else: [])
  end

  # Only a network that has an address and isn't being built has people sign in. One being built acts as whoever a
  # client says, so its first owner joins nowhere. One who has joined is given no link to join by again.
  def hook("owner", %{"who" => "users/" <> name = who}, _call) do
    signs_in = Application.get_env(:one_core, :address) && !Application.get_env(:one_core, :development, false)

    with true <- signs_in and OneCore.User.first_owner?(who),
         %{"link" => link} <- ask(%{ask: "owner", who: name}),
         do: Logger.info(joins(name, link))

    :ok
  end

  @impl true
  # Who invites is whoever may change what this app keeps for the network: its owners, and whoever they made
  # an editor of it. Whoever joins by an invitation is that user, so someone who has joined is invited again only by
  # the network's owner, and an owner never: they add a passkey on a device they're signed in on, as Apple and
  # Google have it. The edge knows who has joined, and is told whether this may be that (`again`). A name someone was
  # taken out of while what they made is kept is theirs still, since all of it is reached by that name, so nobody
  # else is invited by it, as GitHub retires the name of an account that still has repos.
  def call("invite", %{"name" => name}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    who = "users/#{name}"

    if not Enum.any?(ask(%{ask: "everyone"})["people"], &(&1["who"] == who)) and OneCore.Bucket.list(who <> "/") != [],
      do: raise("#{who} was taken out of the network, and what they made is kept under that name, so nobody else is given it. Invite them by another name.")

    again = owns_network?(call, call.actor) and not owns_network?(call, who)
    %{"link" => link} = ask(%{ask: "invite", who: name, again: again})
    %{who: "users/#{name}", link: link}
  end

  def call("everyone", _inputs, call) do
    if not Call.member?(call, "network"), do: raise("Only a member of the network sees who's in it.")
    %{people: ask(%{ask: "everyone"})["people"]}
  end

  # Whoever invites takes out, and nobody takes themselves out. Only an owner takes out an owner, as only a super
  # admin removes another in Google Workspace.
  def call("remove_person", %{"who" => who}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    if who == call.actor, do: raise("You can't take yourself out of the network.")

    if owns_network?(call, who) and not owns_network?(call, call.actor),
      do: raise("Only the network's owners take out one of its owners.")
    ask(%{ask: "remove_person", who: who})
    Call.tool(call, "permissions", "forget", %{"who" => who})
    %{who: who}
  end

  # The link makes a passkey of theirs for whoever opens it, so it's shown only on their own screen, which opens it.
  # Anything else that asks, an assistant or `one`, has them notified, and Sign In & Security makes it there.
  def call("add_passkey", _inputs, call) do
    who = person(call)

    if screen?(call, who) do
      %{link: ask(%{ask: "passkey", who: who})["link"]}
    else
      Call.tool(call, "notifications", "send", %{
        "title" => "Add a passkey",
        "text" => "Open Sign In & Security and tap Add Passkey.",
        "to" => who,
        "app" => call.app,
        "section" => "passkeys"
      })

      %{message: "Your person was notified: Sign In & Security, on a device of theirs, makes it. The link is shown only there."}
    end
  end

  def call("devices", _inputs, call), do: devices(person(call))

  def call("encrypt_for_passkeys", %{"passkey" => passkey, "keys" => keys}, call) do
    case JSON.decode(keys) do
      {:ok, %{} = keys} ->
        %{passkeys: ask(%{ask: "encrypt_for_passkeys", who: person(call), passkey: passkey, keys: keys})["passkeys"]}

      _ ->
        raise "That isn't your key encrypted to your passkey. A device that holds it fills it in."
    end
  end

  def call("encrypt_for_passkeys", _inputs, _call),
    do: raise("A device that holds your key encrypts it to a passkey your person allowed: in Sign In & Security there, or with `one` on it.")

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
          "text" => "Open Settings › Devices to let it in.",
          "to" => who,
          "section" => "devices#waiting=#{request}"
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
         %{"devices" => devices} <- devices(who),
         do: Enum.find_value(devices, &(&1["id"] == device && &1["name"])),
         else: (_ -> nil)
  end

  # What the edge says a person's devices are, each app under the name it says it has now: the edge names a device as
  # it signed in, and an app says what it's called each time it connects.
  defp devices(who) do
    %{"devices" => devices} = list = ask(%{ask: "devices", who: who})
    names = Builds.names(who, for(%{"kind" => "app", "id" => id} <- devices, do: id))
    passkeys = for passkey <- list["passkeys"] || [], do: Map.put(passkey, "code", code(passkey["prf_key"]))
    %{list | "devices" => for(device <- devices, do: Map.update!(device, "name", &Map.get(names, device["id"], &1))), "passkeys" => passkeys}
  end

  # A passkey's key, as its person checks it's the one they used, as Signal's safety numbers are: 20 digits in fours
  # of five from its SHA-256, the same the sign-in page shows (`edge/page/passkey.ts`) and a device holds an Allow to
  # (`apps/encryption/client/native/src/device.rs`).
  defp code(key) when is_binary(key) and key != "" do
    <<number::72, _::binary>> = :crypto.hash(:sha256, "one/passkey/1/code\n" <> key)
    digits = number |> rem(Integer.pow(10, 20)) |> Integer.to_string() |> String.pad_leading(20, "0")
    digits |> String.graphemes() |> Enum.chunk_every(5) |> Enum.map_join(" ", &Enum.join/1)
  end

  defp code(_key), do: nil

  # Whether `who` owns the network, as the apps that validate `reach` say.
  defp owns_network?(call, who) do
    request = %{"who" => who, "relation" => "owner", "artifact" => "network"}
    OneCore.Hooks.validate("reach", request, {call.actor, call.subject}) == :ok
  end

  # Whether a call comes from its person's own screen: the app, or the web client, on a device of theirs that isn't an
  # assistant. A client's name is only what it says (`Call.client/1`), so an assistant, which comes over the web too and
  # may say it's anything, is told apart by its device, which the edge knows. What runs on a device with its key, as
  # `one` does, could say it's the app as well, and holds what the device does already.
  defp screen?(call, who) do
    case Call.client(call) do
      %{"name" => name, "device" => device} when name in ["one-app", "one-web"] ->
        %{"devices" => devices} = ask(%{ask: "devices", who: who})
        not Enum.any?(devices, &(&1["id"] == device and &1["kind"] == "assistant"))

      _ ->
        false
    end
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

    # Signed as every ask of the edge is, so one refused for this server's clock is signed again by the edge's.
    case Edge.ask!([method: :post, url: address() <> "/sign-in/ask", body: body, retry: false], "/sign-in/ask") do
      %{status: 200, body: %{"error" => why}} -> raise why
      %{status: 200, body: response} -> response
      %{status: status} -> raise "The network's edge answered #{status}"
    end
  end
end
