# Glossary

What our words mean, and which nearby words they are not. Read it with ARCHITECTURE.md. A rename changes this file in
the same commit. "Avoid" lists the words not to use for that thing, in code, docs, screens, CLI or MCP text.

## The network

- **Network**: one user's or one workplace's whole: its users, their devices and nodes, their apps, its bucket, and
  the pieces at its address on Cloudflare. A key belongs to one network forever, and networks never talk to each other.
  Users see "your One App network", or a custom build's, "the PTC App network". Avoid: install.
- **Custom build**: this code built with its own `config.toml`, the only place the product is named (The One App, The
  PTC App). It usually serves one workplace's network, but can serve many. Avoid: white label.
- **Throwaway network**: what `one-dev throwaway` makes from your checkout's build: a network on your machine, with its own
  pieces at its own address, where you're a new user signing in with no passkey. For testing. Avoid: try install.
- **User**: someone in the network. An **admin** is a user who may change what's everyone's. Avoid: person.
- **Group**: users who share apps and data.
- **Everyone**: what users see for the network-wide scope (`global` in code).
- **Scope**: whose something is: everyone's, a user's, or a group's. Each has its own keys and its own app databases.
  Our word only: users see a user, a group or everyone.
- **Identity**: what a user signs in with: a passkey, or their account at a **provider** (an OIDC one the network has
  registered with, by issuer and subject). A user can have several, each linked from a device already signed in.
  Avoid: login, for this.
- **Agent**: an AI acting for a user (Claude Code, Codex, a T3 thread), through one of that user's clients: the CLI,
  MCP or a browser. It acts as that user, never as itself: it has no permissions of its own.

## Machines

- **Device**: where a user's clients and node are, listed as theirs: a machine they own (a phone or a computer), a
  browser they signed in on, or a remote MCP client's place in the remote MCP server. It has clients, a node, or both:
  a computer can have both; a phone, a browser and a remote MCP client's device have only a client. A browser's
  sign-in is its device: nothing ties it to the computer it's on, so it's never part of that computer's. The Cloudflare
  node is not a device.
- **Onboarding**: what someone sees on a device before they're in the network, first time or coming back: the first
  screen, proving who they are, waiting for a device of theirs to approve it, and starting. Our word for the journey and
  for its folder (`src/onboarding/`); users see Sign In, Join and Waiting for Approval.
- **Client**: where a user or their agent uses the network, and the only place keys are opened: the native apps (Mac,
  iPhone, Android, Windows), the web client in a browser, the CLI and the local MCP server on a computer, and the remote
  MCP server. Each has its own key, never shared.
- **oned**: the small program, and the spec it follows, that makes a machine a node: it connects the machine to the
  network, and installs and runs what the node's owner switches on there (the app runtime, services). It runs nothing
  itself, and stays small. It has a version for each platform it runs on: computers (Mac, Windows, Linux) and
  Cloudflare, and perhaps Android one day.
- **Node**: one place oned runs: a computer's node, or the Cloudflare node. It holds no keys, except for app schedules
  and the Vault, where a user chose that node. A phone is a device and not a node; the Cloudflare node is a node and
  not a device; a computer can be both. Avoid "node" for the app runtime. Avoid: serverless node.
- **Platform**: what a client or oned is built for: Mac, Windows, Linux, iPhone, Android, a browser, Cloudflare.
- **Endpoint**: an iroh identity, of its own key. Each client, node and service is one; the relay lets in only the
  network's.
- **Network message**: what one node tells another directly over iroh: `hello` (what it runs and its load, every so
  often) and `changed` (that something changed). A nudge, never trusted for what's true, which the bucket says.
- **Agent device**: a machine agents use (Linux with a browser, Windows, Android, an iPhone), which the Agent Devices
  service runs on a user's node. It is not in the network: it has no key there, and is reached only through that
  service.

## Apps

