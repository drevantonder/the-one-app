# Glossary

- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients. A
  trusted boundary, not a social network. It is an artifact.
- **User**: someone in the network, as relation tuples name them. How they sign in is not part of who they are.
- **Group**: an artifact whose members share other artifacts. Being in one is a relation tuple: the group, `member`, and
  a user or another group.
- **Server**: a machine that runs a network's plugins and answers its clients. A computer at home, a colleague's, or a
  cloud machine started when needed. A network has one or more. It is an artifact, holding its key and how to reach it,
  and is a who.
- **Client**: what a user or their agent uses, on a phone, a computer or in a browser, or as the CLI or MCP. It asks a
  server, keeps what servers answered, and keeps secrets safe on its device.
- **Signer**: the only holder of the bucket's credentials. It hands servers short-lived access to what they may reach.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`owner`, `editor`, `viewer`,
  `parent`, `member`) and who. An owner is an editor, and an editor is a viewer. A user, a group, everyone, a server, a
  plugin and the holder of a sharing link are all a who. Avoid: fact, grant.
- **Encrypted artifact**: an artifact whose blobs are encrypted before they're stored, so only holders of its key read
  it. For passwords and the keys that reach devices. Avoid: sealed.
- **Plugin**: an artifact whose blobs are code. It provides any of: apps, widgets, tools, scheduled jobs, services and
  kinds of artifact. It uses other artifacts, and is a who. A git repo can be where its code comes from, but needn't be.
  Avoid: module.
- **App**: something with UI that goes on a home screen. A plugin provides it. It can have a server part if it needs
  one, which is its plugin's tools.
- **Home page**: where apps and widgets sit. A user has their own, and so does each group and the network. A user sees
  theirs, their groups' and the network's.
- **Widget**: what a plugin shows on a home page at a glance. The plugin returns a layout from a fixed list of parts, in
  a phone's own sizes, and each client draws it. So one widget shows in the client, on a phone's own home screen, and to
  an agent as text. A control in one calls a tool.
- **Tool**: a named function a plugin defines, which the plugin's code runs on a server. Apps, agents and scheduled jobs
  all call the same tools. Avoid: action.
- **Service**: native code a plugin provides, for what the sandbox can't do or what has no API on the internet. An
  instance runs on each server whose owner installed it, as its own process. It and the rest of its plugin call each
  other's functions, which nothing else can call. The plugin decides which instance it calls.
- **One Core**: what every plugin stands on: blobs, each plugin's own artifact, running plugins, and the interface
  they're written against. Small on purpose, and it works with no plugins. Avoid: the core.
- **Package**: one piece built separately: the server, a client, or a plugin.
- **Plugin hook**: a moment a plugin can be called at, such as a tool being called or an artifact changing. Each states
  whether a plugin only hears about it, can change it, or decides it.
- **Lease**: the record of which server writes a database for now. One server holds it at a time. Part of the database
  plugin.
- **Sharing link**: a who that is whoever holds it, without signing in. Making one writes a relation tuple naming it.
- **Scheduled job**: a tool call made by the clock.
- **Tool guide**: text a plugin provides for agents, on when and how to use its tools.
