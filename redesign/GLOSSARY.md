# Glossary

- **Artifact**: a couple of blobs together in the bucket, with who may do what to them. Everything a network keeps is
  built on top of one: files, a database, a git repo. Avoid: space.
- **Relation tuple**: one line of who may do what, as in Zanzibar: an artifact, a relation (`read`, `write`, `control`,
  `parent`) and who. A user, a group, everyone and the holder of a link are all a who. Avoid: fact, grant.
- **Encrypted artifact**: an artifact whose blobs are encrypted on a device before they're stored, so only holders of
  its key read it. For passwords and the keys that reach devices. Avoid: sealed.
