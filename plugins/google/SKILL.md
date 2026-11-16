---
name: google
description: Read before signing a Google account in to Gmail, Google Calendar, Drive, Docs or Sheets, or out of one, and when a Google app says an account needs signing in.
---

Each Google app (gmail, google-calendar, drive, docs, sheets) has its own `accounts`, `sign_in` and `sign_out`.
`apps` lists the ones the person has. Their Google screen shows each app's accounts, with Add Account and Sign
Out, which is where a person does this themselves.

- `<app>_sign_in` gives a link: give it to the person to open in their browser. Google signs them in and sends
  them back to the network's address, which finishes it. Nothing is pasted back. With `account`, Google suggests
  that one: use it for an account that needs signing in again.
- An account is one item of their Vault, "Google <address>", shared by every Google app signed in to it.
  Signing in to it for another app lets that app in too, and keeps the others. `<app>_sign_out` takes it from
  that app alone.
- Every other tool of a Google app takes `account`: an address, or a part only it has. Left out, it's the first
  `<app>_accounts` lists.

The first sign-in needs the item "Google client" in the person's Vault: `client_id` (not secret) and
`client_secret` of a Google Cloud OAuth client of the Web application kind, whose authorized redirect URI is the
network's address followed by `/vault/back`. Ask for it with `vault_request`; never ask for it in the chat.
