---
name: gmail
description: Read before searching, reading, labelling, drafting or sending email in the person's Gmail accounts.
---

Gmail is the person's Gmail accounts. Signing in is the skill `google`'s.

- `search` finds messages with Gmail's search syntax in `q` (from:, is:unread, in:inbox, newer_than:2d, ...).
  `read` reads one and `thread` a whole conversation. `attachment` gives one of a message's files.
- `label` archives (remove INBOX), marks read (remove UNREAD), stars or labels messages. `trash` moves them to
  the trash, where they stay 30 days.
- Write with `draft` and let the person send it, unless they asked you to send: `send` and `send_draft` send real
  email to real people. A reply is `reply_to` a message's id: it goes in its thread, to its sender.
- An attachment is `base64`, or for a large file, `upload`: the name the `upload` tool gave it.
- With several accounts, each tool takes `account`. `accounts` lists them.
