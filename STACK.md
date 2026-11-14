# What it's built with

Chosen on 14 November 2026. `GLOSSARY.md` and `ARCHITECTURE.md` name no technology, so the names are here. What else
exists and what was set aside is in `PRIOR-ART.md`.

## Chosen

- **A server is one program on Mac, Linux and Windows.**
- **A service is a part of a plugin's Elixir that names the machines it needs.** Code in another language is a program
  the plugin brings and starts as a child of the server.
- **A Rust add-on to the server for SQLite.**
- **A database is SQLite plus numbered log blobs.** Each log blob is written only if absent, and writing the next
  number is the lease, so no clock is needed. A snapshot is named by the log number it covers.
- **The bucket is Cloudflare R2**, unless measured write times say otherwise.
- **Everything at the network's address is a Cloudflare Worker**: the gateway, the signer, the relay, the web client's
  files and the way in for outsiders. The relay is a Durable Object that sleeps while idle.
- **Each client is one Tauri shell per platform**, drawing in the device's web view.
- **A plugin's app is web content, written in JavaScript.**
- **The Tauri shell is a window on the client's Rust half**, which already is the CLI and MCP.

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

1. Two more real plugins in Elixir, written by an agent: 1 day.
2. iroh through a Rust add-on on Windows: 1 day.
3. An app in Tauri's web view on a mid-priced Android phone, opening, scrolling and going back: 2 to 3 days.
4. A phone on mobile data reaching a home server: how often directly, and how long the first connection takes: 1 day.
5. How long a write to the bucket takes from a home connection: 1 hour.
6. Whether the bucket refuses a signed upload that drops its condition: 1 hour.
