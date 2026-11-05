# Architecture

Your devices, and your team's, as one place for apps and agents. An install is one person's or one workplace's: its people, their devices, their apps, and a bucket (a folder or S3-compatible storage) holding everything, sealed. Every machine in it runs the same program; the kernel, a few small Workers on Cloudflare, only routes, signs in and relays. This page holds only what the code can't say: where to look, and the rules that span many files. How a piece works is in its module doc (`//!` at the top of each file in `src/`, a comment at the top of each Worker, `client/`, `serverless/` and `services/`) and its tests.

## Primitives

Key, log, view, blob, call, app. Scopes, devices, deploys, services, the Vault and backups are made of them.
- **Key:** every device has its own key, never shared. Each scope (a person's, a group's, or the install's `global`) has numbered epochs of keys, sealed to the devices that may open it and to the install's recovery key.
- **Log:** who can do what is signed entries kept in the bucket, replayed in causal order on every device. A change counts by who signed it, never by the time it claims.
- **Blob:** bytes an entry names, kept in the bucket sealed with their scope's key.
- **Call:** runs once, on a node that runs every scope it reaches, chosen by run priority.
- **App:** deployed code. Its data in a scope is one SQLite database, the truth, kept sealed in the bucket as steps at numbered positions; a write is a step created only if its position is free, so the bucket gives the order. How things went (usage, errors, runs, builds) is the node's own database of a scope, changed in place.

## Trust

- **Keys are opened only on clients**, the devices of people and their agents (apps, browsers, the CLI, the MCP Worker). **Nodes** run calls and hold no keys: a caller hands a node the database keys one call needs, which expire with its sign-in. The only exceptions are standing work and the Vault, where a person or an admin chooses a machine to hold keys and can take them back.
- **The kernel opens nothing.** A hacked node gets only the calls it ran while it was hacked; a hacked kernel reads no data. The Workers that serve browsers' code could take what browsers open, which is why they stay small, and why the Cloudflare account is guarded with a hardware security key.
- **Clients take who's who from the signed entries themselves**, never from what a node or the relay says, and seal or hand keys only to whom they list.
- **Signing in proves who you are, nothing more.** It never opens a key. A new device gets in only when one of its person's devices approves it.
- **One install, forever.** A key belongs to one install; installs never talk to each other.
- **Secrets stay put:** a Vault item's secret leaves a node only inside a request to one of its hosts, by an app switched on for it. Nothing reads one back.
- **Keys decide access; the relay decides who's on the network.** It admits only the install's endpoints, a list devices change by signed tells.
- **Only the URL signer holds the bucket's credentials;** everything in the bucket is sealed, so what it hands out opens nothing.

## Rules that span files

- **The core never names an app or service.** Apps are deployed into the install while it runs; services run only where a device's owner turned them on. `config.toml` is the one place that names the product (the white label).
- **Devices reach each other over iroh,** each an endpoint of its own key, through the relay at the install's address when no direct path opens. A device answers only the install's devices.
- **The bucket decides the order of writes.** Notices between devices are hints; each device reads what changed from the bucket.
- **An older device can safely ignore what newer ones send.** Phones and Macs run behind the others.
- **Speed** is held to targets measured in `one try`, from a phone on mobile data where it matters: the web client usable within 1.5 s of a cold open; a remote MCP call adding at most 100 ms to a local one; a write seen on another device within 1 s; a call one round trip; no data rewritten when someone adds a device.

## Designed, not built

- One storage model, and permissions as data: the plan under way is `docs/plan-storage-and-permissions.md`.
- One key per scope at a time, changed only when a person or a lost device is removed, every database copied under the new key and the old one dropped. Today a scope has numbered epochs, and a new one is made when someone loses the right to open it.
- Backups' recovery from nothing: a device restoring with no one left to pair with admits itself with the recovery key's countersignature.
- Services each running as their own OS user per scope, and Shell and Computer Use only with a grant for that device and session, approved by the person.
- A new device let in by its sign-in when the proof is one the person's clients can check themselves (an OIDC provider's signed ID token, or a passkey, naming the new device's key), approved by whichever of their clients is online; otherwise one tap on their phone: `docs/plan-device-approval.md`.
- Native apps keeping their own sealed copy, for reads with no network; writes while offline, replayed like a rebase, never repeating what went outside.

## Where to look

`src/` and each file's module doc, until the plan's phase 3 generates a map. `tests/contracts.rs` is where two languages must agree.
