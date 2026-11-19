# What exists already

Read this before designing anything here, so we don't build what exists, look again at what was set aside, or decide
again what's decided. Add to it whenever something turns up that nobody in the conversation knew or remembered.
Everything is as of November 2026, and from reading, not running, unless it says otherwise.

## Exists, and close to what's designed here

None of these is chosen. Each is named so it's weighed before anything like it is built.

- **celld** (celld.dev, Deno, Apache-2.0, v0.6.2, beta): runs a Cloudflare Workers application, Durable Objects
  included, on your own machines. Each cell is a SQLite database shipped to your bucket in Litestream's format. One node
  serves a cell at a time, claimed by a conditional write of a small record to the bucket. Each turn as writer ships to
  its own numbered folder, and a write is confirmed only while the record still names that node. That is nearly this
  redesign's database and lease. Not chosen as the server. Its limits: one application per fleet, unencrypted traffic between
  nodes, no Windows, clients only query a node, and upgrades that need every node stopped. Its ideas are being merged
  into open-source workerd, unreleased as of 14 November 2026, so workerd may matter more later.
- **Litestream**: ships a SQLite database's log to a bucket. One writer, no failover, and no check before an upload, so
  by itself it doesn't stop a server that lost its turn from writing.
- **Zanzibar**: where relation tuples and the relations' names come from. SpiceDB and OpenFGA are services that
  implement it; we keep the tuples ourselves and run neither.
- **TanStack DB**: a store inside a client that screens query, fed by plain requests to a backend. Live queries,
  optimistic changes at the screen, kept on disk in SQLite (browsers, Tauri), and a separate package that queues
  changes made offline. A 1.0 release candidate is reported for 29 October 2026, by secondary sources only.
- **Conditional writes**: Amazon S3, Cloudflare R2, Tigris, Google Cloud Storage and Azure Blob have them. Backblaze B2,
  Hetzner and DigitalOcean Spaces don't, by celld's own tests.

## What the clients could be built with

- **Expo**: phone apps, and it can send them updates without a store release. No desktop apps. Not chosen.
- **Tauri**: desktop and phone apps from one codebase. What the app as it runs today uses. Chosen on 14 November 2026. From 19 November 2026 it stays the phones' shell (an iPhone's and Android's), and the web view a phone falls back to for a page that needs a newer host; a computer's app has none of it.
- **Our own renderer** (19 November 2026): native screens on a computer, an iPhone and an Android phone are drawn by a renderer of
  ours for our closed set of primitives and tokens, with code copied in rather than depended on, so the repo stays MIT.
  Each copied file keeps its source's notice and says where it came from and at which upstream commit.
  - **GPUI** (Zed, Apache-2.0): its scene, shaders, glyph atlas and wgpu renderer and its desktop windows, input and
    panel code. Not its App, Entity and View layers, which Solid replaces, and not as a dependency.
  - **Parley** (Linebender, as a dependency): text, the same on every platform and on the web, with fontique finding
    fonts and swash rasterizing glyphs. It replaced GPUI's three text systems (Core Text, DirectWrite, cosmic-text),
    which each measured a little differently and broke lines differently from the DOM (19 November 2026: of 347
    texts in the scenes, Core Text broke 39 where the DOM didn't), so one text system gives the same widths
    everywhere (344 of 347 as the DOM breaks them, within 0.42 px on average, on Linux, Windows and a Mac), and is
    what the web's renderer will use.
  - **Taffy**: layout.
  - **Rune** (anomalyco/rune, MIT): its text editing model (cursor, selection, bidi, undo, IME composition), and the
    design of its inspector. Rune itself as the renderer was set aside: weak IME on Windows and Linux, and it takes no
    pull requests.
  - **Flutter's Android embedder** (BSD-3): its windowing, keyboard bridge (`InputConnection`) and autofill.
  - **Flutter's text input plugins** (BSD-3, 19 November 2026): typing on every platform goes through Flutter's own
    bridge, which keeps the input's editing state itself and speaks Flutter's text input protocol (set client, editing
    state, caret and marked text rects, editable transform, actions) with our editing model: Android's
    `TextInputPlugin`, the Mac's `FlutterTextInputPlugin` (Objective-C, in place of GPUI's `NSTextInputClient`), the
    iPhone's `FlutterTextInputPlugin` (Objective-C, with its keyboard, autocorrection and Password AutoFill) and
    Windows' `TextInputPlugin` with its IMM32 `TextInputManager` (C++, in place of GPUI's), with its shared
    `TextInputModel`, built with the `cc` crate and their engine connections replaced by a small C interface. Linux
    keeps GPUI's XIM: Flutter's GTK plugin needs GTK's input method context, which our X11 window hasn't.
  - **AccessKit**: accessibility, from one semantic tree, wired as GPUI wires it.
  - **Flutter's iOS semantics objects** (BSD-3, 19 November 2026): an iPhone's VoiceOver reads that semantic tree
    through Flutter's `SemanticsObject` and its kin (Objective-C), as Flutter's iOS engine gives them its own, kept by a
    bridge modelled on its `AccessibilityBridge`. AccessKit has no iOS adapter, and Flutter's are the UIKit elements a
    renderer of its own draws under, with text fields that read and edit through its text input plugin.
  - **React Native's JSI and Fabric**: how a page's JS tells the host what to draw. Each mutation is a direct call of a
    host function with its values as they are, applied once a turn of the sandbox has run, as Fabric's mounting layer
    applies its mutations, in place of a batch of JSON, as React Native's old bridge had it. In QuickJS, for 11,000
    mutations (1,000 rows), JSON took 8.4 ms, a typed-array command buffer 1.5 ms and direct calls 1.05 ms (measured
    19 November 2026), so direct calls.
  - **wgpu** (19 November 2026): the GPU, one backend for every platform: Vulkan on Linux and Android, Metal on a Mac,
    Direct3D 12 on Windows, from GPUI's WGSL shaders, as GPUI itself moved to it. It replaced Blade (Vulkan and Metal,
    through our fork of it) and the Direct3D 11 renderer copied from GPUI: one backend to keep, not three, and the
    same renderer and shaders run on the web later, as WebGPU in WASM.
  - **tao** (19 November 2026): an iPhone's native windows, with their fingers, safe areas and lifecycle, are tao's,
    in the event loop Tauri runs for the app, which hands them its events (a wry plugin). winit was the first choice
    and can't be used there: its UIKit backend starts only by calling `UIApplicationMain` itself, which tao already has.
  - Not used: Zed's `editor` crate, which is GPL-3.0.
