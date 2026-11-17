---
name: permissions
description: Read before sharing something, making a group or a sharing link, or when a call is refused for who may reach what, and before a plugin reaches what it didn't make.
metadata:
  subtitle: "Who can see and change what"
  about: "Share what you make with people, groups or a link, and choose who can see or change it. Make groups for your family or team."
  category: "Privacy & Security"
---

Permissions says who may do what, as relation tuples: an artifact, a relation, and who. A plugin reaches an
artifact only if a tuple says so, both for whoever it runs for and for the plugin. Without it, or another
plugin deciding, nobody but the network's first owner does anything.

Its words:

- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`owner`, `editor`, `viewer`,
  `installer`, `parent`, `member`) and a subject, as in SpiceDB and OpenFGA. An owner is an editor, and an editor is a viewer. A
  user, a group, everyone, a server, a plugin and the holder of a sharing link are all a subject. Whoever asks for a
  call is its actor. Avoid: fact, grant.

- An artifact is named by where its blobs are, and its name starts with whose it is:
  `users/alice/made/notes/7` is alice's, `groups/family/made/notes/2` is the group's. A plugin's tool that
  makes something answers with its name. A group is the artifact `groups/<name>`, and the network `network`.
  A plugin is whoever's deployed it, `users/alice/plugins/<name>`, and its own artifact is whoever's it was
  installed for, `users/alice/own/<name>`: the network's are `network/plugins/<name>` and `network/own/<name>`.
- Who is a user (`users/alice`), a group, meaning its members (`groups/family`), `everyone` in the network, a
  plugin by its artifact's name, or a sharing link's holder (`users/alice/links/...`).
- `owner`, `editor` and `viewer` are what someone may do: an owner is an editor, and an editor is a viewer.
  Whoever an artifact's name starts with owns it, a group's members own what was made for the group
  (`groups/family/made/...`), and the network's owner owns everything. Whoever has a relation to a user or a group
  has it to everything that's theirs: a group's owner owns its plugins and what they keep. `member` puts a user or
  a group in a group. `parent` names another artifact as who, and gives whoever has a relation to that one the
  same relation to this.
- `installer` of the network (or of a group) may install plugins for it, as its owner does, and nothing more an owner
  may: `share --artifact network --relation installer --who users/carol`, which only an owner writes or removes.
  `users --artifact network --relation installer` lists who may, owners among them; anyone in the network sees it.
- `share` writes a tuple and `unshare` removes one. An owner or an editor shares with viewers and editors and
  adds a group's members, and only an owner adds an owner or a parent. `shared` lists an artifact's tuples,
  and `check` says whether someone may reach it.
- `forget` removes every tuple that names someone taken out of the network, so whoever joins by their name next
  inherits nothing. Taking someone out (`sign-in_remove_person`) calls it, and it refuses anyone still in.
- `groups` lists the groups whoever asks is in, each with what it's called, its members and owners.
- `group` makes a group called what it's given (The Smiths is `groups/the-smiths`), with whoever asks as its
  owner and first member. A member has a call run for the group, with the group's artifacts:
  `ONE_SUBJECT=groups/family` before a command, and an app opened from the group's Home Screen page. Every user is a member
  of the network.
- `link` makes a sharing link to an artifact, as a viewer or an editor. Whoever holds the link reaches the
  artifact without signing in, through the tools that take a link's holder, until its tuple is removed with
  `unshare`. The link is answered once and kept nowhere, so pass it on then. It starts with whose the artifact
  is, and at the network's address it's `<the address>/link/<the link>`.
- A plugin reaches something made by another plugin only once it has a relation to it too:
  `share --artifact users/alice/made/notes/7 --relation viewer --who users/alice/plugins/reader`.
- Sharing, and taking it away, are a person's decisions. Do them when asked to, and say what you did.

In a plugin, nothing of this is called: `OneCore.Call.reach(call, name, "editor")` gives the artifact or
raises with the reason, and `OneCore.Call.reaches?/3` asks. A plugin whose own tools are for some users only
asks for a relation to its own artifact, such as who may approve:
`OneCore.Call.reaches?(call, OneCore.Artifact.name(call.own), "editor")`.
