# Plan: a small oned, and services as apps' own programs

Intent decided by Andre on 2026-11-05 (GLOSSARY.md, ARCHITECTURE.md b15b9733). He left the decisions below to the
simplest choice that works; they're at the end.
This file goes once phase 7 is done: from then on the code and ARCHITECTURE.md say it.

## Why

Today oned and the node are one program, and services are four different things:
- **built into the node**, in its process: Shell, Models, AGENTS.md (`services::built_in`);
- **this program run again** as `oned <name>`: AI Usage and, since #1376, Agent Devices (`services::hosted`);
- **a program in another language**, published with the release key: WhatsApp (`services/whatsapp`, Go,
  `service-release` entries, fetched by `oned::keep_parts`);
- **Swift in the Mac app's process**: iMessage, Calendar, Contacts, Reminders, Computer (`apple/Mac/Services`), because
  macOS grants its privacy permissions to the Mac app.

That costs us:
- the core names apps (`built_in`, `hosted`), against "the core never names an app";
- a bug in Shell or Models takes the node down, and every service runs as the person with their whole home and network;
- only we can ship a service, since only the release key makes a node run one;
- an app and its service live in two places (`apps/whatsapp`, `services/whatsapp`) and are switched on in two places
  (the App Store adds the app; Settings › Devices or `apps_install_on` switches its service on a device);
- the Cloudflare node can run apps but nothing else, and is built and switched differently from every other node.

## The end state

**oned is small.** It holds the node's key, connects it to the network, reads the scope database for what should run
there, and installs, starts, watches and stops it. It runs no calls itself. What it runs:
- **the app runtime**: our wrapper around workerd, which is the rest of today's node (calls, screens, schedules,
  the Vault's requests, steps to the bucket). It is switched on per node, as the `apps` part is today;
- **services**, each its own program, in the wrapper.

oned has one version per platform: computers (Linux, Mac, later Windows) and Cloudflare. On a Mac, oned stays inside the
Mac app, run by its launch agent, as now.

