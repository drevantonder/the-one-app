# Architecture

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- An artifact's blobs are read, written only if absent, replaced only if unchanged, and listed. Nothing else is asked
  of a bucket.
- A database is an artifact holding a snapshot, a log and a lease. One server at a time holds the lease and writes its
  blobs. An `editor` asks that server, which checks the relation tuples.
- Only servers hold the bucket's credentials. A client asks a server, and keeps a copy of what its user may read.
- Relation tuples are kept in a database artifact, a network's first. Servers check them. Taking access away takes a
  few seconds to reach every server.
- With no server up, nothing is lost: clients keep their copies and wait.
- Encryption wraps blobs, so it works for anything built on them, a database too.
- A server opens no encrypted artifact unless the user it belongs to, or the group's owner, chose to let it. That
  choice is a relation tuple naming the server, and gives the server the key.
- A server takes an encrypted database's lease only if it holds its key.
- Two servers can take over from each other only on a bucket with conditional writes. A network checks this when it
  connects.
- An app reaches an artifact only if both it and the user running it have a relation to it. It reaches what it makes
  without asking.
- What an app makes is owned by the user or group it was made for, not by the app. Removing an app leaves its data.
- An app's code reaches nothing except through the platform.
