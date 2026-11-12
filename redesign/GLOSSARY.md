# Glossary

- **Network**: one user's, one family's or one workplace's whole: its users, its bucket, its servers and its clients.
  A trusted boundary, not a social network.
- **Server**: a machine that holds the bucket's credentials and answers a network's clients: it checks relation tuples
  and writes databases. A computer at home, a colleague's, or a cloud machine started when needed. A network has one or
  more.
- **Client**: what a user or their agent uses: the app on a phone, a computer or in a browser, the CLI, or MCP. It asks
  a server, keeps a copy of what its user may read, and holds the keys of the encrypted artifacts its user may read.
- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`read`, `write`, `control`,
  `parent`) and who. A user, a group, everyone and the holder of a link are all a who. Avoid: fact, grant.
- **Encrypted artifact**: an artifact whose blobs are encrypted before they're stored, so only holders of its key read
  it. For passwords and the keys that reach devices. Avoid: sealed.
