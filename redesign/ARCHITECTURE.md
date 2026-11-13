# Architecture

## What it has to be

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- One visual language across platforms, with no platform's own look. What a device does itself still follows the device:
  scrolling, the keyboard, the back gesture, its pickers and its share sheet.
- It runs on Mac, Linux, Windows, iPhone, Android and the web.
- It costs a family of four about one coffee a month, and an organisation less per person than that.
- A plugin, and anything it provides, isn't forced to behave a certain way. The platform gives it the freedom to do
  what's best, and gives guidance to the agents that build it so they make wise decisions.

## Storage

- An artifact's blobs are read, written only if absent, replaced only if unchanged, and listed. Nothing else is asked of
  a bucket.
- A database is an artifact holding a snapshot, a log and a lease. One server at a time holds the lease and writes its
  blobs. An `editor` asks that server, which checks the relation tuples.
- A server confirms a write only once losing that server couldn't lose the write.
- Only the signer holds the bucket's credentials. It hands servers and clients short-lived access to what they may
  reach, so removing a server changes nothing for the others.
- Two servers can take over from each other only on a bucket with conditional writes. A network checks this when it
  connects.
- A server without a connection to the bucket isn't in a normal state. Nothing is designed around it.
- Backups are what the bucket's provider offers: keeping older versions of blobs, and copying them to a second place.
  Nothing of ours is built for it.
- A deleted artifact can be brought back for 30 days. Deleting hides it, and its blobs are removed after that.
- Putting a database back to an earlier moment is the database plugin's to offer, from its log. It isn't built early.

## One Core and the packages

- One Core knows blobs. Every kind of artifact built on them, a database too, is something a plugin provides.
- The server is one package, and each client is its own: the CLI, MCP, the web client, and the apps for phones and
  computers.
- Each plugin is its own package too, the ones The One App ships with included. Those differ only in shipping with it.
- One Core works with no plugins. With none that decides who may reach what, everything in a network is shared.
- The One App ships with these plugins: signing in, relation tuples, the database, encryption, home pages and Insights.
- Plugins are written against one published interface, and reach nothing else of a server's or a client's.
- A plugin has parts that run on a server and parts that run in a client. The server gives each client the parts meant
  for it.

## Who may do what

- Everything a relation tuple is about is an artifact: the network, a group, a server, a plugin.
- A network is an artifact: the top `parent` of everything in it. Its first owner is the first user to sign in.
- A network's owner owns every artifact in it, but reads an encrypted one only with its key. Users have the privacy they
  need, and the owner isn't responsible for keeping their credentials safe.
- Relation tuples are a plugin. It keeps them in its own artifact, and decides whether anything may reach any other, on
  the server and for the signer.
- More than one plugin can decide whether something may be reached. Any of them can say no, and that blocks it.
- Servers and the signer ask the relation tuples plugin. Taking access away takes a few seconds to reach every server.
- The relation tuples plugin uses the database plugin.
- Every plugin has an artifact of its own in the network. Only that plugin reaches it, and it needs no relation tuple,
  because it's always that one plugin's.
- Any user can add a plugin. It's an artifact they own, and relation tuples say who else may use it.
- A plugin writes a relation tuple only where the user or group it runs for may.
- An `owner` or an `editor` of an artifact can share it, up to their own relation. Only an owner adds or removes an
  owner.
- Making a sharing link writes a relation tuple naming it as `viewer` or `editor`. Whoever holds the link gets that,
  without signing in, until the tuple is removed. A sharing link opens in the web client, where there is one.
- A history of who changed what, relation tuples included, is a plugin's to keep, through plugin hooks.

## Servers

- Any user can bring a server.
- A server's owner chooses its members, and each agrees. A new one is for its owner alone.
- A server acts for its members and reaches what they can. A viewer's server can keep a copy and read it. An editor's
  can hold a database's lease.
- With no server up, nothing is lost: clients keep what servers answered and wait.
- A server has a URL the network's address can reach. A server at home gets one by connecting out, so nothing at home is
  opened to the internet. How is its owner's choice.
- Leases are part of the database plugin.
- A tool call, a scheduled job and a call to a service all go to a server that acts for the user or group they're for.
  If it writes a database whose lease another server holds, that server passes it on to the one holding it.
- A scheduled job is a tool call made by the clock. The server holding the lease of the database its schedule is kept in
  makes the call.
- A plugin can use this same way of choosing a server for its own calls, a service's included, or choose its own.
- A server takes the newest release by itself, so none lags behind. One Core is small and stable, so that's seldom.

## Clients

