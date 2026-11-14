# Architecture

## What it has to be

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- One visual language across platforms, with no platform's own look. What a device does itself still follows the device:
  scrolling, the keyboard, the back gesture, its pickers and its share sheet.
- It runs on Mac, Linux, Windows, iPhone, Android and the web.
- It's built for networks of up to about 20,000 users, all in one network with many servers: more than half its users
  bring their own, about one group in eight has one, and the network runs about one for every 50 to 100 users.
- Speed should feel like quality: fast and well-tuned, never the slowness people put up with in workplace software. It
  opens, scrolls, types and goes back as well as the best apps on a mid-priced phone, and a CLI command or an MCP call
  is held to the same standard.
- It costs a family of four about one coffee a month, which is about $5, and an organisation less per person than that.
  That isn't rigid: $10 a month is also fine.
- A plugin, and anything it provides, isn't forced to behave a certain way. The platform gives it the freedom to do
  what's best, and gives guidance to the agents that build it so they make wise decisions.

## Storage

- A database is an artifact holding a snapshot, a log and a lease. One server at a time holds the lease and writes its
  blobs. An `editor` asks that server, which checks the relation tuples.
- Each user's and group's blobs sit under a prefix of their own in the bucket, so a blob's path says whose it is.
- One Core keeps, for each user and group, the list of servers that act for them, under that prefix. A server on the
  list can add another. The first is written by a server acting for the network, and only if the list is absent.
- A client reads the bucket only through a server. For something large, a server asks the signer for a short-lived link
  and hands it to the client.
- Two servers can take over from each other only on a bucket with conditional writes. A network checks this when it
  connects.
- A server without a connection to the bucket isn't in a normal state. Nothing is designed around it.
- Backups are flexible. They're what the bucket's provider offers where it offers it, such as keeping older versions of
  blobs and copying them to a second place, and otherwise whatever is simplest.
- A deleted artifact can be brought back for 30 days. Deleting hides it, and its blobs are removed after that.
- Putting a database back to an earlier moment is the database plugin's to offer, from its log. It isn't built early.

## One Core and the packages

- One Core knows blobs. Every kind of artifact built on them, a database too, is something a plugin provides.
- The server is one package, and each client is its own: the web client, and the apps for phones and computers.
- The CLI and MCP are the app for computers without its window. A computer with the app has both, and a machine with no
  screen installs the same program.
- Each plugin is its own package too, the ones The One App ships with included. Those differ only in shipping with it.
- Every plugin runs the same way. The ones The One App ships with are no more special than anyone's. That way isn't a
  sandbox: a plugin's code on a server is free to use the machine it runs on.
- What plugins run in can give them things to import, such as cryptography or a database engine. One Core itself still
  holds no database code, so that stays flexible.
- A network is trusted, and its users aren't assumed to be malicious. Keeping plugins apart is wise, but it's a bonus:
  other things decide what plugins run in first.
- The One App ships with these plugins: signing in, relation tuples, the database, encryption, home pages, Insights,
  repos, the Vault and notifications.
- Plugins are written against one published interface for everything One Core gives them. In a client they reach
  nothing else. On a server they're also free to use the machine: its files, its programs and its network.
- A plugin's parts needn't be written in the same language. Its server part and its client part already aren't.
- A plugin's code runs on servers and in clients, and nowhere else.

## Who may do what

- Everything a relation tuple is about is an artifact: the network, a group, a server, a plugin.
- A network is an artifact: the top `parent` of everything in it. Its first owner is the first user to sign in.
- A network's owner owns every artifact in it, but reads an encrypted one only with its key. Users have the privacy they
  need, and the owner isn't responsible for keeping their credentials safe.
- Relation tuples are a plugin. It keeps them in its own artifact, and decides on a server whether anything may reach
  any other.
- Servers ask the relation tuples plugin. The signer doesn't: it only knows which servers act for whom. Taking access
  away reaches every server eventually. No time is promised.
- The relation tuples plugin uses the database plugin.
- A plugin is added for a user, for a group or for the network.
- Every plugin has an artifact of its own for each one it was added for. Only that plugin reaches it, and it needs no
  relation tuple, because it's always that one plugin's.
- That artifact belongs to whoever the plugin was added for, so only a server its owner chose holds its lease. What's
  added for the network is safe from its members, and what's added for a group is safe from that group's members.
- Any user can add a plugin. It's an artifact they own, and relation tuples say who else may use it.
- A plugin writes a relation tuple only where the user or group it runs for may.
- An `owner` or an `editor` of an artifact can share it, up to their own relation. Only an owner adds or removes an
  owner.
- Making a sharing link writes a relation tuple naming it as `viewer` or `editor`. Whoever holds the link gets that,
  without signing in, until the tuple is removed. A sharing link opens in the web client, where there is one.
- Undo follows what people ordinarily expect of an app. A plugin that wants more keeps its own history. What a tool did
  outside the network isn't ours to undo.
