defmodule SignIn do
  @moduledoc """
  Signing in: whose each device is. People sign in at the network's address, where this plugin's edge part
  (`edge/`) keeps their passkeys and what they let act as them, and writes each device into this plugin's own
  artifact for the network as `devices/<device>`: the user whose it is. One Core asks whose a device is, the
  `device` hook, and it's answered from that blob, so no call asks anything of the edge. What each person goes
  by is there too, `people/<name>`, which `people` tells whoever may see who's in the network.

  What a person does with their devices in the app is asked of the edge part, which keeps them. A network
  with an address that isn't being built is joined by invitation, its first owner's included: until they've joined,
  the server's log says where, each time it's told who they are. A new link ends the one before.
  """
  @behaviour OneCore.Plugin
  require Logger
  alias OneCore.{Artifact, Call, Edge}

  @impl true
  def apps do
    [
      %{name: "People", page: "app/people.html", icon: "people.svg", settings: true},
      %{name: "Devices", page: "app/index.html", icon: "icon.svg", settings: true},
      %{name: "Passkeys", page: "app/passkeys.html", icon: "passkeys.svg", settings: true},
      %{name: "Connect Your AI", page: "app/connect.html", icon: "connect.svg", settings: true}
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
        }
      },
      %{
        name: "everyone",
        read_only: true,
        description:
          "Lists everyone in the network and everyone invited to it: the name each goes by, and whether they've joined",
        inputs: none
      },
      %{
        name: "remove_person",
        description:
          "Takes someone out of the network: their devices, assistants and passkeys stop working, and what they made stays",
        inputs: %{
          type: "object",
          properties: %{who: %{type: "string", pattern: "^users/[a-z][a-z0-9-]*$", description: "Who: users/<name>"}},
          required: ["who"]
        }
      },
      %{
        name: "add_passkey",
        description:
          "Makes a link that you make another passkey by, in a browser, within a quarter of an hour. Open it, don't pass it on",
        inputs: none
      },
      %{
        name: "devices",
        read_only: true,
        description: "Lists your devices, assistants and browsers, and your passkeys",
        inputs: none
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
        }
      },
      %{
        name: "name",
        description: "Changes the name you go by in the network",
        inputs: %{
          type: "object",
          properties: %{called: %{type: "string", minLength: 1, description: "Your name, as people know you"}},
          required: ["called"]
        }
      },
      %{
        name: "let_in",
        description: "Lets in a device that shows a code, as yours: one you were asked to let in",
        inputs: %{
          type: "object",
          properties: %{code: %{type: "string", description: "The code the device shows, as WDJB-MJHT"}},
          required: ["code"]
        }
      },
      %{
        name: "remove_device",
        description: "Takes one of your devices, assistants or browsers out. It's nobody's from then on",
        inputs: %{
          type: "object",
          properties: %{device: %{type: "string", description: "The device's id, as `devices` lists it"}},
          required: ["device"]
        }
      },
      %{
        name: "remove_passkey",
        description: "Removes one of your passkeys",
        inputs: %{
          type: "object",
          properties: %{passkey: %{type: "string", description: "The passkey's id, as `devices` lists it"}},
          required: ["passkey"]
        }
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
      _ -> %{}
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
  # Who invites is whoever may change what this plugin keeps for the network: its owners, and whoever they made
  # an editor of it.
  def call("invite", %{"name" => name}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    invite(name)
  end

  def call("everyone", _inputs, call) do
    if not Call.member?(call, "network"), do: raise("Only a member of the network sees who's in it.")
    ask(%{ask: "everyone"})["people"]
  end

  # Whoever invites takes out, and nobody takes themselves out.
  def call("remove_person", %{"who" => who}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    if who == call.by, do: raise("You can't take yourself out of the network.")
    %{"removed" => name} = ask(%{ask: "remove_person", who: who})
    Call.tool(call, "relations", "forget", %{"who" => who})
    "#{name} is out of the network. What they made stays."
  end

  def call("add_passkey", _inputs, call), do: ask(%{ask: "passkey", who: person(call)})["link"]

  def call("devices", _inputs, call), do: ask(%{ask: "devices", who: person(call)})

  # Whoever may see who's in the network sees who each of them is: a member of it, as the plugins that decide
  # `reach` say. Everyone sees who they are themselves.
  def call("people", inputs, call) do
    users = Map.get(inputs, "users", [person(call)])

    if users -- [call.by] != [] and not Call.member?(call, "network"),
      do: raise("Only a member of the network sees who's in it.")

    for "users/" <> name = user <- users do
      case Artifact.read(call.own, "people/" <> name) do
        {:ok, said, _tag} -> Map.put(JSON.decode!(said), "who", user)
        :absent -> %{"who" => user, "called" => nil, "email" => nil}
      end
    end
  end

  def call("name", %{"called" => called}, call) do
    %{"called" => called} = ask(%{ask: "called", who: person(call), called: called})
    "You go by #{called} now."
  end

  def call("let_in", %{"code" => code}, call) do
    %{"let_in" => name} = ask(%{ask: "let_in", who: person(call), code: code})
    "Let in: #{name} is yours now."
  end

  def call("remove_device", %{"device" => device}, call) do
    %{"removed" => name} = ask(%{ask: "remove_device", who: person(call), device: device})
    "#{name} is out, and is nobody's from now on."
  end

  def call("remove_passkey", %{"passkey" => passkey}, call) do
    %{"removed" => name} = ask(%{ask: "remove_passkey", who: person(call), passkey: passkey})
    "#{name} is removed."
  end

  defp invite(name) do
    %{"link" => link} = ask(%{ask: "invite", who: name})
    joins(name, link)
  end

  defp joins(name, link), do: "users/#{name} joins at #{link} within a week. It's shown once, so pass it on now."

  # Devices are a person's, whoever a call runs for.
  defp person(%Call{by: "users/" <> _ = by}), do: by
  defp person(_call), do: raise("This is a person's to ask, and nobody is asking.")

  # Asks this plugin's edge part, as this server: it answers only the network's own.
  defp ask(what) do
    address =
      Application.get_env(:one_core, :address) ||
        raise "People sign in at a network's address, and this network has none."

    body = JSON.encode!(what)

    asked = [method: :post, url: address <> "/sign-in/ask", body: body, headers: Edge.signed("/sign-in/ask", body)]

    case Edge.send!(asked ++ [retry: false]) do
      %{status: 200, body: %{"error" => why}} -> raise why
      %{status: 200, body: answer} -> answer
      %{status: status} -> raise "The network's edge answered #{status}"
    end
  end
end
