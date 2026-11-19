---
name: live-network
description: Use before you run the installed `one`, or anything else, against the owner's live network rather than one you started yourself: what you may and may not do there.
---

The installed `one` (and the One App's MCP tools) act on the owner's live network, as the owner. The machines here
run it alongside your work: never install over the installed `one`, stop it, or run it with your own build. For
anything you build, use a network of your own (`local-network`).

## You may

- Read: `one builds`, `one --help`, `ONE_SUBJECT=network one code --name <app>`, an app's read-only tools.
- Release your own change, once it's on `main`, by the smallest path (`release`).

## You may not, without the owner saying so

- Delete or rewrite anything stored: blobs, databases, Recently Deleted, backups.
- Remove an app, a server, a device or a person.
- Touch the Vault: no `vault set`, `use`, `grant` or `remove`, and never read a secret.
- Send anything outside: email, messages, notifications to people.

The network's address, its servers' names and where its secrets are kept are in Agent Memory, not the repo. Never
write them into a commit. Never print the client's own files (under `~/.local/share/app.theone.client` or its config folder) to find
them: they hold this device's token.

## Mid-update or broken?

`one builds` lists each server with its `build`, whether it's `server_ready` (`server_membership` active and
`server_daemon_status` running) or its `server_ready_reason` (`joining`, `starting`, `draining`, or `stopped` with
`last_seen`), its `build` and the `runtime` it runs on, and its `server_update_status`: `downloading`, whose
`server_update_stage` is `checking` once it's downloaded (the build is started to check it), then `checked`, waiting
for the network's update to say go, then `verifying`, whose stage is `starting` the new build and `one` in its place
(it hands its work to another and stops), and `up_to_date` once they're kept. If they don't answer, it goes back to the pair before by
itself and says why in `went_back`: read its log, not the network's. A
server neither ready nor coming back after a few minutes is broken; say so, and don't release over it.
