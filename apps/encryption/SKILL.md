---
name: encryption
description: Read before an app keeps something that whoever stores the network's data mustn't read, and when a call says something is encrypted and no server may open it, or that a device doesn't hold someone's key.
metadata:
  title: "Encryption"
  subtitle: "Data only you can read"
  about: "Encrypts what your apps keep, so whoever stores your network's data can't read it. Your Vault is kept this way."
  category: Admin
---

Encryption keeps an artifact's blobs encrypted in the bucket, so whoever runs the bucket can't read them, and nor
can a server its owner didn't choose. The Vault keeps its items this way.

Its words:

- **Encrypted artifact**: an artifact whose blobs are encrypted before they're stored, so only holders of its key read
  it. For passwords and the keys that reach devices. Avoid: sealed.

- Someone's key is made on a device of theirs, and stays on their devices: `approve`, on a device, makes it the
  first time, and after that lets in another device, by the key `key` lists it under. A device that isn't in yet
  asks with `ask`, and a device that is lets it in. No server ever holds the key.
- A server opens something encrypted only once its owner chose it, from a device that holds their key: the
  app that keeps it has a tool for that, as the Vault's `grant`. Choosing writes a relation tuple naming the
  server as an `editor` of the artifact, and hands the server that artifact's keys. Taking a server off removes
  both, and what's written after is under a key it was never handed. What it read before, it read.
- The network's owner can write that tuple for any server, and it opens nothing: only a device that holds the
  key hands one over. Nobody asks for a user here but the user: not their groups, not the owner.
- A passkey that has WebAuthn's PRF lets a new device in too, with no other device: Unlock with Passkey. Its PRF
  output, which only the device it's used on ever has, derives a key that a device that holds their key encrypts the
  key to, once its person allows that passkey on a device that holds the key, and the network's sign-in page opens it
  where the passkey's used and encrypts it to the new device, as `approve` does. Signing in with one on a new device lets it in at once. `sign-in`'s `encrypt_for_passkeys` and
  `unlock_with_passkey` do it from a device. A passkey without PRF says so in Sign In & Security, and the other ways
  stay.
- What that trusts: whoever can use your passkey can open your key, the same as whoever can sign in as you. The PRF
  output and the key it opens stay on the device, in the sign-in page; what's kept, with the passkey, is the key it
  derives and your key encrypted to it, which no server opens. Removing the passkey removes both. The PRF is asked
  with a salt of the network's own, so a passkey gives each network a different key. The page comes from the
  network's edge, so whoever can change the edge's code could take what a passkey opens there, as they could change
  the page you sign in on.
- When every device that held the key is gone, its person lets a new one in with their recovery key, which the
  first device showed them: in the app, or as `approve`'s `key`. Without it, or a device that's in, nobody is
  let in again. A recovery key is a secret: never ask for one in a chat, and never repeat one.
- Every server that acts for whose it is should hold their key for it, since any of them may run its calls, and no
  other should. `key`'s `access` says, for each artifact encrypted for someone, which servers hold it, which are
  waiting for it (`missing`) and which hold it and no longer act for them (`extra`), and Telemetry has an alert
  open while that's so (`check`, every 10 minutes). A call that no server can open says which are waiting. None
  of it is fixed from a server.
- An assistant that reaches the network over the web has no device, so it lets nothing in and chooses no
  server: its person does that in the app, or an agent with `one` on a device of theirs.

An app keeps something encrypted by requiring this app:

    def requires, do: %{"encryption" => "to keep your notes unreadable"}

    def call("keep", %{"text" => text}, call) do
      encryption = OneCore.Call.app(call, "encryption")
      notes = encryption.encrypted(call, OneCore.Call.make(call, "notes"))
      OneCore.Artifact.write(notes, "note", encryption.secret(call, text))
      "Kept."
    end

- `encrypted(call, artifact)` gives the artifact, encrypted: `OneCore.Artifact` reads and writes it as any
  other, and what reaches the bucket is encrypted. On a server that wasn't chosen, the call ends there and a
  server that was runs it instead, so nothing after `encrypted` runs where the artifact can't be opened. A
  blob's name and size aren't hidden, so name blobs by what says nothing. The names under `keys/` are this
  app's.
- A database is encrypted by giving it the artifact: `database.transaction(%{call | own: notes, folder:
  Path.join(OneCore.Server.home(), OneCore.Artifact.name(notes))}, schema, fun)`. A server that wasn't chosen
  never opens it, so it takes no lease. The server writing it keeps a working copy on its own disk, as it does
  of any database. An app's scheduled jobs are kept in its own artifact's database, which then isn't the one
  to encrypt: keep what's encrypted in an artifact the app makes.
- What a person types that's secret is encrypted on their device, by an input that says so: `%{type: "string",
  format: "secret", unlocked_on: "unlocked_on"}`, where `unlocked_on` names a read-only tool of the app that
  answers `encryption.unlocked_on(call, artifact)`, all of it: the list of servers its owner chose comes with
  their signature, which a device checks before it encrypts to them. The app, `one` and an agent's MCP on a device
  all encrypt it before it's sent, and `encryption.secret(call, input)` opens it on a chosen server. So no hook, log or other
  server reads it. What arrives as it was typed is refused.
- Choosing servers is a tool of the app's that takes `servers`, their keys, and an input the device fills
  in, `%{type: "string", format: "artifact-keys", unlocked_on: "unlocked_on"}`, and calls `encryption.choose(call,
  artifact, keys)`. `servers(artifact)` lists the ones chosen.
- Never answer with what's encrypted, raise it or log it: a tool's answer goes to agents and into what clients
  keep.