- **App**: code deployed into a scope, run in the app runtime. It publishes any of four things: **screens** (pages
  users open), **widgets** (what it shows on a home page), **tools** (what agents use, through MCP and the CLI), and
  **services**. An app with only tools is still an app (Gmail, Shell). The core's own features (the Vault, Backups,
  Groups, Devices, Widgets, Push, logs) have tools too, named the same way (`vault_list`), but are not apps: no list of
  apps has them. Avoid: worker, connector, operation (for a tool).
- **Service**: native code an app publishes, for what the app runtime can't do (reach a Mac's Messages, hold WhatsApp's
  linked phone, run a local model), run on one of the nodes of whoever added the app, which they choose while adding it,
  as its own process, an endpoint of its own key. Adding WhatsApp asks which of your nodes runs its service. A service
  exposes **functions** (`ctx.services.whatsapp.send(…)`), and is its app's alone: only that app calls them, and other
  apps call that app's tools. Built-in apps' services ship with each release. Avoid: part, native component, device
  service; operation, for a function.
- **Added**: an app recorded as added to a scope, by a user, or for a group or everyone by its admin: the state the
  network should be in. The App Store only records it, like any app could; the core makes it so, deploying the app and
  running its service on the node the user chose while adding it, and takes it all away when it's removed. Avoid:
  install, uninstall.
- **Accounts**: the accounts apps sign in to (a Gmail address, a WhatsApp number), listed together in the sidebar.
- **App runtime**: where apps run on a node, switched on per node: a sandbox with no disk and no keys of its own, where
  an app reaches only its own databases, its own services, the web, and other apps' tools, through `ctx`. It is our
  wrapper around workerd (Cloudflare's open-source Workers runtime). Avoid: node, app service.
- **Call**: one run of an app's tool, screen or schedule, on one node, with the keys the caller hands it for that call,
  which expire with the caller's sign-in.
- **Deploy**: a version of an app put into a scope. **Built-in apps** are the folders in `apps/` with an `app.js`, shipped with every
  release; every other one is made in the network.
- **App schedules**: a user choosing a node to keep an app running there, holding that app's keys in that scope (for
  up to 30 days, and longer as they use it), so its schedules run while no one has it open. Avoid: keep, grant, standing
  work.
- **Vault**: secrets apps use without anyone seeing them: a secret leaves a node only inside a request to one of its
  hosts.

## Data

- **Bucket**: the network's one store, a folder or S3-compatible storage, holding everything sealed. Only the URL signer
  holds its credentials.
- **Key**: every client, node and service has its own. Each scope has numbered **epochs** of keys, sealed to the devices
  that may open it and to the **recovery key**, which an admin keeps offline.
- **Database**: one SQLite database, kept sealed in the bucket as steps at numbered positions, the bucket deciding their
  order. A scope has three kinds: **an app's database**, its data and the truth; **the node's database**, how things
  went (usage, errors, runs); and **the scope database**, who's who, permissions and keys.
- **Blob**: bytes a database's step names, kept in the bucket sealed with their scope's key.

## At the network's address

These run on Cloudflare, each named by its job, with no name for them together. Avoid: kernel.

- **Gateway**: the network's address, the one users use. It sends each request where its path says, and holds and runs
  nothing.
- **Auth**: who you are, and nothing more (a passkey, or a provider the network has registered with). It opens no key. Users still "sign in" to it; it is never "the sign-in". Avoid: sign-in.
- **Relay**: carries packets between endpoints that can't reach each other directly, and keeps the list of the network's
  endpoints. It reads nothing.
- **URL signer**: the only holder of the bucket's credentials. It hands the network's endpoints signed URLs, which open
  nothing, since everything in the bucket is sealed.
- **Remote MCP server**: the client for MCP clients that aren't on a user's devices (claude.ai, ChatGPT). It opens
  keys, but only while an MCP client calls, under that client's own tokens. The **local MCP server** is `one mcp`, which
  agents on a computer start themselves. Avoid: MCP Worker.
- **Cloudflare node**: oned on Cloudflare. A node like any other.
