# Glossary

An app's own words are defined in that app's `SKILL.md`.

- **the One App**: the product's name, spelled with a lowercase "the" everywhere, even at the start of a sentence or a
  title, as iPhone and eBay keep their first letter: "open the One App", "the PTC App". "One App" alone, "The One App"
  and "one app" are wrong. Only the App Store listing, "One App to rule them all", is a different name.
- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients. A
  trusted boundary, not a social network. It is an artifact.
- **User**: someone in the network, as relation tuples name them. How they sign in is not part of who they are.
- **Group**: an artifact whose members share other artifacts. Being in one is a relation tuple: the group, `member`, and
  a user or another group.
- **Server**: a machine that runs a network's apps and answers its clients. A computer at home, a colleague's, or a
  cloud machine started when needed. A network has one or more, and each acts for the network, or for one user or group
  alone. It is an artifact, holding its key and how to reach it, and is a subject.
- **Client**: what a user or their agent uses, on a phone, a computer or in a browser, or as the CLI or MCP. It asks a
  server, keeps what servers answered, and keeps secrets safe on its device.
- **Presigner**: the only holder of the bucket's credentials. It hands a server the blobs of those it acts for, and
  short-lived access to them.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **App**: what you install and remove; it has one or more app entrypoints. It's an artifact whose blobs are code,
  and provides any of: app entrypoints, widgets, tools, scheduled jobs, services, an edge function and kinds of
  artifact. It `requires` other apps, its requirements, and is a subject. A git repo can be where its code comes from,
  but needn't be. Avoid: plugin, module.
- **Owner**: whoever something belongs to: a user, a group or the network, as an artifact's name says. An app is
  installed for an owner (`for`), and each owner has a Home Screen. Not the relation `owner`, which says who may share
  what an owner has.
- **App entrypoint**: an icon an app puts in the launcher, with its name, category and page; people just see apps.
  It's on a Home Screen page or in an App Library, and in code it's an `entrypoint`.
- **Background app**: an app with no app entrypoint, such as Gmail or Database: its tools, jobs and services work
  for other apps and agents. It's only in the App Library, under Background Apps, never on a page.
- **Account**: a user's account at an outside service (a Gmail address, a WhatsApp number), which they sign an app
  in to so its tools act there as them. It's a word for people: what's kept is an item in their Vault, of a type the
  app registers, which the Vault lists under Accounts. An account isn't an app entrypoint, and an app doesn't
  get one for having accounts.
- **Launcher**: the app every client opens first, which draws the rest. A network has exactly one, in the category
  `launcher`, and Home Screen is it.
- **Rail**: on a phone or a tablet, what's above every app, its way back to the Home Screen: a home line on the right
  edge over an app, iOS's home indicator on its side, which a finger pulls in to go home. A computer has none: an app
  window's breadcrumb goes home instead.
- **App window**: on a computer (Mac, Windows, Linux), an app open in a window of its own, which the One App's one
  process draws natively, the app in a sandbox of its own: while one is open, the One App has one icon in the Dock
  or taskbar and one entry in Cmd-Tab or Alt-Tab, with every app's window under it, and on a Mac the menu bar is the
  front window's app's. It asks the network through the One App, which keeps the Home Screen, and each app's shortcut
  (`shortcuts`) opens it. Its title bar has a breadcrumb, whose Home goes to the Home Screen. In code, it's a native
  window (`native.rs`), and the host (`host.rs`) is what an app's shortcut starts, which asks the One App to open it.
- **App Store**: the app that installs and removes apps, in the category `app-store`, which a client opens itself
  where there's no launcher yet to open it from.
- **Trusted**, **Untrusted**: the state of an installed app, apart from where it is in its lifecycle, and either
  can become the other. A trusted app runs in its server's VM, on a server of whoever it's installed for. An
  untrusted app runs in a sandbox: a VM of its own, for each user, group or network whose data it keeps, by any of
  the network's servers. So installing a stranger's app needs no server of one's own. The sandbox is only where an
  untrusted app runs (bubblewrap, Seatbelt, AppContainer), never what an app is. Avoid: sandboxed, sandbox mode.
- **Widget**: what an app shows on a Home Screen page at a glance. The app returns a layout from a fixed list of parts, in
  a phone's own sizes, and each client draws it. So one widget shows in the client, on a phone's own home screen, and to
  an agent as text. A control in one calls a tool.
- **Tool**: a named function an app defines, which the app's code runs on a server. Apps, agents and scheduled jobs
  all call the same tools. Avoid: action.
- **Service**: a part of an app's server code that must run on particular machines, for what is only there: a Mac's
  messages, a signed-in browser, a GPU. An instance runs on each server whose owner installed it, and keeps running
  between calls. It and the rest of its app call each other's functions, which nothing else can call. The app
  decides which instance it calls.
- **Edge**: what of a network is always on and faces the internet, at the network's address: one Worker
  with the presigner, the relay, the web client's files, the network's front door (its MCP endpoint, for its own
  devices, whoever signed in and a sharing link's holder, and the `/llms.txt` that says how to come in); and each
  app's edge function, a Worker of its own.
- **Edge function**: a part of an app that runs at the network's address, for what comes in over the web: a
  page people sign in on, a call from an outside service. One runs for the whole network, with storage of its own
  there, and whoever sets up the edge puts it there with the edge's other pieces.
- **One Core**: what every app stands on: blobs, each app's own artifact, running apps, and the interface
  they're written against. Small on purpose, and it works with no apps. Avoid: the core.
- **Package**: one piece built separately: the server, a client, or an app.
- **App hook**: a moment an app can be called at, such as a tool being called or an artifact changing. Each states
  whether an app observes it, mutates it, or validates it, as Kubernetes' admission webhooks do, or collects it,
  each app answering its own part, where one that can't answer is left out.
  One Core offers some, and an app offers its own, named with its name (`soft-delete.moved`), for apps it
  can't know of to hook into.
- **Soft delete**: what a network does with an artifact someone deletes: it's kept, as it was, so it can be restored
  for 30 days, and then purged, as Rails' and Laravel's soft deletes keep a row. An artifact is **active**, then
  **deleted**, **purging** while it's being deleted for good, and **purged**. Uninstalling an app deletes what it
  kept and made for whoever it was installed for, together. People see it as Recently Deleted, the app's name.
- **Sharing link**: a subject that is whoever holds it, without signing in. Making one writes a relation tuple naming it.
- **Gateway**: sends each request at the network's address where its path says.
- **Relay**: passes traffic between a network's servers and clients when they can't reach each other directly. It
  can't read it.
- **Network update**: how a network takes each build published to it, by itself, which its edge's updater runs, one
  per network (`core/edge/updater/updater.ts`): every awake server prepares the build, then each restarts on it,
  one at a time, never the last ready one. A newer build takes the place of one under way, a server that goes back holds it, and one asleep or gone is
  caught up once it's back. Its state is `network_update_status`, beside each server's `server_update_status`.
  People see it as Software Update. Avoid: rollout, release (for what servers do; a release is what's published).
- **Scheduled job**: a tool call made by the clock.
- **Skill**: text for agents on when and how to use an app's tools, which an agent reads when it needs it. An app
  provides one as a file, and One Core has some of its own: building apps,
  installing them and the UI library.
