# Architecture

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- An artifact's blobs are read, written only if absent, replaced only if unchanged, and listed. Nothing else is asked
  of a bucket.
- A database is an artifact holding a snapshot, a log and a lease. One server at a time holds the lease and writes its
  blobs. An `editor` asks that server, which checks the relation tuples.
- Only the signer holds the bucket's credentials. It hands servers and clients short-lived access to what they may
  reach, so removing a server changes nothing for the others.
- A client changes an artifact only by calling a tool, which goes straight to a server. Whatever an app can do, an
  agent can.
- A client keeps what servers have answered, shows it at once, and asks again behind it.
- A server confirms a write only once losing that server couldn't lose the write.
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
- A plugin, and anything it provides, isn't forced to behave a certain way. The platform gives it the freedom to do
  what's best, and gives guidance to the agents that build it so they make wise decisions.
- A service runs for a user or a group, as the rest of its plugin does, and reaches artifacts by the same rule. One
  instance serves everyone its server acts for.
- A service keeps its data where its plugin chooses: in artifacts, on its machine, or both. In artifacts, a user or a
  group owns it and relation tuples say who else may use it.
- A service says what it needs of a machine, and its owner sees that before installing it.
- A call from a service into its plugin says which user or group it's for, and its server must act for them.
- A service follows its plugin's newest version.
- A server has a URL the network's address can reach. How is its owner's choice.
- A client on the same local network as a server sends its requests straight to it. Otherwise it sends them to the
  network's address, where a small on-demand piece passes them to the server holding the lease. Going direct between
  different local networks would be nice, and stays open.
- That piece also signs access to the bucket and serves the web client. Nothing at the network's address keeps state
  or holds a connection open.
- A server without a connection to the bucket isn't in a normal state. Nothing is designed around it.
- Adding an app to a home page puts it on the clients of everyone who sees that page.
- An app opened from a group's home page works with the group's artifacts.
- One visual language across platforms, with no platform's own look. What a device does itself still follows the
  device: scrolling, the keyboard, the back gesture, its pickers and its share sheet.
- It runs on Mac, Linux, Windows, iPhone, Android and the web.
- It costs a family of four about one coffee a month, and an organisation less per person than that.
- Every tool is also a CLI command and an MCP tool.
- Getting a notification onto a device is the platform's. What to send, an inbox and what a user wants to hear about
  are a plugin's.
- The Vault is a plugin. Its secrets are encrypted artifacts, and a tool of its uses one on a server and hands back
  only the answer.
- Insights is a built-in plugin. It keeps what every tool, service and server did and how long it took, in
  OpenTelemetry's shapes. It may be lost, and it holds no user data.
- A plugin's tools are its only public part. Anything may call one for the user or group it runs for: an app, an
  agent, a scheduled job, another plugin's tool.
- A plugin says which other plugins it uses, so adding it brings them along.
- A plugin's deployed code is open to everyone in the network. A git repo it came from can be private.
- The server is one package, and each client is its own: the CLI, MCP, the web client, and the apps for phones and
  computers.
- Each plugin is its own package too, built-in ones included. A built-in plugin differs only in shipping with every
  server.
- Plugins are written against one published interface, and reach nothing else of a server's or a client's.
- A plugin has parts that run on a server and parts that run in a client. The server gives each client the parts
  meant for it.
- A client has little of its own: it signs in, keeps what servers answered, keeps secrets safe on its device, and
  runs and draws what plugins give it.
- Encryption is a built-in plugin, with a part on the server and a part in each client.
- The core knows blobs and relation tuples. Every kind of artifact built on them, a database too, is something a
  plugin provides.
- The database is a built-in plugin.
- Home pages are a built-in plugin. How they split between a server and the UI the clients share isn't worked out.
- Hooks are part of the interface plugins are written against. A plugin hooks into a moment, such as a tool being
  called, an artifact changing or a server joining, and is called when it happens.
- Each hook states whether a plugin only hears about it, can change it, or decides it.
- A hook gives no access of its own. A plugin hears about an artifact only if it could already reach it.
- A plugin writes a relation tuple only where the user or group it runs for may.
- Sharing by link writes a relation tuple naming the link as `viewer` or `editor`. Whoever holds the link gets that,
  without signing in, until the tuple is removed.
