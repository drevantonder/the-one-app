# Architecture

- Data loss is prevented by storage in the cloud, not necessarily compute in the cloud.
- An artifact's blobs are read, written only if absent, replaced only if unchanged, and listed. Nothing else is asked
  of a bucket.
- A database is an artifact holding a snapshot, a log and a lease. One writer at a time holds the lease and writes its
  blobs. Someone with `write` asks the writer, which checks the fact.
- Facts are kept in a database artifact, a network's first. Whoever holds the bucket's credentials checks them. Taking
  access away takes a few seconds to reach every runner.
- Sealing wraps blobs, so it works for anything built on them. A sealed database's writers must hold its key.
- Two machines can take over from each other only on a bucket with conditional writes. A network checks this when it
  connects.