- A client changes an artifact only by calling a tool, which goes straight to a server. Whatever an app can do, an agent
  can.
- A client keeps what servers have answered, shows it at once, and asks again behind it.
- A client asks again when its user opens or returns to an app, and after its own tool calls, as most apps do.
- A server can also tell a client that something changed, and the client asks again. A plugin is free to do more.
- A client has little of its own: it keeps what servers answered, keeps secrets safe on its device, and runs and draws
  what plugins give it.
- A client on the same local network as a server sends its requests straight to it. Otherwise it sends them to the
  network's address, where a small on-demand piece passes them to the server holding the lease. Going direct between
  different local networks would be nice, and stays open.
- That piece also signs access to the bucket and serves the web client. Nothing at the network's address keeps state or
  holds a connection open.

## Signing in

- Signing in is a plugin. It tells One Core who a request is from. With none, everyone who reaches a network is the same
  one user.
- Signing in only says who a user is. No key comes from it, so how a network signs in can change.
- A client that's signed in reaches everything its user may, except encrypted artifacts. Those need something that
  already holds the key to let it in.
- Signing in needs a server up. Clients already signed in keep showing what servers answered.

## Encryption

- Encryption is a plugin, with a part on the server and a part in each client. It wraps blobs, so it works for anything
  built on them, a database too.
- A server opens no encrypted artifact unless the user it belongs to, or the group's owner, chose to let it. That choice
  is a relation tuple naming the server, and gives the server the key.
- A server takes an encrypted database's lease only if it holds its key.

## Plugins

- A plugin always runs for a user or a group: whoever is using it, or, with nobody there, the one it was set up for.
- A plugin reaches an artifact only if both it and the user or group it runs for have a relation to it. It reaches what
  it makes without asking.
- What a plugin makes is owned by the user or group it was made for, not by the plugin. Removing a plugin leaves its
  data.
- A plugin's code reaches nothing except through the interface plugins are written against, apart from a service, which
  runs only on a server whose owner chose to install it.
- Everyone using a plugin runs its newest version.
- A plugin's tools are its only public part. Anything may call one for the user or group it runs for: an app, an agent,
  a scheduled job, another plugin's tool.
- A plugin says which other plugins it uses, so adding it brings them along.
- A plugin's deployed code is open to everyone in the network. A git repo it came from can be private.
- Every tool has a name, a description and its inputs, and is also a CLI command and an MCP tool. A plugin may also
  provide a tool guide: text for agents on when and how to use its tools. The CLI and MCP hand both over.
- Plugin hooks are part of the interface plugins are written against. A plugin hooks into a moment, such as a tool being
  called, an artifact changing or a server joining, and is called when it happens.
- Each plugin hook states whether a plugin only hears about it, can change it, or decides it.
- A plugin hook gives no access of its own. A plugin hears about an artifact only if it could already reach it.
- The database is a plugin.
- A plugin provides a kind of artifact as functions other plugins call. They run as the calling plugin, for the user or
  group it runs for.
- Home pages are a plugin. How they split between a server and the UI the clients share isn't worked out.
- Adding an app to a home page puts it on the clients of everyone who sees that page.
- An app opened from a group's home page works with the group's artifacts.
- Insights is a plugin. It keeps what every tool, service and server did and how long it took, in OpenTelemetry's
  shapes. It may be lost, and it holds no user data.
- The Vault is a plugin. Its secrets are encrypted artifacts, and a tool of its uses one on a server and hands back only
  the answer.
- Notifications are a plugin: getting one onto a device, what to send, an inbox, and what a user wants to hear about.

## Services

- A service runs for a user or a group, as the rest of its plugin does, and reaches artifacts by the same rule. One
  instance serves everyone its server acts for.
- A service keeps its data where its plugin chooses: in artifacts, on its machine, or both. In artifacts, a user or a
  group owns it and relation tuples say who else may use it.
- A service says what it needs of a machine, and its owner sees that before installing it.
- A call from a service into its plugin says which user or group it's for, and its server must act for them.
- A service follows its plugin's newest version.

## Updates and builds

- Plugins reach clients from a server, the ones The One App ships with included, so they update without a store release.
  Store releases are kept few, so iterating stays fast and cheap.
- The One App itself is made the way anyone's is: a name, an icon and a choice of plugins to ship with, on top of our
  packages. Someone who wants their own installs the same packages and builds theirs on them.
- Anything more than that is a fork, and forks should be few and far between.
- Starting a network to build or test with is cheap, fast and repeatable: a whole network on one machine, with a folder
  as its bucket.
