# Glossary

- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients.
  A trusted boundary, not a social network. It is an artifact, holding its relation tuples.
- **User**: someone in the network, as relation tuples name them. How they sign in is not part of who they are.
- **Group**: an artifact whose members share other artifacts. Being in one is a relation tuple: the group, `member`,
  and a user or another group.
- **Server**: a machine that answers a network's clients: it checks relation tuples and writes databases. A computer
  at home, a colleague's, or a cloud machine started when needed. A network has one or more. It is an artifact, holding
  its key and how to reach it, and is a who.
- **Client**: what a user or their agent uses, on a phone, a computer or in a browser, or as the CLI or MCP. It asks
  a server, keeps a copy of what its user may read, and holds the keys of the encrypted artifacts its user may read.
- **Signer**: the only holder of the bucket's credentials. It hands servers and clients short-lived access to what they
  may reach.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`owner`, `editor`,
  `viewer`, `parent`, `member`) and who. An owner is an editor, and an editor is a viewer. A user, a group, everyone, a
  server, a plugin and the holder of a link are all a who. Avoid: fact, grant.
- **Encrypted artifact**: an artifact whose blobs are encrypted before they're stored, so only holders of its key read
  it. For passwords and the keys that reach devices. Avoid: sealed.
- **Plugin**: an artifact whose blobs are code. It provides any of: apps, widgets, tools, scheduled jobs and services.
  It uses other artifacts, and is a who. A git repo can be where its code comes from, but needn't be. Avoid: module.
- **App**: something with UI that goes on a home screen. A plugin provides it. It can have a server part if it needs
  one, which is its plugin's tools.
- **Tool**: a named function a plugin defines, which the plugin's code runs on a server. Apps, agents and scheduled
  jobs all call the same tools. Avoid: action.
- **Service**: native code a plugin provides, for what the sandbox can't do or what has no API on the internet. An
  instance runs on each server whose owner installed it, as its own process. It and the rest of its plugin call each
  other's functions, which nothing else can call. The plugin decides which instance it calls.
