# Architecture

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- An artifact's blobs are read, written only if absent, replaced only if unchanged, and listed. Nothing else is asked
  of a bucket.
- A database is an artifact holding a snapshot, a log and a lease. One server at a time holds the lease and writes its
  blobs. An `editor` asks that server, which checks the relation tuples.
- Only the signer holds the bucket's credentials. It hands servers and clients short-lived access to what they may
  reach, so removing a server changes nothing for the others.
- A client changes an artifact only by calling a tool, which goes straight to a server. Whatever a screen can do, an
  agent can.
- A client's copy holds only what a server wrote. Screens read it directly.
- A client copies an artifact when an app first opens it.
- Everything a relation tuple is about is an artifact: the network, a group, a server, a plugin.
- A network is an artifact: the top `parent` of everything in it, holding its relation tuples. Its first owner is the
  first user to sign in.
- A network's owner owns every artifact in it, but reads an encrypted one only with its key. Users have the privacy
  they need, and the owner isn't responsible for keeping their credentials safe.
- Servers and the signer check relation tuples. Taking access away takes a few seconds to reach every server.
- Any user can bring a server.
- A server's owner chooses its members, and each agrees. A new one is for its owner alone.
- A server acts for its members and reaches what they can. A viewer's server can keep a copy and read it. An editor's
  can hold a database's lease.
- With no server up, nothing is lost: clients keep their copies and wait.
- Encryption wraps blobs, so it works for anything built on them, a database too.
- A server opens no encrypted artifact unless the user it belongs to, or the group's owner, chose to let it. That
  choice is a relation tuple naming the server, and gives the server the key.
- A server takes an encrypted database's lease only if it holds its key.
- Two servers can take over from each other only on a bucket with conditional writes. A network checks this when it
  connects.
- A plugin always runs for a user or a group: whoever is using it, or, with nobody there, the one it was set up for.
- A plugin reaches an artifact only if both it and the user or group it runs for have a relation to it. It reaches
  what it makes without asking.
- What a plugin makes is owned by the user or group it was made for, not by the plugin. Removing a plugin leaves its data.
- A plugin's code reaches nothing except through the platform, apart from a service, which runs only on a server whose
  owner chose to install it.
- Everyone using a plugin runs its newest version.
- Signing in only says who a user is. No key comes from it, so how a network signs in can change.
- A client that's signed in reaches everything its user may, except encrypted artifacts. Those need something that
  already holds the key to let it in.
- Signing in needs a server up. Clients already signed in keep working from their copies.
- Plugins, tools and services aren't forced to behave a certain way. The platform gives them the freedom to do what's
  best, and gives guidance to the agents that build them so they make wise decisions.
- A service runs for a user or a group, as the rest of its plugin does, and reaches artifacts by the same rule. One
  instance serves everyone its server acts for.
- A service keeps its data where its plugin chooses: in artifacts, on its machine, or both. In artifacts, a user or a
  group owns it and relation tuples say who else may use it.
- A service says what it needs of a machine, and its owner sees that before installing it.
