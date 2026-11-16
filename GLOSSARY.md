# Glossary

A bundled plugin's own words are defined in that plugin's `SKILL.md`.

- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients. A
  trusted boundary, not a social network. It is an artifact.
- **User**: someone in the network, as relation tuples name them. How they sign in is not part of who they are.
- **Group**: an artifact whose members share other artifacts. Being in one is a relation tuple: the group, `member`, and
  a user or another group.
- **Server**: a machine that runs a network's plugins and answers its clients. A computer at home, a colleague's, or a
  cloud machine started when needed. A network has one or more, and each acts for the network, or for one user or group
  alone. It is an artifact, holding its key and how to reach it, and is a subject.
- **Client**: what a user or their agent uses, on a phone, a computer or in a browser, or as the CLI or MCP. It asks a
  server, keeps what servers answered, and keeps secrets safe on its device.
- **Presigner**: the only holder of the bucket's credentials. It hands a server short-lived access to the blobs of those it
  acts for.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Plugin**: an artifact whose blobs are code. It provides any of: apps, widgets, tools, scheduled jobs, services, an
  edge function and kinds of artifact. It uses other artifacts, and is a subject. A git repo can be where its code comes from, but needn't be.
  Avoid: module.
- **App**: something with UI that the launcher shows, on a Home Screen page or in Settings. A plugin provides it. It can
  have server-side code where it needs it, which is its plugin's tools.
- **Launcher**: the app every client opens first, which draws the rest. A network has exactly one, in the category
  `launcher`, and Home Screen is it.
- **Sandbox mode**: a plugin run in a VM of its own, for each user, group or network it's installed for, behind its
  machine's sandbox, by any of the network's servers. So installing a stranger's plugin needs no server of one's own.
- **Widget**: what a plugin shows on a Home Screen page at a glance. The plugin returns a layout from a fixed list of parts, in
  a phone's own sizes, and each client draws it. So one widget shows in the client, on a phone's own home screen, and to
  an agent as text. A control in one calls a tool.
- **Tool**: a named function a plugin defines, which the plugin's code runs on a server. Apps, agents and scheduled jobs
  all call the same tools. Avoid: action.
- **Service**: a part of a plugin's server code that must run on particular machines, for what is only there: a Mac's
  messages, a signed-in browser, a GPU. An instance runs on each server whose owner installed it, and keeps running
  between calls. It and the rest of its plugin call each other's functions, which nothing else can call. The plugin
  decides which instance it calls.
- **Edge**: what of a network is always on and faces the internet, at the network's address: the gateway,
  the presigner, the relay, the web client's files, the network's front door (its MCP endpoint, for its own
  devices, whoever signed in and a sharing link's holder, and the `/llms.txt` that says how to come in), and each
  plugin's edge function.
- **Edge function**: a part of a plugin that runs at the network's address, for what comes in over the web: a
  page people sign in on, a call from an outside service. One runs for the whole network, with storage of its own
  there, and whoever sets up the edge puts it there with the edge's other pieces.
- **One Core**: what every plugin stands on: blobs, each plugin's own artifact, running plugins, and the interface
  they're written against. Small on purpose, and it works with no plugins. Avoid: the core.
- **Package**: one piece built separately: the server, a client, or a plugin.
- **Plugin hook**: a moment a plugin can be called at, such as a tool being called or an artifact changing. Each states
  whether a plugin observes it, mutates it, or validates it, as Kubernetes' admission webhooks do.
  One Core offers some, and a plugin offers its own, named with its name (`recently-deleted.removed`), for plugins it
  can't know of to hook into.
- **Sharing link**: a subject that is whoever holds it, without signing in. Making one writes a relation tuple naming it.
- **Gateway**: sends each request at the network's address where its path says.
- **Relay**: passes traffic between a network's servers and clients when they can't reach each other directly. It
  can't read it.
- **Scheduled job**: a tool call made by the clock.
- **Skill**: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. A plugin
  provides one as a file, and One Core has some of its own: building plugins,
  installing them and the UI library.
