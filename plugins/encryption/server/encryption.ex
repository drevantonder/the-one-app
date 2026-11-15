defmodule Encryption do
  @moduledoc """
  Encryption: a kind of artifact whose blobs are encrypted before they're stored, so that whoever runs the bucket
  can't read them, and nor can a server its owner didn't choose.

  An encrypted artifact's key is its owner's to give. Their key is made on a device of theirs and kept only
  encrypted to the devices they let in, in an artifact this plugin makes for them, so no server ever holds it. A
  server opens an artifact once its owner chose it: a relation tuple names the server, and a device of theirs
  hands it that artifact's keys, encrypted to the server's own key, as the blobs `keys/<server>` in the artifact.
  A server that isn't chosen passes a call on to one that is, and reads nothing.

  What a device does is the client's (`plugins/encryption/client/native/src/lib.rs`), and what each thing is encrypted with is
  `plugins/encryption/client/native/src/cipher.rs`, which a server runs too (`Cipher`).
  """
  @behaviour OneCore.Plugin
  alias Encryption.Keys
  alias OneCore.{Artifact, Call, Hooks, Server, User}
  alias OneCore.Plugins.Calls

  @doc """
  `artifact`, encrypted: its blobs are encrypted as they're written and opened as they're read, by everything that
  uses what this gives. Only a server the artifact's owner chose gives it. Any other has a server that was chosen
  run the call instead, which ends here with that server's answer, and raises if none is up.
  """
  def encrypted(%Call{} = call, %Artifact{wrapped: nil} = artifact) do
    name = Artifact.name(artifact)
    keys = Call.process(call, Keys, name, {%{artifact | reached: nil}, call.server})

    with :ok <- chosen(call, name, call.server),
         {:ok, keys} <- GenServer.call(keys, :keys) do
      %{artifact | wrapped: {Cipher, keys}}
    else
      refused ->
        # A server that's no longer chosen lets go of the keys it held, whatever still holds them.
        with {:refuse, _why} <- refused, do: GenServer.call(keys, :forget)
        chosen = servers(artifact)
        :no_answer = Call.forward(call, chosen)

        if chosen == [],
          do:
            raise(
              "#{name} is encrypted, and no server may open it yet. Its owner chooses one, from a device that holds their key."
            ),
          else: raise("#{name} is encrypted, and no server that may open it is up.")
    end
  end

  # Whether the artifact's owner chose a server: a relation tuple names it as an `editor`, which the plugins that
  # decide `reach` say. One Core keeps their answer, so a call asks once.
  defp chosen(call, artifact, server) do
    about = %{"who" => "servers/" <> server, "relation" => "editor", "artifact" => artifact}
    Hooks.decide("reach", about, {call.by, call.for})
  end

  @doc "The keys of the servers that hold an artifact's keys."
  def servers(%Artifact{} = artifact) do
    for "keys/" <> server <- Artifact.list(%{artifact | wrapped: nil}, "keys/"),
        server =~ ~r/^[0-9a-f]{64}$/,
        do: server
  end

  @doc """
  What a device needs of an artifact to encrypt for it, for a plugin's read-only tool to answer: the artifact's
  name, its `epoch`, the `servers` that open it, by their keys, and the ones that `may` be chosen, which are those
  that run this call's plugin, each with what its machine is called.
  """
  def opened_by(%Call{} = call, %Artifact{} = artifact) do
    %{
      artifact: Artifact.name(artifact),
      epoch: epoch(artifact),
      servers: servers(artifact),
      may: for(server <- OneCore.Servers.acting(call.added), do: %{key: server, name: OneCore.Servers.name(server)})
    }
  end

  defp epoch(artifact) do
    case Artifact.read(%{artifact | wrapped: nil}, "keys/epoch") do
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
    artifact = %{artifact | wrapped: nil}
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

      for server <- had -- Map.keys(slots) do
        Artifact.remove(artifact, "keys/" <> server)
        share(call, "unshare", name, server)
      end

      GenServer.call(Call.process(call, Keys, name, {artifact, call.server}), :look)
      look(call, name, Enum.uniq(had ++ Map.keys(slots)))
      Map.keys(slots)
    else
      _ ->
        raise "That isn't an artifact's keys as a device hands them over. A device that holds your key fills them in."
    end
  end

  # Has each of `servers` look at what it's handed of an artifact now, so one that was taken off lets go of its
  # keys at once, and one that was handed a new epoch's writes with them. One that isn't up looks when it starts.
  defp look(%Call{uses: %{"encryption" => {encryption, _module}}} = call, name, servers) do
    {at, plugin} = OneCore.Plugins.added(call.for, encryption)
    said = %{who: nil, for: User.owner(name), forwarded: true}
    ask = %{added: at, plugin: plugin, function: "looked", arguments: [name], _meta: said}
    for server <- servers -- [call.server], do: Server.request(server, "plugins/call", ask)
  end

  @doc false
  # What another server asks of this one once the servers that open an artifact were chosen again.
  def looked(%Call{} = call, name) do
    GenServer.call(Call.process(call, Keys, name, {%Artifact{prefix: name <> "/"}, call.server}), :look)
  end

  # A relation tuple is the relations plugin's to write, as whoever asked.
  defp share(call, tool, artifact, server) do
    inputs = %{"artifact" => artifact, "relation" => "editor", "who" => "servers/" <> server}

    case Calls.call("relations", tool, inputs, {call.by, call.for}) do
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
        raise "That wasn't encrypted on a device for this server, so it isn't kept. A secret is typed on a device of yours, or given to `one` there: an assistant somewhere else asks its person for it instead."
    end
  end

  # What's encrypted for someone is theirs alone to hand keys for: a user's by that user, and a group's or the
  # network's by an owner of it. No relation tuple lets anyone else, the network's owner included.
  defp owner!(%Call{for: who, by: by} = call, artifact) do
    cond do
      User.owner(artifact) != who ->
        raise "#{artifact} isn't #{who}'s"

      String.starts_with?(who, "users/") and by != who ->
        raise "What's encrypted for #{who} is theirs alone to hand the keys of."

      String.starts_with?(who, "users/") ->
        :ok

      true ->
        with {:refuse, why} <-
               Hooks.decide("reach", %{"who" => by, "relation" => "owner", "artifact" => who}, {by, who}),
             do: raise(why)
    end

    call
  end

  @impl true
  def tools do
    device = %{type: "string", format: "device", description: "The device's key. This device's, if left out"}

    [
      %{
        name: "key",
        read_only: true,
        description: "Says which of your devices hold your key, which are asking to, and what this device holds of it",
        inputs: %{type: "object", properties: %{device: device}}
      },
      %{
        name: "let_in",
        description: """
        Lets a device hold your key: this device the first time, which makes the key, and then any other, from a
        device that holds it. With your recovery key as `key`, it lets in a device that isn't in yet.
        """,
        inputs: %{
          type: "object",
          properties: %{
            device: device,
            name: %{type: "string", format: "device-name", description: "What to call the device"},
            key: %{type: "string", format: "owner-key", description: "Your recovery key, to let this device in with it"}
          }
        }
      },
      %{
        name: "ask",
        description: "Asks to be let in: your devices that hold your key show this one, to let it in",
        inputs: %{
          type: "object",
          properties: %{device: device, name: %{type: "string", format: "device-name", description: "What to call it"}}
        }
      },
      %{
        name: "take_out",
        description: "Takes a device's copy of your key away, or its asking for one",
        inputs: %{type: "object", properties: %{device: device}, required: ["device"]}
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
    %{owner: call.for, device: inputs["device"], check: check, slot: slot, devices: held, asking: asking}
  end

  @on_a_device "A device is let in from a device: in the app there, or with `one` on it. An assistant somewhere else has none."

  # The first key written is the owner's key from then on: what a device hands over is kept only if it's that one.
  def call("let_in", %{"device" => device, "key" => given} = inputs, call) do
    key = key(call)

    with {:ok, %{"check" => check, "slot" => slot}} when is_binary(check) and is_binary(slot) <- JSON.decode(given),
         true <- device =~ ~r/^[0-9a-f]{64}$/ do
      Artifact.write(key, "check", check)

      if Artifact.read(key, "check") |> elem(1) != check,
        do:
          raise(
            "That isn't your key: one was made on another device. Let this one in from there, or with your recovery key."
          )

      name = inputs["name"] || "A device"
      Artifact.remove(key, "devices/" <> device)
      :ok = Artifact.write(key, "devices/" <> device, JSON.encode!(%{name: name, slot: slot}))
      Artifact.remove(key, "asking/" <> device)
      "#{name} holds your key."
    else
      _ -> raise @on_a_device
    end
  end

  def call("let_in", _inputs, _call), do: raise(@on_a_device)

  def call("ask", %{"device" => device} = inputs, call) do
    key = key(call)
    if not (device =~ ~r/^[0-9a-f]{64}$/), do: raise("A device is named by its key.")
    Artifact.write(key, "asking/" <> device, JSON.encode!(%{name: inputs["name"] || "A device"}))
    "Asked. A device that holds your key lets this one in."
  end

  def call("ask", _inputs, _call), do: raise(@on_a_device)

  def call("take_out", %{"device" => device}, call) do
    key = key(call)
    if not (device =~ ~r/^[0-9a-f]{64}$/), do: raise("A device is named by its key.")
    for kept <- ["devices/", "asking/"], do: Artifact.remove(key, kept <> device)
    "That device no longer holds your key."
  end

  # Where an owner's key is kept, encrypted to each of their devices. It's theirs alone: nobody asks for a user.
  defp key(%Call{for: who, by: by} = call) do
    if String.starts_with?(who, "users/") and by != who, do: raise("#{who}'s key is theirs alone.")
    Call.make(call, "key")
  end

  defp devices(key, kept) do
    for name <- Artifact.list(key, kept), {:ok, device, _tag} <- [Artifact.read(key, name)] do
      {String.replace_prefix(name, kept, ""), JSON.decode!(device)}
    end
  end
end
