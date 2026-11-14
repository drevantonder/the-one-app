# What it's built with

Chosen on 14 November 2026. `GLOSSARY.md` and `ARCHITECTURE.md` name no technology, so the names are here. What else
exists and what was set aside is in `PRIOR-ART.md`.

## Chosen

- **A server is one program on Mac, Linux and Windows.**
- **The bucket is Cloudflare R2**, unless measured write times say otherwise.
- **Each client is one Tauri shell per platform**, drawing in the device's web view.

## Not chosen yet

- What the UI that clients share is built on. Vercel's UI SDK is being considered.
- What starts a network's cloud server.
- What signs users in.

## What this costs

- A plugin's author writes two languages: Elixir on the server and JavaScript in the client.
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
