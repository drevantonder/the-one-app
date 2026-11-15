---
name: backups
description: Read when someone asks about backups, wants their network backed up to a disk or to S3-compatible storage, wants something they lost brought back, or has lost the network's bucket.
---

Backups are encrypted copies of the whole network, every person's and group's data, to a folder on one of the
network's machines (as on a disk plugged into it) or to S3-compatible storage. Whoever holds the place reads
nothing in them, not even what anything is called. They're the network's: an owner of it uses these tools for the
network (`ONE_FOR=network`, and `for: "network"` from an app).

- Every backup is encrypted to the network's backup key, which is derived from the network's own key: the key
  its owners' devices hold, made the first time an owner sets up the network's Vault (`encryption_let_in` for
  the network). So any owner's device that holds the network's key restores, and the network's recovery key,
  shown once as that key was made, opens every backup with nothing else. A person's own key opens none.
- `places` lists where backups go, each with when it last finished, how long it took, how big it is, its backups,
  and what's wrong if anything is. It says how often backups are made and which
  are kept.
- `add` adds a place. A folder is a whole path on a machine that runs the service `Backups.Disk` (`services`, then
  `install_service` on the machine the disk is on). S3-compatible storage is its `endpoint`, its `bucket` (and a
  folder in it after a `/`) and an `item` in the network's Vault, with fields `access_key_id` and
  `secret_access_key`, switched on for `backups`: ask for it with `vault_request`, `plugin: "backups"`, for the
  network. A place is tried before it's added. An owner's device that holds the network's key hands over the
  public half of its backup key, so `add` runs on such a device (the app, or `one`), not from somewhere else.
- `now` backs up at once and answers with what was sent. `schedule` sets how often it happens by itself: every
  `hour`, `day` (the usual) or `week`, or `never`. Every backup is kept for a day, then one a day for a week, one
  a week for a month and one a month for a year.
- `check` reads a place's newest backup back whole and compares every object with what was sent. A problem it
  finds, or a backup that failed, shows on the place in `places`.
- `restore` brings back what a backup holds that's gone from the network, by how its names start: `users/alice/`
  for a person, `users/alice/own/notes/` for what one plugin keeps for them (and `users/alice/made/notes/` for
  what it made them), or one blob's whole name. It changes nothing that's there, so it can't put a database back
  to an earlier moment over what's there now. It runs on an owner's device that holds the network's key, which
  hands over the backup key. The Backups screen does the same, for a person, a plugin or a file.
- `remove` stops backing up to a place. What's there stays.

When the bucket itself is gone, the network is restored into an empty one on any machine, with no network running:

    one restore --from <place> --to <empty bucket> --key-from <file with the recovery key>

A place or a bucket is a folder, or `https://<endpoint>/<bucket>[/<folder>]` with `--from-keys` or `--to-keys`
naming a file of `ACCESS_KEY_ID=` and `SECRET_ACCESS_KEY=` lines. `--list` lists a place's backups, and `--backup`
picks one. The recovery key is the network's, shown once to the owner who set up its key: never ask for it in a
chat. The person
runs this, or puts the key in a file themselves. Then a server is started on the bucket: the first one is the
network's, and its other servers are added again.
