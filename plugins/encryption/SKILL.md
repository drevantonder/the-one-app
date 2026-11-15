---
name: encryption
description: Read before a plugin keeps something that whoever stores the network's data mustn't read, and when a call says something is encrypted and no server may open it, or that a device doesn't hold someone's key.
---

Encryption keeps an artifact's blobs encrypted in the bucket, so whoever runs the bucket can't read them, and nor
can a server its owner didn't choose. The Vault keeps its items this way.

- Someone's key is made on a device of theirs, and stays on their devices: `let_in`, on a device, makes it the
  first time, and after that lets in another device, by the key `key` lists it under. A device that isn't in yet
  asks with `ask`, and a device that is lets it in. No server ever holds the key, and signing in gives none.
- A server opens something encrypted only once its owner chose it, from a device that holds their key: the
  plugin that keeps it has a tool for that, as the Vault's `grant`. Choosing writes a relation tuple naming the
  server as an `editor` of the artifact, and hands the server that artifact's keys. Taking a server off removes
  both, and what's written after is under a key it was never handed. What it read before, it read.
- The network's owner can write that tuple for any server, and it opens nothing: only a device that holds the
  key hands one over. Nobody asks for a user here but the user: not their groups, not the owner.
- When every device that held the key is gone, its person lets a new one in with their recovery key, which the
  first device showed them: in the app, or as `let_in`'s `key`. Without it, or a device that's in, nobody is
  let in again. A recovery key is a secret: never ask for one in a chat, and never repeat one.
- "No server may open it yet" means its owner hasn't chosen one. "No server that may open it is up" means the
  ones they chose aren't running. Neither is fixed from a server.
- An assistant that reaches the network over the web has no device, so it lets nothing in and chooses no
  server: its person does that in the app, or an agent with `one` on a device of theirs.

A plugin keeps something encrypted by using this plugin:

    def uses, do: ["encryption"]

    def call("keep", %{"text" => text}, call) do
      encryption = OneCore.Call.plugin(call, "encryption")
      notes = encryption.encrypted(call, OneCore.Call.make(call, "notes"))
      OneCore.Artifact.write(notes, "note", encryption.secret(call, text))
      "Kept."
    end

- `encrypted(call, artifact)` gives the artifact, encrypted: `OneCore.Artifact` reads and writes it as any
  other, and what reaches the bucket is encrypted. On a server that wasn't chosen, the call ends there and a
  server that was runs it instead, so nothing after `encrypted` runs where the artifact can't be opened. A
  blob's name and size aren't hidden, so name blobs by what says nothing. The names under `keys/` are this
  plugin's.
- A database is encrypted by giving it the artifact: `database.transaction(%{call | own: notes, folder:
  Path.join(OneCore.Server.home(), OneCore.Artifact.name(notes))}, schema, fun)`. A server that wasn't chosen
  never opens it, so it takes no lease. The server writing it keeps a working copy on its own disk, as it does
  of any database. A plugin's scheduled jobs are kept in its own artifact's database, which then isn't the one
  to encrypt: keep what's encrypted in an artifact the plugin makes.
- What a person types that's secret is encrypted on their device, by an input that says so: `%{type: "string",
  format: "secret", opened_by: "opened_by"}`, where `opened_by` names a read-only tool of the plugin that
  answers `encryption.opened_by(call, artifact)`. The app, `one` and an agent's MCP on a device all encrypt it
  before it's sent, and `encryption.secret(call, input)` opens it on a chosen server. So no hook, log or other
  server reads it. What arrives as it was typed is refused.
- Choosing servers is a tool of the plugin's that takes `servers`, their keys, and an input the device fills
  in, `%{type: "string", format: "artifact-keys", opened_by: "opened_by"}`, and calls `encryption.choose(call,
  artifact, keys)`. `servers(artifact)` lists the ones chosen.
- Never answer with what's encrypted, raise it or log it: a tool's answer goes to agents and into what clients
  keep.
