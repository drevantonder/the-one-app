# What it's built with

Chosen on 14 November 2026. `GLOSSARY.md` and `ARCHITECTURE.md` name no technology, so the names are here. What else
exists and what was set aside is in `PRIOR-ART.md`.

## Chosen

- **A server is one program on Mac, Linux and Windows.**
- **The bucket is Cloudflare R2**, unless measured write times say otherwise.
- **Each client is one Tauri shell per platform**, drawing in the device's web view.
- **What sits at a network's address is Cloudflare Workers**, and a plugin's part there is one more: an ordinary
  Worker in the plugin's folder, deployed with the address's own.
- **Signing in is Better Auth, in a Worker with a D1 database of its own**: the sign-in plugin's part at the
  address. It keeps passkeys, lets a new device in with its Device Authorization plugin (RFC 8628), and is the
  OAuth provider that MCP's authorization asks for.
- **An app's UI is Solid 2.0, built by a server with Vite**, which Bun runs. Its components' behaviour is Zag's,
  in Ark UI's shape until Ark runs on Solid 2.0, when they import Ark's instead.

## Not chosen yet

- What starts a network's cloud server.

## What this costs

- A plugin's author writes two languages: Elixir on the server and JavaScript in the client.
- Solid 2.0, Zag 2.0 and Vite 8 are pre-releases, pinned to versions tried together.
- A crash in a Rust add-on ends the VM. Rust panics are caught, and what's left is rare.
- The VM takes up to a second to start, which a cloud server started on demand pays each time.
- No official build of the Erlang VM for Windows on ARM was found. The x64 build likely runs there emulated.

## To test first

None of this was run. About a week in all.

1. iroh through a Rust add-on on Windows: 1 day.
2. An app in Tauri's web view on a mid-priced Android phone, opening, scrolling and going back: 2 to 3 days.
3. A phone on mobile data reaching a home server: how often directly, and how long the first connection takes: 1 day.
4. How long a write to the bucket takes from a home connection: 1 hour.
5. Whether the bucket refuses a signed upload that drops its condition: 1 hour.
