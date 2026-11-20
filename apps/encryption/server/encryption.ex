defmodule Encryption do
  @moduledoc """
  Encryption: a kind of artifact whose blobs are encrypted before they're stored, so that whoever runs the bucket
  can't read them, and nor can a server its owner didn't choose.

  An encrypted artifact's key is its owner's to give. Their key is made on a device of theirs and kept only
  encrypted to the devices they let in, in an artifact this app makes for them, so no server ever holds it. A
  server opens an artifact once its owner chose it: a relation tuple names the server, and a device of theirs
  hands it that artifact's keys, encrypted to the server's own key, as the blobs `keys/<server>` in the artifact.
  A server that isn't chosen passes a call on to one that is, and reads nothing.

  What a device does is the client's (`apps/encryption/client/native/src/lib.rs`), and what each thing is encrypted with is
  `apps/encryption/client/native/src/cipher.rs`, which a server runs too (`Cipher`).
  """
  @behaviour OneCore.App

  # Every device's key is kept with it, and the Vault and notifications use it.
  @impl true
  def icon, do: "icon.svg"

  # It reads which apps run inside a call as `call.loaded` names them.
  @impl true
  def requires, do: %{"core" => ">= 0.32.202"}

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are keys, and what they decrypt.
  @impl true
  def sensitive, do: true

  # Whether a device of the app holds your key is a group on its page in Settings.
  @impl true
  def hooks, do: ["settings.sections"]

  @impl true
  def hook("settings.sections", _about, _call), do: [%{in: "device", component: "device", kinds: ["app"]}]

  alias Encryption.Keys
  alias OneCore.{Artifact, Call, Hooks, Server, User}
  alias OneCore.Apps.Calls

  @doc """
  `artifact`, encrypted: its blobs are encrypted as they're written and opened as they're read, by everything that
  uses what this gives. Only a server the artifact's owner chose gives it. Any other has a server that was chosen
  run the call instead, which ends here with that server's answer, and raises if none is up.
  """
  def encrypted(%Call{} = call, %Artifact{codec: nil} = artifact) do
    name = Artifact.name(artifact)
    keys = Call.process(call, Keys, name, {%{artifact | reached: nil}, call.server})

    with :ok <- chosen(call, name, call.server),
         {:ok, codec} <- GenServer.call(keys, :keys) do
      %{artifact | codec: codec}
    else
      refused ->
        # A server that's no longer chosen lets go of the keys it held, whatever still holds them.
        with {:refuse, _why} <- refused, do: GenServer.call(keys, :forget)
        chosen = servers(artifact)
        :no_response = Call.forward(call, chosen)

        if chosen == [],
          do:
            raise(
              "#{name} is encrypted, and no server may open it yet. Its owner chooses one, from a device that holds their key."
            ),
          else: raise("#{name} is encrypted, and no server that may open it is up.")
    end
  end

  # Whether the artifact's owner chose a server: a relation tuple names it as an `editor`, which the apps that
  # decide `reach` say. One Core keeps their answer, so a call asks once.
  defp chosen(call, artifact, server) do
    about = %{"who" => "servers/" <> server, "relation" => "editor", "artifact" => artifact}
    Hooks.validate("reach", about, {call.actor, call.subject})
  end

  @doc "The keys of the servers that hold an artifact's keys."
  def servers(%Artifact{} = artifact) do
    for "keys/" <> server <- Artifact.list(%{artifact | codec: nil}, "keys/"),
        server =~ ~r/^[0-9a-f]{64}$/,
        do: server
  end

  @doc """
  What a device needs of an artifact to encrypt for it, for an app's read-only tool to answer: the artifact's
  name, its `epoch`, the `servers` that open it, by their keys, and the ones that `may` be chosen, which are those
  that cover whose it is, their own first, then a group's they're in, then the network's (`OneCore.Apps.covering/1`), each with what its machine is called.
  """
  def unlocked_on(%Call{} = call, %Artifact{} = artifact) do
    Map.merge(
      %{
        artifact: Artifact.name(artifact),
        epoch: epoch(artifact),
        servers: servers(artifact),
        may:
          for(
            server <- OneCore.Apps.covering(OneCore.User.owner(Artifact.name(artifact))),
            do: %{key: server, name: OneCore.Servers.name(server)}
          )
      },
      signed(artifact)
    )
  end

  # Its owner's signature of which servers open it at its epoch, and the public key that signed it, as the device that
  # chose them gave them: a device checks it before it encrypts to them (`device.rs`). A server can't make one.
  defp signed(artifact) do
    case Artifact.read(%{artifact | codec: nil}, "keys/signed") do
      {:ok, json, _tag} -> Map.take(JSON.decode!(json), ["signature", "signer"])
      :absent -> %{}
    end
  end

  defp epoch(artifact) do
    case Artifact.read(%{artifact | codec: nil}, "keys/epoch") do
      {:ok, epoch, _tag} -> String.to_integer(epoch)
      :absent -> 0
    end
  end

  @doc """
  Makes the servers that open an artifact the ones its owner's device handed keys: `given` is what the device
  filled in for an input whose format is `artifact-keys`. Each server newly chosen gets a relation tuple naming it
  as an `editor`, and each one taken off loses its tuple and its keys. Only whoever the artifact belongs to
  chooses: the call is theirs, and by them.
  """
  def choose(%Call{} = call, %Artifact{} = artifact, given) do
    artifact = %{artifact | codec: nil}
    name = Artifact.name(artifact)
    owner!(call, name)

    with {:ok, %{"artifact" => ^name, "epoch" => epoch, "was" => was, "slots" => %{} = slots}} <- JSON.decode(given),
         true <- is_integer(was) and epoch in [was, was + 1] do
      if was != epoch(artifact),
        do: raise("Which servers open #{name} was changed from another device just now. Look, and do this again.")

      had = servers(artifact)

      for server <- Map.keys(slots) -- had, do: share(call, "share", name, server)
      for {server, slot} <- slots, do: put(artifact, "keys/" <> server, Base.decode64!(slot))
      put(artifact, "keys/epoch", Integer.to_string(epoch))

      # A device from before lists were signed signs none, and the signature of a list before isn't this one's.
      case JSON.decode!(given) do
        %{"signature" => signature, "signer" => signer} when is_binary(signature) and is_binary(signer) ->
          put(artifact, "keys/signed", JSON.encode!(%{signature: signature, signer: signer}))

        _ ->
          Artifact.remove(artifact, "keys/signed")
      end

      for server <- had -- Map.keys(slots) do
        Artifact.remove(artifact, "keys/" <> server)
        share(call, "unshare", name, server)
      end

      GenServer.call(Call.process(call, Keys, name, {artifact, call.server}), :look)
      look(call, name, Enum.uniq(had ++ Map.keys(slots)))
      Map.keys(slots)
    else
      _ ->
        raise "That isn't an artifact's keys as a device hands them over. A trusted device fills them in."
    end
  end

  # Has each of `servers` look at what it's handed of an artifact now, so one that was taken off lets go of its
  # keys at once, and one that was handed a new epoch's writes with them. One that isn't up looks when it starts.
  defp look(%Call{loaded: %{"encryption" => {encryption, _module}}} = call, name, servers) do
    {at, app} = OneCore.Apps.installed(call.subject, encryption)
    meta = %{actor: nil, subject: User.owner(name), forwarded: true}
    ask = %{installed: at, app: app, function: "looked", arguments: [name], _meta: meta}
    for server <- servers -- [call.server], do: Server.request(server, "apps/call", ask)
  end

  @doc false
  # What another server asks of this one once the servers that open an artifact were chosen again.
  def looked(%Call{} = call, name) do
    GenServer.call(Call.process(call, Keys, name, {%Artifact{prefix: name <> "/"}, call.server}), :look)
  end

  # A relation tuple is the permissions app's to write, as whoever asked.
  defp share(call, tool, artifact, server) do
    inputs = %{"artifact" => artifact, "relation" => "editor", "who" => "servers/" <> server}

    case Calls.call("permissions", tool, inputs, {call.actor, call.subject}) do
      {:ok, _said} -> :ok
      {:forwarded, %{"isError" => true, "content" => [%{"text" => why} | _]}} -> raise why
      {:forwarded, _result} -> :ok
      {:error, why} -> raise why
    end
  end

  defp put(artifact, name, data) do
    wrote =
      case Artifact.read(artifact, name) do
        {:ok, _was, tag} -> Artifact.replace(artifact, name, data, tag)
        :absent -> Artifact.write(artifact, name, data)
      end

    if wrote != :ok,
      do: raise("#{Artifact.name(artifact)}'s keys were changed from another device just now. Look, and do this again.")
  end

  @doc """
  What a person typed on their device, which encrypted it for the servers that open what it's kept in: `given` is
  what the device filled in for an input whose format is `secret`. Raises if it wasn't encrypted for this server.
  """
  def secret(%Call{server: server}, given) do
    with {:ok, %{^server => sealed}} <- JSON.decode(given),
         {:ok, sealed} <- Base.decode64(sealed),
         {:ok, own} <- Cipher.key(Server.key_file()),
         {:ok, secret} <- Cipher.open(own, "one/encryption/1/secret\n#{server}", sealed) do
      secret
    else
      _ ->
        raise "That wasn't encrypted on a device for this server, so it isn't kept.\n\nA secret is typed on a device of yours, or given there to `one`'s own tool, never in code mode. An assistant in code mode or anywhere else asks its person for it with `vault_request`, which has them type it on their device."
    end
  end

  # What's encrypted for someone is theirs alone to hand keys for: a user's by that user, and a group's or the
  # network's by an owner of it. No relation tuple lets anyone else, the network's owner included.
  defp owner!(%Call{subject: subject, actor: actor} = call, artifact) do
    cond do
      User.owner(artifact) != subject ->
        raise "#{artifact} isn't #{subject}'s"

      String.starts_with?(subject, "users/") and actor != subject ->
        raise "What's encrypted for #{subject} is theirs alone to hand the keys of."

      String.starts_with?(subject, "users/") ->
        :ok

      true ->
        with {:refuse, why} <-
               Hooks.validate(
                 "reach",
                 %{"who" => actor, "relation" => "owner", "artifact" => subject},
                 {actor, subject}
               ),
             do: raise(why)
    end

    call
  end

  @impl true
  def tools do
    device = %{type: "string", format: "device", description: "The device's key. This device's, if left out"}

    named = %{
      type: "object",
      properties: %{device: %{type: "string", description: "The device's key"}, name: %{type: "string"}},
      required: ["device", "name"]
    }

    [
      %{
        name: "key",
        read_only: true,
        description: "Says which of your devices are trusted, which want to be, and what this device holds of your key",
        inputs: %{type: "object", properties: %{device: device}},
        outputs: %{
          type: "object",
          properties: %{
            owner: %{type: "string", description: "Whose key"},
            device: %{type: ["string", "null"], description: "The device asked about"},
            check: %{type: ["string", "null"], description: "What tells the key from another, once there's one"},
            slot: %{
              type: ["string", "null"],
              description: "The key, encrypted to the device asked about, if it holds it"
            },
            devices: %{type: "array", items: named, description: "The devices that hold it"},
            asking: %{type: "array", items: named, description: "The devices asking to"}
          },
          required: ["owner", "device", "check", "slot", "devices", "asking"]
        }
      },
      %{
        name: "approve",
        description: """
        Lets a device hold your key: this device the first time, which makes the key, and then any other, from a
        device that holds it. With your recovery key as `key`, it lets in a device that isn't in yet.
        """,
        inputs: %{
          type: "object",
          properties: %{
            device: device,
            name: %{type: "string", format: "device-name", description: "What to call the device"},
            key: %{
              type: "string",
              format: "owner-key",
              description: "Your recovery key, to approve this device with it"
            }
          }
        },
        outputs: named
      },
      %{
        name: "ask",
        description: "Asks to be trusted: your trusted devices show this one, to approve it",
        inputs: %{
          type: "object",
          properties: %{device: device, name: %{type: "string", format: "device-name", description: "What to call it"}}
        },
        outputs: %{type: "object", properties: %{device: %{type: "string"}}, required: ["device"]}
      },
      %{
        name: "revoke",
        description: "Takes a device's copy of your key away, or its asking for one",
        inputs: %{type: "object", properties: %{device: device}, required: ["device"]},
        outputs: %{type: "object", properties: %{device: %{type: "string"}}, required: ["device"]}
      }
    ]
  end

  @impl true
  def call("key", inputs, call) do
    key = key(call)
    held = for {device, %{"name" => name}} <- devices(key, "devices/"), do: %{device: device, name: name}
    asking = for {device, %{"name" => name}} <- devices(key, "asking/"), do: %{device: device, name: name}

    slot =
      with device when is_binary(device) <- inputs["device"],
           {:ok, held, _tag} <- Artifact.read(key, "devices/" <> device),
           do: JSON.decode!(held)["slot"],
           else: (_ -> nil)

    check = with {:ok, check, _tag} <- Artifact.read(key, "check"), do: check, else: (:absent -> nil)
    %{owner: call.subject, device: inputs["device"], check: check, slot: slot, devices: held, asking: asking}
  end

  @on_a_device "A device is approved from a device: in the app there, or with `one` on it. An assistant somewhere else has none."

  # The first key written is the owner's key from then on: what a device hands over is kept only if it's that one.
  def call("approve", %{"device" => device, "key" => given} = inputs, call) do
    key = key(owner!(call, call.subject))

    with {:ok, %{"check" => check, "slot" => slot}} when is_binary(check) and is_binary(slot) <- JSON.decode(given),
         true <- device =~ ~r/^[0-9a-f]{64}$/ do
      Artifact.write(key, "check", check)

      if Artifact.read(key, "check") |> elem(1) != check,
        do:
          raise(
            "That isn't your key: one was made on another device. Approve this one from there, or with your recovery key."
          )

      name = inputs["name"] || "A device"
      Artifact.remove(key, "devices/" <> device)
      :ok = Artifact.write(key, "devices/" <> device, JSON.encode!(%{name: name, slot: slot}))
      Artifact.remove(key, "asking/" <> device)
      %{device: device, name: name}
    else
      _ -> raise @on_a_device
    end
  end

  def call("approve", %{"device" => _}, _call),
    do:
      raise(
        "Only a device that holds your key lets another hold it. Approve this one from there, or with your recovery key."
      )

  def call("approve", _inputs, _call), do: raise(@on_a_device)

  def call("ask", %{"device" => device} = inputs, call) do
    key = key(call)
    if not (device =~ ~r/^[0-9a-f]{64}$/), do: raise("A device is named by its key.")
    Artifact.write(key, "asking/" <> device, JSON.encode!(%{name: inputs["name"] || "A device"}))
    %{device: device}
  end

  def call("ask", _inputs, _call), do: raise(@on_a_device)

  def call("revoke", %{"device" => device}, call) do
    key = key(owner!(call, call.subject))
    if not (device =~ ~r/^[0-9a-f]{64}$/), do: raise("A device is named by its key.")
    for prefix <- ["devices/", "asking/"], do: Artifact.remove(key, prefix <> device)
    %{device: device}
  end

  # Where an owner's key is kept, encrypted to each of their devices. It's theirs alone: nobody asks for a user.
  # Who holds a group's or the network's key is its owners' to say, as which servers open what's theirs is
  # (`owner!/2`): approve and revoke ask that first. Anyone it's for reads what their own device holds.
  defp key(%Call{subject: subject, actor: actor} = call) do
    if String.starts_with?(subject, "users/") and actor != subject, do: raise("#{subject}'s key is theirs alone.")
    Call.make(call, "key")
  end

  defp devices(key, prefix) do
    for name <- Artifact.list(key, prefix), {:ok, device, _tag} <- [Artifact.read(key, name)] do
      {String.replace_prefix(name, prefix, ""), JSON.decode!(device)}
    end
  end
end