- **Lynx** (ByteDance): native views on Android from JS. A spike on 19 November 2026 drew a native screen in it with a
  cold start 35% faster than the web view and a fifth of its memory. Not chosen: Android uses our own renderer, and the
  web view until that works.
- **Blitz**: an HTML and CSS renderer in Rust. Not chosen.
- **Vercel's UI SDK** (`@ui-sdk`, Apache-2.0, 0.1.0 beta, 12 November 2026): behaviours with no look of their own
  (popups, selection, keyboard, focus) for React, Vue, Svelte, Solid and Web Components, made for agents to build
  with. Not depended on as a package (its DOM parts don't run on a native host); instead, its behaviours and headless
  components are copied into `core/ui/src/behavior-primitives/` and `core/ui/src/headless-components/` (with
  `core/ui/src/UI-SDK-LICENSE.txt` kept and DOM parts replaced by our host's primitives and bridge, as
  `core/ui/AGENTS.md` says).
- **Ark UI** (`@ark-ui/solid`) and **Zag**'s machines: not used. Ark's `6.0.0-next.0` still imports `solid-js/web`, which
  Solid 2.0 removed, so it doesn't build on Solid 2.0 (tried 14 November 2026), and Zag's machines need the DOM, which a
  native host doesn't have. The UI library's components are its own, and no Zag package is used.
- **Asuka** (`egoist/asuka`, MIT, v0.1.0, 13 November 2026): apps in Go drawn with each platform's own widgets. Set
  aside: it's compiled, so an app's entrypoint couldn't arrive from a server, and its widgets are each platform's own look.
- **What an app's JS runs with on a native host** (18 November 2026, measured): the WinterTC Minimum Common API.
  - **AWS LLRT's modules** (Apache-2.0, on rquickjs): chosen for URL, encoding, crypto, Blob, streams, DOMException
    and structuredClone. Its v0.10 is only on GitHub; crates.io's is on an older rquickjs. Its Event, EventTarget and
    AbortSignal give a listener no event's target, no `preventDefault` and no `handleEvent`, so those are
    event-target-shim's (MIT) and ours. Its `Intl` is a time zone helper for dayjs, in en-US only.
  - **FormatJS's Intl polyfills**: set aside. In QuickJS they took 45 ms to load as bytecode and 89 ms to add one
    locale's data, every time a sandbox starts, and their data for CLDR's 421 modern locales is 130 MB (2.7 MB
    compressed together).
  - **ICU4X 2.4**: chosen for Intl, each formatter made in under 0.1 ms. Every locale's data built in is about
    8.5 MB of each binary, so only English's is (`en` and its regional variants), and any other locale falls back to
    English. Its `unstable` `DateTimeFieldBag` mirrors ECMA-402's date options. icu_experimental 0.7 formats only
    lengths, areas, masses, volumes and durations as units, not bytes, temperatures or percentages, so units come from
    CLDR's JSON (`clients/app/renderer/units.js`). It has no DisplayNames for Intl, number ranges or plural ranges yet.

## App systems to learn from

None of these is chosen. All three run apps inside the host with its full access. On 14 November 2026 this redesign
chose the same: an app's code on a server is free, and which servers act for whom is what keeps users apart.

- **Nuxt modules** (Nuxt 4): pages, components and imports are modules on the same list as anyone's, but five core
  ones are exempt from being switched off. One kind of hook: handlers run in the order they registered and change what
  they're handed, with no way to refuse and no record of who changed what. Order between modules is the standing
  complaint. Nuxt 4.1 added dependencies a module declares, with versions, resolved before any module runs.
- **OpenCode's apps, version 2** (`@opencode/app`, 2.0 in September 2026): about sixty of its own features, every
  tool and provider among them, are apps on the public interface. Three separate things where version 1 had one:
  registering what an app provides, hooks that change a running operation, and events that are only heard. An app
  has a stable id, storage of its own, and a typed contract between its parts. The app's context is the public
  client plus a little. Version 1 apps all broke at once.
- **DeepSeek Harness** (`dsh`, developer preview, built on Cordis): "no privileged core"; the agent loop and tool
  registry are apps. An app provides a service under a key and others ask for the key, never the app, so one
  can be swapped. Each hook's contract says how it's called: observed, mutated, or validated by the first to answer.
  Its most reported trap is a wrapping hook that forgets to pass the call on and so blocks it silently.

## Not checked yet

- That celld can load many apps inside its one application.

## To watch

- **A SQLite-over-S3 engine that fences its writer with conditional writes alone** (Litestream, an S3 VFS, celld and
  the like): once one lets a writer commit only while a conditional write in the bucket says it still holds the
  database, with no coordinator to run, it could replace `apps/database`'s `Database.Lease` and `Database.Log`. As of
  20 November 2026 Litestream doesn't check before an upload, and celld fences this way but as a whole application
  server, not a library.

## Looked at and set aside

- **Durable Objects on Cloudflare as where the truth is kept**: their data can't be moved out to your own machine.
- **D1 as a standby**: it has no feed of its changes, and a database per user or group fits it badly.
- **Turso and sqld**: one primary that copies pass writes to. Its newer engine lets many connections write at once on
  one machine, in preview. Neither is several servers writing one database.
- **CouchDB**: two equal servers both writing, but documents and kept conflicts, not SQL. **rqlite**: SQL and no
  conflicts, but three machines.
- **Willow, Earthstar, iroh-willow**: no "write only if absent", last write wins by a clock the writer sets, and
  unreleased or archived. **iroh-blobs and iroh-docs**: a way to copy blobs between devices, not a store.
- **Jazz**: its encrypted design is frozen and its makers wrote up why they left it; Jazz 2 is rows a server is
  trusted with, in alpha.
- **What an app's server-side code could run in**, weighed on 14 November 2026 before Elixir was chosen:
  - **QuickJS-NG inside the server**: smallest, with limits per app, but no Web APIs of its own and no way to reach
    the machine, so services would stay a separate thing.
  - **V8 inside the server** (`deno_core`): fast, but it leans on Deno's crates, whose team moved to Cloudflare.
  - **workerd beside the server**: a second program to run, loading code at run time behind an experimental flag, and
    every database call crossing a socket.
  - **Wasm components**: any language, but the heaviest path for JavaScript and a compile step on the server.
  - **Each app as its own process**: any language and every package, at 20 to 60 MB each.
- **`service-manager`** (crate, 0.11.0, checked 20 November 2026) to register a server or Computer Use's desktop
  with the system: on Windows it has only `sc.exe` and WinSW, both of which refuse a user-level service and install
  one an administrator must, running apart from its person, not the person's own scheduled task in their session that
  ours are. On Linux and a Mac it has user-level systemd units and LaunchAgents, but takes our own unit or plist
  (`contents`) for what it can't say (`Type=notify`, `PartOf=graphical-session.target`, `LimitLoadToSessionType`),
  and loads an agent with the older `launchctl load`, not `bootstrap gui/<uid>`. So it would cover two of three
  systems and still leave us writing every definition.
- **Podman's REST API** (its Docker-compatible one, over its Unix socket with Req) for Containers, in place of its
  CLI (checked 20 November 2026, Podman 4.9.3): the socket is a service of its own, `podman.socket`, which Ubuntu's
  package leaves disabled, so every machine would need it enabled, or the server would start `podman system service`,
  the CLI again. `exec` becomes a create, a start and an inspect with a multiplexed stream to split; `cp` becomes tar
  archives packed and unpacked; `build` takes its folder as a tar. What the CLI prints is read as JSON already
  (`--format json`), so its parsing is a few lines. It would be no shorter and one more thing to set up.
- **`git http-backend`, or gix, for a push to Git Repos** (20 November 2026): a fetch is `git http-backend`'s
  already, protocol v2 included. A push is decided against the bucket, each ref moving only from where the pusher saw
  it, not against the mirror `http-backend` would write to, so it would need a hook calling back into the server for
  each ref; ours is about 40 lines of pkt-lines (`GitRepos.Wire`). gix speaks git's protocols as a client, with no
  receive-pack to serve one.
- **cargo-dist** (dist) for releasing (20 November 2026, from reading): it builds Rust binaries in GitHub Actions on a
  tag, publishes them to GitHub Releases with shell, PowerShell, MSI, Homebrew or npm installers, and its updater reads
  them from there. Ours is mostly what it has no place for: a server that's an Erlang release with its NIFs, Bun, the
  UI library and MinGit, split into code and a runtime published only as its inputs change; builds published to the
  network's bucket, each with the platform's Ed25519 signature that `one` checks; a Mac's client from whichever of
  GitHub's macOS runner and the Mac is first; TestFlight; a server and a client published as one update; and a build
  left alone where the network has one from this commit or a newer. Windows' signing there is SSL.com's alone, not
  jsign with Artifact Signing, and its installer is MSI, not ours with NSIS. It would take over only the client's
  cross-builds, about 40 lines, and move them from this machine's kept checkout and compiler cache to CI. Nothing in
  ours is worth consolidating meanwhile: `tooling/on-mac` is the one race, which `nifs` and `release` share already.
- **Elixir WebRTC** for reaching by key: browsers would reach a home server directly and shells would need no
  networking code. Set aside: reaching by key would be ours to add, its relay can't be a Worker, the first connection
  is slower, and large transfers over data channels are its weak spot.
- **redb, or SQLite, for what a client keeps** (19 November 2026): a client's folder is shared by every program of
  it at once, the app, each `one` and each `one mcp` (257 on one Linux machine that day) and an iPhone's notification
  extension. redb lets one process open its file: a second gets `DatabaseAlreadyOpen`. SQLite takes many, but holding
  its lock in an iPhone's App Group gets the app killed as it's suspended (`0xdead10cc`), and it would replace about
  30 lines. A file per answer, written whole and renamed, is what many programs at once need.
- **DuckDB for reading Telemetry** (19 November 2026): through `duckdbex` and the `otlp` community extension it reads
  the OTLP protobuf kept, an hour's exports one after the other too: 200,000 spans, 13.6 MB, in 157 ms. Set aside: what
  `Telemetry.Read` gives is ours either way, error kinds, who a call was from, quantiles from explicit buckets, so it
  would shrink by about a third, not most; `Telemetry.Alerts` reads the same exports, so `Telemetry.Otlp` stays; a
  blob goes through its artifact's codec, so it would be written to a file for DuckDB on every look; and it adds DuckDB's
  C++ to every server, with no build of `duckdbex` for Windows and the extension fetched at run time. Telemetry's
  slow looks that day were spent passing the call between servers, which DuckDB doesn't touch. Worth it once more is
  kept than a server reads in memory.
- **restic and kopia for Backups** (19 November 2026): both back up files on a disk, or one stream, into a repository
  they reach with its own keys. A network's bucket isn't files: a server reaches it only through the edge's short-lived
  links, one blob at a time, so either would need the bucket mounted with its keys, or a snapshot of their own per
  blob. Both encrypt with a password of their own beside the Encryption app's keys, and restore to a disk, so `one
  restore` would need their program, that password, and a second step into the bucket. Backups already sends each
  blob only once, by what's in it, and reads only blobs whose tag changed.

## Chosen

- **iroh**: reaching a machine by its key, directly where possible and through a relay otherwise. Today's app links
  its devices this way, with the relay as a Worker. Chosen on 14 November 2026.
- **Better Auth** (1.7): signing in, with passkeys, and the OAuth provider MCP's authorization asks for, all in one
  Worker on D1. The old PTC app runs on it. Chosen on 14 November 2026, and run. Its OAuth provider ties an
  assistant's code to the browser session that said yes, so that browser stays signed in until the code is used.
  Its support for client ID metadata documents wants a fetch that pins the address a name resolved to, which a
  Worker's fetch can't, so assistants register themselves, which MCP still allows.

- **Executor** (executor.sh, `UsefulSoftwareCo/executor`, MIT): an MCP server that gives agents many apps' tools
  through a few of its own: `skills`, `search`, `describe`, `call`, `execute` and `resume`. Its `search` is BM25 over
  each tool's path, description and labels, and the `search()` inside an `execute` script ranks the same; `call`
  makes one call or several at once with no script. Chosen on 16 November 2026 as how agents find and call tools
  here: code mode is `search`, `call`, `execute` and `skills` as it has them, and `search` ranks as OpenCode's
  `search()` in a script does, with an app's description as its namespace's, so the two agree. MCP without code
  mode lists every tool and `skills`, since clients there search tools themselves. Left out: `describe`, as our
  `search` gives each tool's whole signature, and `resume`, as nothing of ours pauses a script for a person to
  answer. Its index of tools kept as rows in a database neither: a network has hundreds of tools, not thousands, so
  they're ranked in memory as they're asked for.

## Decided earlier, and easy to miss

- **How devices link.** Five options were compared in a table earlier in November 2026: A, everything through the
  network's edge; B, A plus direct on the same local network; C, B plus one simple attempt across the internet;
  D, C plus WebRTC for browsers; E, iroh. The choice was B, with C held back until streaming a screen across the internet
  is common. Most traffic is small requests, and the case to make fastest is an AI on its user's own computer. On 14
  November 2026 the choice moved to E: every server and client is reached by its key.
- **"Without needing the internet"** in the vision means two devices on one local network can reach each other. It
  never meant a network works with its bucket unreachable.
- **Trust.** A network is trusted, and its users aren't assumed to be malicious. Keeping apps apart is wise, but it's
  a bonus: other things decide what apps run in first. An app's code on a server is free, and nothing checks it.
  Agents are intelligent, and a network is trusted. Letting a computer be a server for a user or a group means
  trusting the apps they add with that computer.
- **Encryption.** Encrypting everything ourselves isn't built: a mistake there would lose everything, where a mistake
  with an encrypted artifact loses something small.
- **Signing in.** Signing in only says who a user is. No key comes from it, so how a network signs in can change.
- **The Erlang VM.** The VM takes up to a second to start, which a cloud server started on demand pays each time. No
  official build of the Erlang VM for Windows on ARM was found. The x64 build likely runs there emulated.

## Not built yet

- A server runs on Windows as well as Mac and Linux. On Windows it may run through WSL, if that doesn't weigh on the
  computer and an agent can set it up. To test first: iroh through a Rust NIF on Windows: 1 day.
- The web client is static files, served at the network's address. A sharing link opens in the web client, where
  there is one.
- Left open until the rest lands: what starts a network's cloud server when no other server answers for a user or
  group. It's a big choice, and fewer other things should be moving first.
- A server can keep a copy of what a server acting for an artifact's owner answered it.
- If search reaches past tools to what apps keep, it's SQLite's FTS5, which the database app's SQLite has built in,
  not more of `OneCore.Search`'s ranking in memory.
- To test first: an app in Tauri's web view on a mid-priced Android phone, opening, scrolling and going back: 2 to 3
  days.
- To test first: a phone on mobile data reaching a home server: how often directly, and how long the first connection
  takes: 1 day.

## Still to come

Not designed yet: how a client's offline writes reach a server.

Claims is the pressing need, but a bad platform would mean another rebuild, so the platform comes first.

Later, and not critical: bringing data across from the app as it runs today and from the old PTC app.

Parked, to keep in mind: the public as a who, anyone at all and not signed in, as a `viewer` of what an owner makes
public. Filling in something like a form, and visitors without an account, come after that.
