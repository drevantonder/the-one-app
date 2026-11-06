# Architecture

Your devices, and your team's, as one place for apps and agents. A network is one person's or one workplace's: its people, their devices, their apps, and a bucket (a folder or S3-compatible storage) holding everything, sealed. Every machine in it runs the same program; at its address, a few small Workers on Cloudflare, each named by its job, only route, sign in, relay and carry pushes. Each app's database for a scope is the truth, kept as signed steps in the bucket, and who can do what is the scope database's rows.

It's held to targets measured in `one throwaway`, from a phone on mobile data where it matters: the web client usable within 1.5 s of a cold open; a remote MCP call adding at most 100 ms to a local one; a write seen on another device within 1 s; a call one round trip; no data rewritten when someone adds a device.

This page holds only what the code can't say. How each part works is its folder's doc: `cargo test map -- --nocapture` prints them all, as a map. `tests/contracts.rs` is where two languages must agree.

## Trust

- **Keys are opened only on clients**, the devices of people and their agents (apps, browsers, the CLI, the remote MCP server). **Nodes** run calls and hold no keys: a caller hands a node the database keys one call needs, which expire with its sign-in. The only exceptions are app schedules and the Vault, where a person or an admin chooses a machine to hold keys and can take them back.
- **The Workers at the address open nothing.** A hacked node gets only the calls it ran while it was hacked; a hacked gateway, sign-in, relay, URL signer or push Worker reads no data. The Workers that serve browsers' code could take what browsers open, which is why they stay small, and why the Cloudflare account is guarded with a hardware security key. The remote MCP server is the same: its devices' secrets are sealed under their MCP clients' tokens, which reach it on every call, so whoever can change its code could open the keys of the scopes those devices were approved for. We consider it as secure as our Cloudflare account (Andre, 2026-11-05).
- **Who can do what is signed, and kept in the bucket**: the scope database's steps, in the one order the bucket gives them, each checked by every reader. A change counts by who signed it, never by the time it claims.
- **Clients take who's who from the signed steps themselves**, from the freeze the network's recovery key signed, never from what a node or the relay says, and seal or hand keys only to whom they list.
- **Signing in proves who you are, nothing more.** It never opens a key. A new device gets in only when one of its person's devices approves it: by its code, or by itself when the sign-in is one it checks without trusting sign-in, an OIDC provider's ID token for the network's own client or a passkey's assertion naming the new device and the network, from an account their own devices recorded (`src/sign_in_proof.rs`), and only when it signed in itself: one finished from a link opened elsewhere (the CLI's, or one sent to the person) waits for a tap. Every device of theirs then shows it joined, with Remove.
- **One network, forever.** A key belongs to one network; networks never talk to each other.
- **Secrets stay put:** a Vault item's secret leaves a node only inside a request to one of its hosts, by an app switched on for it. Nothing reads one back.
- **Keys decide access; the relay decides who's on the network.** It admits only the network's endpoints, a list devices change by signed tells.
- **A push is sealed to the one device it's for.** The push Worker, and Apple's, Google's, Microsoft's and browsers' push services, carry locked bytes and read none of them: they see only that a device got a push, and when. A note says which of the network's devices sent it, and an action in one is checked by the device that gets it.
- **A service runs only the build its app's latest deploy names** for its platform, by hash, the deploy signed by whoever deployed it; or, for a built-in app's whose deploy names none, the newest release. The release key signs only what we ship (oned, the node, built-in apps' services), never a network's own code.
- **A service's program runs in its sandbox** (`src/sandbox.rs`): no keys, the network only to the hosts its app declares, and, unless its app declares it runs as the person (said wherever it's switched on), its own OS user and folder and none of theirs; so the sandbox protects a node, not who wrote the code. A machine that can't give one runs it without, and says so on it.
- **Only the URL signer holds the bucket's credentials;** everything in the bucket is sealed, so what it hands out opens nothing.

## The three rules

Who can do what is the signed steps of each scope's database, in the one order the bucket gives them, and every reader checks each step by three rules:

1. the network's first step, and the recovery key it names;
2. epochs never go down;
3. each step's signer holds the permission its change needs (`src/permissions/`).

A removed device's later steps fail rule 3, because they come after its removal in the one order.

## Designed, not built

- One storage model, and permissions as data: the plan under way is `docs/plan-storage-and-permissions.md`.
- One key per scope at a time, changed only when a person or a lost device is removed, every database copied under the new key and the old one dropped. Today a scope has numbered epochs, and a new one is made when someone loses the right to open it.
- Backups' recovery from nothing: a device restoring with no one left to pair with admits itself with the recovery key's countersignature.
- Services each running as their own OS user per scope, and Shell and Computer Use only with a grant for that device and session, approved by the person.
- The rest of `docs/plan-device-approval.md`: a push that wakes the phone to check a new device's sign-in.
- Native apps keeping their own sealed copy, for reads with no network; writes while offline, replayed like a rebase, never repeating what went outside.