**An app is one folder** in `apps/` (and the same shape for a network's own app):

```
apps/whatsapp/
  app.js          screens, widgets, tools, and its service's declaration
  service/        the service's source: go.mod, Cargo.toml or Package.swift
```

A service is its app's alone: only that app's calls reach it (`ctx.service`), and other apps call that app's tools.
`app.js` declares it:

```js
service: {
  network: ["web.whatsapp.com", "*.whatsapp.net"],  // hosts it may reach; none by default
  person: false,      // true: runs as the node's person, with their files (Shell, AGENTS.md, AI Usage, Mac data)
  platforms: ["linux", "mac", "cloudflare"],        // where it can run; each needs a build
}
```

**The wrapper**, per platform. A service gets its own folder, its own endpoint key in it, no scope keys, and the
network only through oned, which lets through only the hosts it declared:
- **Linux:** a transient systemd unit per service with `DynamicUser=` (its own OS user, made and dropped by systemd),
  `StateDirectory=` as its folder, `ProtectHome=`, `ProtectSystem=strict`, `PrivateNetwork=` and one unix socket to oned
  bound in, which carries its calls and a CONNECT proxy that refuses undeclared hosts. oned runs as the person, so
  `install.sh` (which has sudo already) adds a polkit rule letting that user start and stop only `<cli>-service@*`.
- **Mac:** a separate OS user needs an admin password to make and can't reach the person's Messages, Calendar or
  Contacts, which macOS grants per user. So a service runs as the person under a Seatbelt profile (`sandbox-exec`, as
  Chrome, Bazel and Claude Code use) that oned writes from its declaration: its folder only, network only to oned's
  socket. A `person: true` service gets the person's home read and write, but still only oned's socket.
- **Windows** (when there is a Windows node): an AppContainer per service, which is its own identity with no network
  unless given; oned reaches it over a named pipe. No admin needed.
- **Cloudflare:** a service built for Cloudflare is a Worker module the Cloudflare node loads with Worker Loader, its
  `globalOutbound` a filter of its declared hosts, its folder a Durable Object's storage. A service that needs a whole
  machine (Agent Devices' browsers) runs in a Cloudflare Container, if switched on.

**Builds and hashes.** One rule on every node: **a node runs a service's build only if its hash is the one its app's
latest deploy names for that node's platform.**
- A deploy carries `services: { "<platform>": "<blob hash>" }` beside its bundle, each build a blob like the bundle.
- **A network's own app:** `one deploy apps/<name>` builds the service on the caller's machine, as it builds screens
  today, for each platform it declares and that machine can build (Go and Rust cross-build to Linux, Mac and Windows;
  Swift needs a Mac; Cloudflare is JavaScript or WebAssembly). A platform it can't build is left out, and the deploy
  says so; deploying again from a machine that can adds it. The deploy is signed by whoever deployed, as now. The
  release key never signs it.
- **A built-in app:** `one update` and `one release` build its service per platform on our machines (Linux on personal,
  Swift on the Macs) and sign each build with the release key. The release names each build's hash in the built-in
  bundle the node carries, so a built-in app's deploy names them like any other, and the same rule checks them.
  `service-release` entries go.

**Mac permissions (TCC).** The Mac's Swift services become helper apps inside the Mac app, one each:
`Contents/Helpers/<Name> <App>.app` (say "iMessage — The One App"), with its own bundle id, its own `Info.plist` usage
strings and entitlements, signed with our Developer ID. oned starts each as a launch agent of its own in the person's
GUI session (`launchctl bootstrap gui/<uid>`, the plist's program inside the helper, `AssociatedBundleIdentifiers` the
Mac app), so macOS asks for and records each helper's permission under its own name. That means:
- the person grants Full Disk Access to "iMessage — The One App", not the whole Mac app;
- Contacts, Calendars and Reminders prompts name the helper;
- each permission is asked for again once, when phase 3 ships;
- a network's own Mac service can't get TCC permissions unless its helper is signed with a Developer ID; ad hoc signing
  works on one Mac but is asked again after every build. It is still a plain program, and fine for anything TCC doesn't
  guard.

**Adding is desired state.** "Added" is a row in the scope database: `(scope, app, node)`, the node chosen while adding,
and only when the app has a service. The App Store writes it, as any app could (`ctx.install` with a node). Then:
- adding asks "Where should WhatsApp run?", listing the adder's own nodes whose platform the deploy has a build for,
  and, for an admin, the Cloudflare node if it has a build for Cloudflare. One node is picked when there's only one;
  the question is asked for a group's or everyone's app too, by the admin adding it;
- the node named reads the row and runs the service, as `keep_parts` does today with `service` entries;
- the app runtime on every node runs the app's calls, as now;
- removing the app removes the row; that node stops the service and deletes its folder;
- a node runs a service only for a row whose writer may add apps to that scope and owns that node (the Cloudflare node:
  is an admin). Unknown means no.

**What a person sees when the chosen node is off or gone.** One problem box, on the app itself, and the same words to an
agent calling its tools:
- off: "WhatsApp runs on mac, which is off. Turn it on, or move WhatsApp to another device." — Move;
- removed: "WhatsApp ran on mac, which was removed. Choose where it runs now." — Choose;
- no build for any of their nodes: "WhatsApp can't run on your devices yet." — no button.

Moving rewrites the row's node. A service's folder stays on its node and never goes to the bucket, so moving starts it
afresh (sign in to WhatsApp again), unless the service moves its own state, as WhatsApp's `move.go` does. Nothing moves
by itself.

**What's deleted:** `services::built_in` and `hosted`, `src/shell.rs`, `src/models.rs`, `src/agents.rs`,
`src/ai_usage.rs`, `src/agent_devices.rs` (into their apps' folders), `services/`, `apple/Mac/Services`, the Mac app's
TCC entitlements and usage strings, `service-release` entries, the `service` switch entries and Settings › Devices'
per-service switches, and `apps_install_on` / `apps_remove_from`.

## Phases

Every phase ships alone, ends with everything working and sent out, and is safe to stop after. Each is checked in the
throwaway network (`one throwaway`), except TCC, which needs a real Mac.

### Phase 0: this plan

Docs only.

### Phase 1: each service's source in its app's folder

- `services/whatsapp` moves to `apps/whatsapp/service/`; `update.rs` finds services as `apps/*/service/` with a
  `go.mod` or `Cargo.toml`; `services/install.sh` becomes `one services try apps/<name>` (puts your build on a device).
- `app.js` gains the `service` declaration. Today's runtime reads none of it; `uses.services` stays until phase 5.
- Nothing on the wire changes: builds are still `service-release` entries.

Check: `one throwaway`, switch WhatsApp on, link a phone, send a message.

### Phase 2: the node's built-in Rust services become their own programs

- Shell, Models, AGENTS.md, AI Usage and Agent Devices move to `apps/<name>/service/`, each a small crate depending on
  a shared `service` crate (today's `src/service.rs` and the `Host` of `src/services.rs`: register, long-poll, reply).
- Each is built and published per platform as WhatsApp is, and fetched by oned when switched on.
- `built_in()` and `hosted()` go, and with them the core naming apps.
- Older nodes keep their own built-in copies and say the same names in their hello; a client sees no difference.
- The binary shrinks by what those took. Whether they still build with the node (one `cargo build`) or apart is up to
  `Cargo.toml`'s workspace; they publish apart.

Check: `one throwaway shell run`, `one throwaway models chat`, `one throwaway agents get`, borrow and give back an agent device, AI Usage's
status, on a fresh throwaway network where each is switched on from nothing.

Done for Shell, Models, AI Usage and Agent Devices (6bead05..2368cbc): each is `apps/<name>/service/`, a crate on
`service/`; `one update` builds it here for Linux and publishes it before the node; a Mac app carries its own inside it
(`Contents/MacOS/<cli>-<name>`, built by `apple/build.sh` with its node), which oned runs and never fetches
(`oned::carried`), as a checkout's build does for `one throwaway`. Since nothing is built in any more, each runs only
where an entry switches it on: every device of both networks got one for Shell, Models and AGENTS.md before the node
stopped carrying them, and a new device is switched on in Settings › Devices, as phase 6 will ask.

AGENTS.md too: the node hooks its machine's harnesses up itself (`src/harnesses.rs`: its MCP server, the board's
prompt hook, git's token for Repos, the built-in skills), on every device; AGENTS.md and the person's skills are
records in the app's own database, which each person's node copied once from the sealed `agents/sidecar`
(`Node::move_records`, checked by count and every record's contents), and its service asks its app for them
(`_files`) and writes them. `move_records` went once every node of both networks was past it (2026-11-06); the sealed
`agents/sidecar` databases stay in the buckets, read by nothing.

### Phase 3: the Mac's Swift services become helper apps

- Spike first, on a real Mac: one helper (Contacts) as its own bundle, started by oned as its own launch agent. Confirm
  macOS prompts in the helper's name and records it under the helper in Privacy & Security. If it records it under the
  Mac app instead, fall back to helpers started with `open -g` (LaunchServices), which are always their own.
- Then each of iMessage, Calendar, Contacts, Reminders and Computer: source to `apps/<name>/service/` (a Swift package),
  built by `apple/build.sh` into the helper, registered with oned as now over the loopback API.
- The Mac app drops its own entitlements and usage strings.
- The person grants each permission again, once. Each app shows it as its problem box ("iMessage needs Full Disk Access.
  Allow it in System Settings." — Open Settings), which the helper already says in words today.

Check: on the Mac, `apple/build.sh`, then from another device: `imessage_read`, `contacts_search`, `calendar_events`,
`reminders_list`, `computer_screen`. Each permission asked once, under the helper's name.

### Phase 4: the wrapper

- Linux first: transient units with `DynamicUser=`, the polkit rule from `install.sh`, oned's socket and its CONNECT
  proxy. A node whose polkit rule isn't there yet (it wasn't installed again) runs services as today and says so on
  each service: "Runs without its sandbox: install The One App again on linux to give it one."
- Mac: Seatbelt profiles from the declaration.
- `person: true` services run as the person, as now, but still reach the network only through oned.
- Built-in services declare what they need first, so none breaks when the wrapper comes on.
- This is what tests are for: `tests/` gets one that starts a service in the wrapper and checks it can't read the
  person's home, another service's folder, or an undeclared host.

Check: `one throwaway`, WhatsApp still links and sends; `one throwaway shell run` still works (`person: true`).

Done for Linux and the Mac (2026-11-05), `src/sandbox.rs`, its test `tests/sandbox.rs`. Unlike the above: a fixed
template unit (`<cli>-service@.service`, written with polkit's rule by `<cli> services sandbox | sudo sh`, which install.sh runs), not transient units, since starting a transient unit lets its starter set `User=root`; and the
network cut by our launcher inside it (an unprivileged user and network namespace, a port forwarded to the endpoint's
socket, `HTTPS_PROXY` to its proxy), since LXC containers (personal, ptc) switch `PrivateNetwork=` off. A service
whose app declares `"*"` and `person: true` (Shell, Agent Devices) runs as before. A unit's data is systemd's
(`/var/lib/private/<cli>-services/<unit>`): what a service kept in `<home>/<name>` before is moved in once.

### Phase 5: the deploy names its services' builds

Done (2026-11-05). The builds' hashes are in the bundle (`services.json`, `src/bundle.rs`), which the deploy names, so
a rollback brings its builds back with it, and the deploy lists them among its blobs; no field or table changed. Built-in
apps' bundles name none yet: a service whose deploy names no build runs its newest release, as before. The deploy tool
builds a service in Go only (one in Rust would run its build scripts on that machine).

- A deploy gains `services: { platform: hash }`. An older node ignores the field; it runs only release-signed services,
  so it never runs a network's own build.
- `one deploy` builds the service where it can and puts each build as a blob. The deploy tool (MCP) takes builds the
  same way.
- Built-in bundles name their release-signed builds' hashes; oned checks every service by its app's latest deploy.
- A network's own app can have a service from here on.
- Its test: a node refuses a build whose hash its deploy doesn't name, and a deploy by someone who may not deploy it.

Check: in `one throwaway`, deploy a two-file app with a tiny Go service; switch it on; change one byte of the blob in the
bucket and see the node refuse it, with the problem box on the app.

### Phase 6: adding is desired state

- The added row gains its node. Adding an app with a service asks where (web client, the App Store's `apps_add` with a
  `node`, the CLI). The client also writes today's `service` switch entry for that node, so older nodes run it.
- oned reads the added rows, not the switch entries, for what to run.
- Removing the app stops its service and deletes its folder.
- The problem boxes for off, removed, and no build; Move and Choose.
- Settings › Devices stops showing per-service switches; it shows each node, its app runtime switch, and which apps'
  services run there.
- Lands with the storage plan's scope database: if that has switched by then, the node is a column of `installs`;
  until then it's a field of the `install` entry, which older devices ignore.

Check: in `one throwaway`, add WhatsApp and pick the node; see it run there; stop the throwaway node and see the box on
WhatsApp; Move; remove WhatsApp and see the service stop and its folder go. `--as sam` for an app a group's admin adds.

### Phase 7: oned and the app runtime are two programs

- The app runtime (today's node) becomes a program oned starts like any service, but signed by the release key, with
  its own endpoint key, listed by oned as today's service endpoints are. oned keeps only: the node's key, its endpoint,
  reading the scope database, fetching and checking releases and builds, and supervising.
- Older clients reach the app runtime's endpoint as they reach a node's now: oned lists it as that node's.
- Contract: delete `service-release` entries, the `service` switch entries, `apps_install_on` and `apps_remove_from`,
  `uses.services`, and the old paths in `keep_parts`.

Check: `one throwaway` from nothing, every client: open apps, run tools, a schedule runs, a service runs; `one update` hands
over without dropping a call.

The split is done (2026-11-06), simpler than the above: the app runtime has no key and no endpoint (`oned apps`,
`src/apps.rs`). oned stays what everyone calls, holds workerd's port and writes its config, and hands each call to it
over that loopback; its asks come back through oned's outbound port, which holds the call's keys. oned starts it while
the `apps` switch is on, and again when it stops, the services left running; a handover's new oned starts its own in its
own folder. Calls' bookkeeping (keys, databases, steps) stays in oned, since only a holder of the call's keys can do it.
Left for the contract, which needs phase 6: the entries and tools above.

### Phase 8: the Cloudflare node's switches

The Cloudflare node runs today's app runtime already (Worker Loader). It gets the same switches in Settings › Devices,
for admins:
- **app runtime**: on, as now;
- **services built for Cloudflare**, loaded with Worker Loader in the wrapper above;
- **Models**: the Models app's Cloudflare build calls Workers AI through its binding (the account's model list, its
  per-call price shown in AI's usage);

Check: the throwaway network's Cloudflare node runs in local workerd, so app runtime and Worker Loader services are
checked there; Workers AI only on the real account.

Done (2026-11-06): an admin switches the Cloudflare node's services in Settings › Devices or `one services on <name>
--device cloudflare` (a `service` entry naming its key, an admin's to write); the app runtime is always on there. A
built-in app's `service/cloudflare.js` is its service built for Cloudflare, which the Cloudflare node carries and runs
with its `AI` binding; Models' answers with the account's Workers AI models (`@cf/...`, each one's price in models_here)
and passes any other model on to a device. Left: a network's own app's service built for Cloudflare (Worker Loader,
built by the deploying machine) waits for someone to need one; a call a node on a machine runs doesn't reach Workers AI,
since nodes never ask the Cloudflare node (only the Cloudflare node first in run priority does); AI Usage doesn't show
Workers AI's spend.

## Order and owners

Phases 1, 2 and 3 can run in parallel (different folders). 4 needs 2. 5 needs 1. 6 needs 5 and the storage plan's
tables agreed. 7 needs 6. 8 needs 5 for services, and nothing for Models via Workers AI.

## Risks

- **TCC attribution** is the one thing we can't learn from docs: the spike in phase 3 decides it before any service moves.
- **`sandbox-exec` is deprecated** on macOS, though Apple ships on it and others rely on it. If it goes, the fallback is
  an App Sandbox entitlement per helper, which only we can sign.
- **Moving a service loses its state** unless it moves it itself. People will meet this when a Mac is replaced.
- **Phase 7 moves a lot of code.** It waits until the storage plan's phase 3 has moved folders, so the two don't fight.

## Decisions

Taken as the dumbest choice that works (Andre, 2026-11-05: "do what is most dumb and elegant").

1. **The wrapper's identity:** its own OS user on Linux (systemd's `DynamicUser=`), Seatbelt as the person on a Mac,
   AppContainer on Windows once Windows runs a node. ARCHITECTURE.md says "its own identity".
2. **Services that need the person** (`person: true`: Shell, AGENTS.md, AI Usage, the Mac's Apple-data services):
   allowed for any app, said plainly when adding ("Runs as you on mac"), never silently.
3. **Who builds a network's own app's service:** the deploying machine, for every platform it can build. Nodes never
   build.
4. **Mac permissions:** one helper app per Mac service inside the Mac app, each holding its own permission.
5. **A network's own Mac services:** allowed, with no Mac permissions. No path to give them any until someone needs it.
6. **When the chosen node is gone:** nothing moves by itself; the app shows one problem box with Move.
7. **A group's or everyone's apps:** the admin adding one picks one of their own nodes or the Cloudflare node; any
   admin may later move it to one of theirs.
8. **One node per service.**
9. **Cloudflare Containers:** none. The Cloudflare node's switches are the app runtime, and later Models via Workers
   AI; Agent Devices stays on computers.
10. **The app runtime as a program oned starts (phase 7):** yes, last.
