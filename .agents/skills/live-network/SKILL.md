---
name: live-network
description: Use before you run the installed `one`, or anything else, against the owner's live network rather than one you started yourself: what you may and may not do there.
---

The installed `one` (and The One App's MCP tools) act on the owner's live network, as the owner. The machines here
run it alongside your work: never install over the installed `one`, stop it, or run it with your own build. For
anything you build, use a network of your own (`local-network`).

## You may

- Read: `one builds`, `one --help`, `ONE_FOR=network one code --name <plugin>`, a plugin's read-only tools.
- Release your own change, once it's on `main`, by the smallest path (`release`).

## You may not, without the owner saying so

- Delete or rewrite anything stored: blobs, databases, Recently Deleted, backups.
- Remove a plugin, a server, a device or a person.
- Touch the Vault: no `vault set`, `use`, `grant` or `remove`, and never read a secret.
- Send anything outside: email, messages, notifications to people.

The network's address, its servers' names and where its secrets are kept are in Agent Memory, not the repo. Never
write them into a commit.

## Mid-update or broken?

`one builds` lists each server with its `build` and `seen` (`"on"`, or when it was last seen). During
`one update --server <key>` the server hands its work to another, stops and starts on the new build: for a minute
or so it isn't `on`, and then shows the new `build`. If the new build doesn't start and answer, it goes back to the
old one by itself, so a server back `on` with its old build after an update was a failed update: read its log, not
the network's. A server neither `on` nor coming back after a few minutes is broken; say so, and don't release over
it.