- A history of who changed what, relation tuples included, is a plugin's to keep, through plugin hooks.

## Servers

- Any user can bring a server.
- A server runs on Windows as well as Mac and Linux. On Windows it may run through WSL, if that doesn't weigh on the
  computer and an agent can set it up.
- A server's owner chooses its members, and each agrees. A new one is for its owner alone.
- A server acts for its members. It reads and writes their own blobs, and asks a server acting for the owner for
  anything else they may reach. It can keep a copy of what it was answered.
- Only a server acting for an artifact's owner holds its lease.
- With no server up, nothing is lost: clients keep what servers answered and wait.
- Every server and client has a key, and one reaches another by its key: straight to it where a path can be found, and
  through the relay otherwise. Nothing at home is opened to the internet.
- A tool call and a scheduled job go to the server acting for the user or group they're for. If it writes a database
  whose lease another server holds, that server passes it on to the one holding it.
- A scheduled job is a tool call made by the clock. The server holding the lease of the database its schedule is kept in
  makes the call.
- A scheduled job runs when a server is up. A network that has a cloud server uses it, started when there's work and
  stopped after. Nothing sets one up by itself, and with no server available nothing runs.
- A cloud server that's always ready is its owner's choice to pay for. Otherwise it should start fast without costing a
  fortune.
- A plugin can use this same way of choosing a server for its own calls, a service's included, or choose its own.
- A server takes the newest release by itself, so none lags behind. One Core is small and stable, so that's seldom.

## The network's address

- A network has one address. A few small pieces sit behind it, each named by its job: the gateway, the signer, the
  relay, the web client's files and the way in for outsiders. None runs plugin code, and none keeps what users make.
- An assistant running somewhere else, a sharing link and a call from an outside service come in at the address over
  the web, and are passed to a server by its key.

## Clients

- A client changes an artifact only by calling a tool, which goes straight to a server. Whatever an app can do, an agent
  can.
- A server can also tell a client that something changed, and the client asks again. A plugin is free to do more.
- MCP can also be hosted where an assistant running somewhere else reaches it, signing in with OAuth.
- A client has little of its own: it keeps what servers answered, keeps secrets safe on its device, and runs and draws
  what plugins give it.
- A plugin's app is web content, as an MCP App is. Every client draws it in the device's web view, so an app is written
  once.
- The web client is static files, served at the network's address.

## Signing in

- Signing in is a plugin. It tells One Core who a request is from. With none, everyone who reaches a network is the same
  one user.
- Signing in only says who a user is. No key comes from it, so how a network signs in can change.
- A client that's signed in reaches everything its user may, except encrypted artifacts. Those need something that
  already holds the key to let it in.
- Signing in needs a server up. Clients already signed in keep showing what servers answered.

## Encryption

- Whoever runs the bucket can read what's in it, as with any cloud provider. An encrypted artifact is for what should be
  safe even from them. Encrypting everything ourselves isn't built: a mistake there would lose everything, where a
  mistake with an encrypted artifact loses something small.
- Encryption is a plugin, with a part on the server and a part in each client. It wraps blobs, so it works for anything
  built on them, a database too.
- A server opens no encrypted artifact unless the user it belongs to, or the group's owner, chose to let it. That choice
  is a relation tuple naming the server, and gives the server the key.
- A server takes an encrypted database's lease only if it holds its key.

## Plugins

- A plugin always runs for a user or a group: whoever is using it, or, with nobody there, the one it was set up for.
- What a plugin makes is owned by the user or group it was made for, not by the plugin. Removing a plugin leaves its
  data.
- A plugin's code on a server is free, and nothing checks it. Agents are intelligent, and a network is trusted.
- Letting a computer be a server for a user or a group means trusting the plugins they add with that computer.
- Nothing on a server keeps one plugin's code from what another's can reach there. What keeps users and groups apart is
  which servers act for whom.
- Tools to check a plugin before it's deployed are recommended, not needed.
- Everyone using a plugin runs its newest version.
- A plugin's tools are its only public part. Anything may call one for the user or group it runs for: an app, an agent,
  a scheduled job, another plugin's tool.
- A plugin's deployed code is open to everyone in the network. A git repo it came from can be private.
- MCP Apps are supported from the start.
- An app is free in what it shows. A widget is as free as the platform it sits on allows, and we pass on as much of that
  freedom as we can.
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
- Services run on computers. A phone can't be called on demand, so it runs none. Services on phones stay open for later.

## Updates and builds

- Plugins reach clients from a server, the ones The One App ships with included, so they update without a store release.
  Store releases are kept few, so iterating stays fast and cheap.
- The One App itself is made the way anyone's is: a name, an icon and a choice of plugins to ship with, on top of our
  packages. Someone who wants their own installs the same packages and builds theirs on them.
- Nothing is shared between networks. Whoever wants an app on phones publishes their own, built on our packages, and
  sends its notifications. One general app that we run for every network may come later.
- Anything more than that is a fork, and forks should be few and far between.
