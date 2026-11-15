# Glossary

- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients. A
  trusted boundary, not a social network. It is an artifact.
- **User**: someone in the network, as relation tuples name them. How they sign in is not part of who they are.
- **Group**: an artifact whose members share other artifacts. Being in one is a relation tuple: the group, `member`, and
  a user or another group.
- **Server**: a machine that runs a network's plugins and answers its clients. A computer at home, a colleague's, or a
  cloud machine started when needed. A network has one or more. It is an artifact, holding its key and how to reach it,
  and is a subject.
- **Client**: what a user or their agent uses, on a phone, a computer or in a browser, or as the CLI or MCP. It asks a
  server, keeps what servers answered, and keeps secrets safe on its device.
- **Presigner**: the only holder of the bucket's credentials. It hands a server short-lived access to the blobs of those it
  acts for.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`owner`, `editor`, `viewer`,
  `parent`, `member`) and a subject, as in SpiceDB and OpenFGA. An owner is an editor, and an editor is a viewer. A user, a group, everyone, a server, a
  plugin and the holder of a sharing link are all a subject. Whoever asks for a call is its actor. Avoid: fact, grant.
- **Encrypted artifact**: an artifact whose blobs are encrypted before they're stored, so only holders of its key read
  it. For passwords and the keys that reach devices. Avoid: sealed.
- **Plugin**: an artifact whose blobs are code. It provides any of: apps, widgets, tools, scheduled jobs, services, an
  edge function and kinds of artifact. It uses other artifacts, and is a subject. A git repo can be where its code comes from, but needn't be.
  Avoid: module.
- **App**: something with UI that goes on a home screen. A plugin provides it. It can have server-side code where it needs
  it, which is its plugin's tools.
- **Home Screen**: where apps and widgets sit, on its pages. A user has their own, and so does each group and the network. A user sees
  theirs, their groups' and the network's.
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
  the presigner, the relay, the web client's files, the public MCP endpoint for a sharing link's holder, and each plugin's edge function.
- **Edge function**: a part of a plugin that runs at the network's address, for what comes in over the web: a
  page people sign in on, a call from an outside service. One runs for the whole network, with storage of its own
  there, and whoever sets up the edge puts it there with the edge's other pieces.
- **One Core**: what every plugin stands on: blobs, each plugin's own artifact, running plugins, and the interface
  they're written against. Small on purpose, and it works with no plugins. Avoid: the core.
- **Package**: one piece built separately: the server, a client, or a plugin.
- **Plugin hook**: a moment a plugin can be called at, such as a tool being called or an artifact changing. Each states
  whether a plugin observes it, mutates it, or validates it, as Kubernetes' admission webhooks do.
- **Lease**: the record of which server writes a database for now. One server holds it at a time. Part of the database
  plugin.
- **Sharing link**: a subject that is whoever holds it, without signing in. Making one writes a relation tuple naming it.
- **Gateway**: sends each request at the network's address where its path says.
- **Relay**: passes traffic between a network's servers and clients when they can't reach each other directly. It
  can't read it.
- **Scheduled job**: a tool call made by the clock.
- **Skill**: text for agents on when and how to use a plugin's tools, which an agent reads when it needs it. A plugin
  provides one as a file, and One Core has one of its own, on building plugins.
