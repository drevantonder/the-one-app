---
name: drive
description: Read before finding, uploading, renaming, moving or trashing files in the person's Google Drive.
---

Google Drive is the person's Drive, through Drive's own API (v3) by its own names. Signing in is the skill
`google`'s.

- `files_list` finds files with Drive's `q` search (name contains 'budget' and trashed = false). `files_get`
  gives one. Answers have the fields asked for, and `fields` asks for more.
- `files_create` makes a folder, `files_upload` uploads a file (`upload`, the name the `upload` tool gave it,
  or `data` in base64 for a small one), and `files_update` renames, moves (addParents, removeParents) or trashes
  one. `generate_ids` gives ids to make files with, so a retry never makes two.
- A Google Doc or Sheet's contents are read with `docs` and `sheets`.
- With several accounts, each tool takes `account`. `accounts` lists them.
