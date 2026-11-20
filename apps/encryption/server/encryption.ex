defmodule Encryption do
  @moduledoc """
  Encryption: a kind of artifact whose blobs are encrypted before they're stored, so that whoever runs the bucket
  can't read them, and nor can a server its owner didn't choose.

  An encrypted artifact's key is its owner's to give. Their key is made on the network's sign-in page, the first
  time their passkey unlocks it, and kept only encrypted to their passkeys and the devices they let in, in an
  artifact this app makes for them, so no server ever holds it. A server opens an artifact once its owner's passkey
  handed it the artifact's keys there, encrypted to the server's own key, as the blobs `keys/<server>` in the
  artifact (`store`), with a relation tuple naming it. A server that holds none passes a call on to one that does,
  and reads nothing.

  What a device does is the client's (`apps/encryption/client/native/src/lib.rs`), and what each thing is encrypted with is
  `apps/encryption/client/native/src/cipher.rs`, which a server runs too (`Cipher`).
  """
  @behaviour OneCore.App

  # Every device's key is kept with it, and the Vault and notifications use it.
  @impl true
  def icon, do: "icon.svg"

  # It reads which apps run inside a call as `call.loaded` names them.
  @impl true
  def requires,
    do: %{
      "core" => ">= 0.32.202",
      "database" => "to keep when it last checked who holds your keys",
      "telemetry" => "to raise an alert while a server that acts for you is waiting for your key"
    }

  # Whether each server that should hold what's encrypted for someone does, said to Telemetry (`check`).
  @impl true
  def jobs, do: [%{tool: "check", every: 600}]

  # Nothing looks into it as it runs (`server-debug`): what it's given and keeps are keys, and what they decrypt.
  @impl true
  def sensitive, do: true

  # Whether a device of the app holds your key is a group on its page in Settings.
  @impl true
  def hooks, do: ["settings.sections"]

  @impl true
  def hook("settings.sections", _about, _call), do: [%{in: "device", component: "device", kinds: ["app"]}]

  alias Encryption.Keys
  alias OneCore.{Artifact, Call, Hooks, Server, Servers, User}
  alias OneCore.Apps.Calls

  @doc """
  `artifact`, encrypted: its blobs are encrypted as they're written and opened as they're read, by everything that
  uses what this gives. Only a server the artifact's owner chose gives it. Any other has a server that was chosen and
  acts for whose it is run the call instead, which ends here with that server's answer, and raises if none is up,
  saying which servers should hold its keys and don't (`access/1`).
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
        should = should(name)
        held = servers(artifact)
        # Only a server that both holds its keys and acts for whose it is runs the call. With none but this one, it
        # isn't passed on: one passed here already would come back as a loop, and say no server answered.
        others = Enum.filter(held, &(&1 in should)) -- [call.server]
        if others != [], do: :no_response = Call.forward(call, others)
        raise unopened(name, should, held)
    end
  end

  # Why nothing opens an encrypted artifact for a call: which servers that act for whose it is lack its keys, and which
  # hold them but don't act for them, so can't run the call.
  defp unopened(name, should, held) do
    owner = User.owner(name)
    open = Enum.filter(held, &(&1 in should))

    said =
      cond do
        held == [] -> "#{name} is encrypted, and no server holds #{whose(owner)} key for it yet."
        open == [] -> "#{name} is encrypted, and no server that acts for #{whom(owner)} holds #{whose(owner)} key for it."
        true -> "#{name} is encrypted, and no server that may open it is up: #{named(open)}."
      end

    Enum.join([said | gaps(owner, should -- held, held -- should)], " ")
  end

  # What's missing of an owner's servers holding an artifact's keys, as a person reads it, a sentence each: `missing`
  # act for them and wait for the keys, and `extra` hold them and don't act for them.
  defp gaps(owner, missing, extra) do
    Enum.map(missing, &"#{named([&1])} is waiting for #{whose(owner)} key.") ++
      Enum.map(extra, &"#{named([&1])} holds #{whose(owner)} key and no longer acts for #{whom(owner)}.")
  end

  defp named(servers), do: servers |> Enum.map(&(Servers.name(&1) || String.slice(&1, 0, 8))) |> Enum.join(", ")
  defp whom("network"), do: "the network"
  defp whom(owner), do: owner |> String.split("/") |> List.last()
  defp whose(owner), do: whom(owner) <> "'s"

  @doc """
  The keys of the servers that should hold an encrypted artifact's keys: those acting for whose it is, as scopes nest,
  their own first, then a group's they're in, then the network's (`OneCore.Apps.covering/1`). They're who run its
  calls, so one that lacks them can't, and one that holds them and isn't here has access it doesn't need.
  """
  def should(name), do: OneCore.Apps.covering(User.owner(name))

  @doc """
  How each artifact encrypted for `owner` stands: its epoch, the servers that hold its keys, those that should and don't
  (`missing`), and those that hold them and shouldn't (`extra`), each with what its machine is called, and each of
  those as a person reads it (`problems`). An artifact is known once a server that should hold its keys ran a call of it (`Encryption.Keys`).
  """
  def access(owner) do
    for name <- Artifact.list(index(owner)) do
      held = servers(%Artifact{prefix: name <> "/"})
      should = should(name)
      {missing, extra} = {should -- held, held -- should}

      %{
        artifact: name,
        epoch: epoch(%Artifact{prefix: name <> "/"}),
        servers: names(held),
        missing: names(missing),
        extra: names(extra),
        problems: gaps(owner, missing, extra)
      }
    end
  end

  defp names(servers), do: for(server <- servers, do: %{key: server, name: Servers.name(server)})

  @doc "Where the artifacts encrypted for `owner` are listed: an empty blob each, named by the artifact."
  def index(owner), do: %Artifact{prefix: "#{owner}/made/encryption/encrypted/"}

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
  that act for whose it is (`should/1`),
  each with what its machine is called.
  """
  def unlocked_on(%Call{} = _call, %Artifact{} = artifact) do
    Map.merge(
      %{
        artifact: Artifact.name(artifact),
        epoch: epoch(artifact),
        servers: servers(artifact),
        may: names(should(Artifact.name(artifact)))
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

  # Makes the servers that open an artifact the ones its owner's passkey handed keys on the sign-in page (`store`):
  # `given` is the artifact's keys encrypted to each, with its owner's signature of which. Each server newly chosen gets a
  # relation tuple naming it as an `editor`, and each one taken off loses its tuple and its keys. Only whoever the
  # artifact belongs to chooses: the call is theirs, and by them.
  defp choose(%Call{} = call, given) do
    with %{"artifact" => name, "epoch" => epoch, "was" => was, "slots" => %{} = slots} = given when is_binary(name) <- given,
         %{"signature" => signature, "signer" => signer} when is_binary(signature) and is_binary(signer) <- given,
         true <- is_integer(was) and epoch in [was, was + 1] do
      artifact = %Artifact{prefix: name <> "/"}
      owner!(call, name)

      if was != epoch(artifact),
        do: raise("Which servers open #{name} was changed from another device just now. Look, and do this again.")

      had = servers(artifact)

      for server <- Map.keys(slots) -- had, do: share(call, "share", name, server)
      for {server, slot} <- slots, do: put(artifact, "keys/" <> server, Base.decode64!(slot))
      put(artifact, "keys/epoch", Integer.to_string(epoch))
      put(artifact, "keys/signed", JSON.encode!(%{signature: signature, signer: signer}))

      for server <- had -- Map.keys(slots) do
        Artifact.remove(artifact, "keys/" <> server)
        share(call, "unshare", name, server)
      end

      GenServer.call(Call.process(call, Keys, name, {artifact, call.server}), :look)
      look(call, name, Enum.uniq(had ++ Map.keys(slots)))
      %{artifact: name, servers: Map.keys(slots)}
    else
      _ -> raise "That isn't an artifact's keys as the sign-in page encrypts them with your passkey."
    end
  end

  # Has each of `servers` look at what it's handed of an artifact now, so one that was taken off lets go of its
  # keys at once, and one that was handed a new epoch's writes with them. One that isn't up looks when it starts.
  defp look(%Call{} = call, name, servers) do
    {at, app} = OneCore.Apps.installed(call.subject, call.app_artifact)
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

    server = %{
      type: "object",
      properties: %{key: %{type: "string"}, name: %{type: ["string", "null"], description: "What its machine is called"}},
      required: ["key", "name"]
    }

    access = %{
      type: "array",
      description: """
      Each artifact encrypted for you, and which servers hold your key for it: every server that acts for you
      should, as it runs what uses it, and none other. One that lacks it is waiting for it, and its calls of that
      artifact fail until it has it.
      """,
      items: %{
        type: "object",
        properties: %{
          artifact: %{type: "string"},
          servers: %{type: "array", items: server, description: "The servers that hold it"},
          missing: %{type: "array", items: server, description: "Servers that act for you and are waiting for it"},
          extra: %{type: "array", items: server, description: "Servers that hold it and don't act for you"},
          problems: %{type: "array", items: %{type: "string"}, description: "Each of those, as a person reads it"}
        },
        required: ["artifact", "servers", "missing", "extra", "problems"]
      }
    }

    [
      %{
        name: "key",
        read_only: true,
        description: "Says which of your devices are trusted, which want to be, what this device holds of your key, and which servers hold the keys of what's encrypted for you, and which should",
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
            asking: %{type: "array", items: named, description: "The devices asking to"},
            wrapped: %{
              type: ["string", "null"],
              description: "A group's or the network's key, encrypted to your own, which your passkey opens on the sign-in page"
            },
            access: access
          },
          required: ["owner", "device", "check", "slot", "devices", "asking", "wrapped", "access"]
        }
      },
      %{
        name: "check",
        description: """
        Says to Telemetry how what's encrypted for you stands, as the clock does every 10 minutes: an alert is open
        while a server that acts for you is waiting for your key, or one that doesn't holds it. For the network, its
        owners are notified once when a server starts waiting (`sign-in`'s `check`). Answers as `key`'s `access`.
        """,
        inputs: %{type: "object", properties: %{}},
        outputs: %{type: "object", properties: %{access: access}, required: ["access"]}
      },
      %{
        name: "store",
        description: """
        Stores what the network's sign-in page encrypted with your passkey, which nothing else can make: your new key's
        check, as it's first made (`check`), your key for a
        device of yours (`device`), the keys of what's encrypted for you for the servers that act for you
        (`artifacts`), or a group's or the network's key encrypted to your own (`wrapped`). Only the sign-in page calls it.
        """,
        inputs: %{
          type: "object",
          properties: %{
            check: %{type: "string", description: "What tells your key from another, as it's first made"},
            device: %{
              type: "object",
              properties: %{key: %{type: "string"}, name: %{type: "string"}, check: %{type: "string"}, slot: %{type: "string"}},
              required: ["key", "name", "check", "slot"]
            },
            wrapped: %{
              type: "object",
              properties: %{check: %{type: "string"}, slot: %{type: "string"}},
              required: ["check", "slot"]
            },
            artifacts: %{type: "array", items: %{type: "object"}}
          }
        },
        outputs: %{
          type: "object",
          properties: %{
            device: %{type: ["string", "null"]},
            wrapped: %{type: "boolean"},
            artifacts: %{
              type: "array",
              items: %{
                type: "object",
                properties: %{artifact: %{type: "string"}, servers: %{type: "array", items: %{type: "string"}}},
                required: ["artifact", "servers"]
              }
            }
          },
          required: ["device", "wrapped", "artifacts"]
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
    wrapped = with {:ok, wrapped, _tag} <- Artifact.read(key, "owners/" <> call.actor), do: wrapped, else: (:absent -> nil)

    %{
      owner: call.subject,
      device: inputs["device"],
      check: check,
      slot: slot,
      devices: held,
      asking: asking,
      wrapped: wrapped,
      access: access(call.subject)
    }
  end

  # Each artifact as a record Telemetry keeps, whose alert is open while it says what's missing (`Telemetry.Alerts`).
  def call("check", _inputs, call) do
    access = access(call.subject)

    records =
      for artifact <- access do
        outcome = if artifact.problems == [], do: "complete", else: "incomplete"
        attributes = %{"one.artifact" => artifact.artifact, "one.outcome" => outcome, "one.problem" => Enum.join(artifact.problems, " ")}
        %{"name" => "one.encryption.access", "attributes" => Map.reject(attributes, &match?({_, ""}, &1))}
      end

    if records != [], do: Call.tool(call, "telemetry", "record", %{"records" => records})
    # The network's owners are told once when a server starts waiting for its key, by Sign-In, whose Sign In & Security
    # hands it over, so its notification opens there.
    if call.subject == "network", do: Call.tool(call, "sign-in", "check", %{})
    %{access: access}
  end

  # What the sign-in page encrypted with a passkey: each part is checked against the key it says it's of, by the key's
  # `check`, so nothing but a holder of that key makes one that's kept.
  def call("store", inputs, call) do
    key = key(owner!(call, call.subject))
    if check = inputs["check"], do: checked!(key, check)

    device =
      with %{"key" => device, "name" => name, "check" => check, "slot" => slot} <- inputs["device"] do
        if not (device =~ ~r/^[0-9a-f]{64}$/), do: raise("A device is named by its key.")
        checked!(key, check)
        Artifact.remove(key, "devices/" <> device)
        :ok = Artifact.write(key, "devices/" <> device, JSON.encode!(%{name: name, slot: slot}))
        Artifact.remove(key, "asking/" <> device)
        device
      end

    wrapped =
      with %{"check" => check, "slot" => slot} <- inputs["wrapped"] do
        if String.starts_with?(call.subject, "users/"), do: raise("Your own key is encrypted to your passkeys, not to itself.")
        with {:ok, held, _tag} when held != check <- Artifact.read(key, "check"), do: raise("That isn't #{whose(call.subject)} key.")
        if Artifact.read(key, "check") == :absent, do: raise("#{whom(call.subject)} has no key yet.")
        Artifact.remove(key, "owners/" <> call.actor)
        :ok = Artifact.write(key, "owners/" <> call.actor, slot)
        true
      end

    %{device: device, wrapped: wrapped == true, artifacts: Enum.map(inputs["artifacts"] || [], &choose(call, &1))}
  end

  @on_a_device "A device is approved from a device: in the app there, or with `one` on it. An assistant somewhere else has none."

  # The first key written is the owner's key from then on: what a device hands over is kept only if it's that one.
  def call("approve", %{"device" => device, "key" => given} = inputs, call) do
    key = key(owner!(call, call.subject))

    with {:ok, %{"check" => check, "slot" => slot}} when is_binary(check) and is_binary(slot) <- JSON.decode(given),
         true <- device =~ ~r/^[0-9a-f]{64}$/ do
      checked!(key, check)

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

  # The first key's check written is the owner's from then on: what's handed over is kept only if it's that one's.
  defp checked!(key, check) do
    Artifact.write(key, "check", check)

    if Artifact.read(key, "check") |> elem(1) != check,
      do: raise("That isn't your key: another was made before it. Use your passkey that unlocks it, or your recovery key.")
  end

  defp devices(key, prefix) do
    for name <- Artifact.list(key, prefix), {:ok, device, _tag} <- [Artifact.read(key, name)] do
      {String.replace_prefix(name, prefix, ""), JSON.decode!(device)}
    end
  end
end
