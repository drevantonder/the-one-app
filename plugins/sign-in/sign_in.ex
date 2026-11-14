defmodule SignIn do
  @moduledoc """
  Signing in: whose each device is. People sign in at the network's address, where this plugin's part there
  (`address/`) keeps their passkeys and what they let act as them, and writes each device into this plugin's own
  artifact for the network as `devices/<device>`: the user whose it is. One Core asks whose a device is, the
  `device` hook, and it's answered from that blob, so no call asks anything of the address.

  What a person does with their devices in the app is asked of the part at the address, which keeps them.
  """
  @behaviour OneCore.Plugin
  alias OneCore.{Address, Artifact, Call}

  @impl true
  def tools do
    none = %{type: "object", properties: %{}}

    [
      %{
        name: "invite",
        description: "Makes a link someone joins the network by, as the user you name",
        inputs: %{
          type: "object",
          properties: %{name: %{type: "string", description: "Their name: lowercase letters, digits and hyphens"}},
          required: ["name"]
        }
      },
      %{
        name: "devices",
        read_only: true,
        description: "Lists your devices, assistants and browsers, and your passkeys",
        inputs: none
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
  def hooks, do: ["device"]

  @impl true
  def hook("device", %{"device" => device}, call) do
    with true <- device =~ ~r/^[0-9a-f]{64}$/,
         {:ok, who, _tag} <- Artifact.read(call.own, "devices/" <> device) do
      %{"who" => who}
    else
      _ -> %{}
    end
  end

  @impl true
  # Who invites is whoever may change what this plugin keeps for the network: its owners, and whoever they made
  # an editor of it.
  def call("invite", %{"name" => name}, call) do
    Call.reach(call, Artifact.name(call.own), "editor")
    %{"link" => link} = ask(%{ask: "invite", who: name})
    "users/#{name} joins at #{link} within a week. It's shown once, so pass it on now."
  end

  def call("devices", _inputs, call), do: ask(%{ask: "devices", who: person(call)})

  def call("remove_device", %{"device" => device}, call) do
    %{"removed" => name} = ask(%{ask: "remove_device", who: person(call), device: device})
    "#{name} is out, and is nobody's from now on."
  end

  def call("remove_passkey", %{"passkey" => passkey}, call) do
    %{"removed" => name} = ask(%{ask: "remove_passkey", who: person(call), passkey: passkey})
    "#{name} is removed."
  end

  # Devices are a person's, whoever a call runs for.
  defp person(%Call{by: "users/" <> _ = by}), do: by
  defp person(_call), do: raise("Devices are a person's, and nobody is asking.")

  # Asks this plugin's part at the network's address, as this server: it answers only the network's own.
  defp ask(what) do
    address =
      Application.get_env(:one_core, :address) ||
        raise "People sign in at a network's address, and this network has none."

    body = JSON.encode!(what)

    case Req.post!(address <> "/sign-in/ask", body: body, headers: Address.signed("/sign-in/ask", body), retry: false) do
      %{status: 200, body: %{"error" => why}} -> raise why
      %{status: 200, body: answer} -> answer
      %{status: status} -> raise "The network's address answered #{status}"
    end
  end
end
