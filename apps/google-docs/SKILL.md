---
name: google-docs
description: Read before reading, making or writing in the person's Google Docs.
metadata:
  title: "Google Docs"
  subtitle: "Your Google Docs"
  about: "Your AI reads your Google Docs, writes in them and makes new ones."
  category: Connectors
---

Google Docs is the person's documents. Signing in is the skill `google-account`'s.

- `read` reads a document as markdown, or text. `create` makes one, `append` adds text at its end, and
  `replace` replaces every match of some text: change only what the person asked.
- A document is given as `documentId`, its id or its address. `google-drive_files_list` finds one.
- With several accounts, each tool takes `account`. `accounts` lists them.
