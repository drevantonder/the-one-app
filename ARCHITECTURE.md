# Architecture

Your devices, and your team's, as one place for apps and agents. A network is one user's or one workplace's: its users, their devices, their apps, and a bucket (a folder or S3-compatible storage) holding everything, sealed. Every machine in it runs the same program; at its address, a few small Workers on Cloudflare, each named by its job, route, sign in, relay, carry pushes and sign bucket URLs. Two Workers are not like those: the remote MCP server is a client, and the Cloudflare node is a node. Each app's database for a scope is the truth, kept as signed steps in the bucket, and who can do what is the scope database's rows.

It's held to targets measured in `one-dev throwaway`, from a phone on mobile data where it matters: the web client usable within 1.5 s of a cold open; a remote MCP call adding at most 100 ms to a local one; a write seen on another device within 1 s; a call one round trip; no data rewritten when someone adds a device.

This page holds only what the code can't say. How each part works is its folder's doc: `cargo test map -- --nocapture` prints them all, as a map. `tests/contracts.rs` is where two languages must agree.

## Trust

- **Keys are opened only on clients**, the devices of users and their agents (apps, browsers, the CLI, the remote MCP server). **Nodes** run calls and hold no keys: a caller hands a node the database keys one call needs, which expire with its sign-in. The only exceptions are app schedules and the Vault, where a user or an admin chooses a machine to hold keys and can take them back.
- **The Workers at the address open nothing.** A hacked node gets only the calls it ran while it was hacked; a hacked gateway, auth, relay, URL signer or push Worker reads no data. The Workers that serve browsers' code could take what browsers open, which is why they stay small, and why the Cloudflare account is guarded with a hardware security key. The remote MCP server is the same: its devices' secrets are sealed under their MCP clients' tokens, which reach it on every call, so whoever can change its code could open the keys of the scopes those devices were approved for. We consider it as secure as our Cloudflare account (Andre, 2026-11-05).
- **A native app's window shows only what the release key signed.** The web client it runs is the copy built into it or a bundle the release key signed (`src/web/bundle.rs`), which it checks before keeping it and again every time it starts: signature, shell version, and never older than what it has. The Workers and the bucket only carry bundles (at `<address>/updates/`), so a hacked Worker can't change what an app runs, where it could in a browser. The window loads the bundle from the app's own origin, never a remote one, so it can ask the app for exactly what the built-in copy can. A bad signature, a failed download or a bundle whose page doesn't start leaves the app on its last good bundle or the built-in one, which stays as the floor; a new one goes in when the window next opens, never under a running page.
- **An app's window runs the same client as a browser**, from its bundle (the client's Rust, as WebAssembly, in the page): a change to it, a `READS` bump included, reaches the app at its next open with no native build. Its device key stays in the OS's secret store, and signing and opening go through the app (the `Opens` and `Signs` traits' JS callbacks), which never hands the key over. An older Android System WebView has no Ed25519 in WebCrypto, so that page uses the shell's own client instead, as a shell built with no WebAssembly does. What runs with no window (widgets, notifications) stays the shell's Rust; a shell behind `READS` keeps showing the last widgets and the window says to update, and needs a native build, as a change to what the shell gives the page (`web::SHELL`) does.
- **Who can do what is signed, and kept in the bucket**: the scope database's steps, in the one order the bucket gives them, each checked by every reader. A change counts by who signed it, never by the time it claims.
- **Clients take who's who from the signed steps themselves**, from the freeze the network's recovery key signed, never from what a node or the relay says, and seal or hand keys only to whom they list.
- **Signing in proves who you are, nothing more.** It never opens a key. A user has any number of linked identities, a passkey or an account at an OIDC provider the network has registered with (Google is one like any other), and one more is only ever linked from a device already signed in, which checks it and records it in the signed steps. A new device gets in only when one of its user's devices approves it: by its code, or by itself when the sign-in is one it checks without trusting auth, an OIDC provider's ID token for the network's own client or a passkey's assertion naming the new device and the network, from an identity their own devices recorded (`src/onboarding/sign_in_proof.rs`), and only when it signed in itself: one finished from a link opened elsewhere (the CLI's, or one sent to the user) waits for a tap. Every device of theirs then shows it joined, with Remove.
- **One network, forever.** A key belongs to one network; networks never talk to each other.
- **Secrets stay put:** a Vault item's secret leaves a node only inside a request to one of its hosts, by an app switched on for it. Nothing reads one back.
- **Every client lists and may use the same tools.** What one may do is decided by its user's rules in the scope database and by what its keys open, never by which kind of client asks. A node signs a change to who can do what only as its own user's device, for its own programs; every other client signs its own.
- **Keys decide access; the relay decides who's on the network.** It admits only the network's endpoints, a list devices change by signed tells.
- **A push is sealed to the one device it's for.** The push Worker, and Apple's, Google's, Microsoft's and browsers' push services, carry locked bytes and read none of them: they see only that a device got a push, and when. A note says which of the network's devices sent it, and an action in one is checked by the device that gets it.
- **A service runs only the build its app's latest deploy names** for its platform, by hash, the deploy signed by whoever deployed it; or, for a built-in app's whose deploy names none, the newest release. The release key signs only what we ship (oned, the node, built-in apps' services), never a network's own code.
- **A device takes its own updates, by a signed record** at the network's address (`src/latest.rs`, `<address>/updates/newest`): a GET, nothing else of the network working, verified against the release key built in, never going backwards. A bad or missing record means staying on what it has and saying why, so no change can cut a device off from its own update.
- **A service's program runs in its sandbox** (`src/services/sandbox.rs`): no keys, the network only to the hosts its app declares, and, unless its app declares it runs as the user (said wherever it's switched on), its own OS user and folder and none of theirs; so the sandbox protects a node, not who wrote the code. A machine that can't give one runs it without, and says so on it.
- **Only the URL signer holds the bucket's credentials;** everything in the bucket is sealed, so what it hands out opens nothing.

## The three rules

Who can do what is the signed steps of each scope's database, in the one order the bucket gives them, and every reader checks each step by three rules:

1. the network's first step, and the recovery key it names;
2. epochs never go down;
3. each step's signer holds the permission its change needs (`src/permissions/`).

A removed device's later steps fail rule 3, because they come after its removal in the one order.

## Designed, not built

- One key per scope at a time, changed only when a user or a lost device is removed, every database copied under the new key and the old one dropped. Today a scope has numbered epochs, and a new one is made when someone loses the right to open it.
- Backups' recovery from nothing: `one backups check --folder <path> --recovery-key <file>` opens a backup with the recovery key alone, but a device restoring with no one left to pair with doesn't yet admit itself. The rules already let the recovery key sign a device's step (`database::rules`); nothing does it yet.
- Services each running as their own OS user per scope, and Shell and Computer Use only with a grant for that device and session, approved by the user.
- The rest of `docs/plan-device-approval.md`: a push that wakes the phone to check a new device's sign-in.
- What's left of the storage move: `docs/plan-storage-and-permissions.md`.
- Native apps keeping their own sealed copy, for reads with no network; writes while offline, replayed like a rebase, never repeating what went outside.

## Decided

Andre, 2026-11-05: the dumbest choice that works.

- A service's identity is its own OS user on Linux (systemd's `DynamicUser=`), Seatbelt as the user on a Mac, AppContainer on Windows once Windows runs a node.
- A service that needs the user (`user: true`: Shell, AGENTS.md, AI Usage, the Mac's Apple-data services) is allowed for any app, said plainly when adding ("Runs as you on mac"), never silently.
- The machine that deploys builds a network's own app's service, for every platform it can build; nodes never build. A network's own Mac service gets no Mac permissions.
- One node runs a service. When it's gone nothing moves by itself: the app shows one problem box with Move. For a group's or everyone's app, the admin adding it picks one of their own nodes or the Cloudflare node, and any admin may move it to one of theirs.
- No Cloudflare Containers. The Cloudflare node runs the app runtime and services built for Cloudflare (Models answers with Workers AI there); Agent Devices stays on computers. Left: a network's own app's service built for Cloudflare, until someone needs one; a call a machine's node runs doesn't reach Workers AI; AI Usage doesn't show its spend.
- Risks: TCC attribution on a Mac is only learned by trying; `sandbox-exec` is deprecated though Apple ships on it (the fallback is an App Sandbox entitlement per helper, which only we can sign); moving a service loses its state unless it moves it itself.
