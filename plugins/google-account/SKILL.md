---
name: google-account
description: Read before signing a Google account in to Gmail, Google Calendar, Drive, Docs or Sheets, or out of one, and when one of those plugins says an account needs signing in.
metadata:
  subtitle: "Sign in to Google once"
  about: "Signs your Google accounts in to Gmail, Google Calendar, Drive, Docs and Sheets, and out again. Each account you add is yours alone."
---

Each Google plugin (gmail, google-calendar, google-drive, google-docs, google-sheets) has accounts: its own
`accounts`, `sign_in` and `sign_out`. None of them is an app.
`plugins` lists the ones the person has. Accounts, on their Home Screen, shows each plugin's accounts, with Add
Account, Sign In and Sign Out, which is where a person does this themselves.

- `<plugin>_sign_in` gives a link: give it to the person to open in their browser. Google signs them in and sends
  them back to the network's address, which finishes it. Nothing is pasted back. With `account`, Google suggests
  that one: use it for an account that needs signing in again.
- An account is one item of their Vault, "Google <address>", shared by every Google plugin signed in to it.
  Signing in to it for another plugin lets that plugin in too, and keeps the others. `<plugin>_sign_out` takes it
  from that plugin alone.
- Every other tool of a Google plugin takes `account`: an address, or a part only it has. Left out, it's the first
  `<plugin>_accounts` lists.

The first sign-in needs the item "Google client" in the person's Vault: `client_id` (not secret) and
`client_secret` of a Google Cloud OAuth client of the Web application kind, whose authorized redirect URI is the
network's address followed by `/vault/callback`, with `oauth2.googleapis.com` its site, where its secret goes. Ask
for it with `vault_request` (`hosts: ["oauth2.googleapis.com"]`); never ask for it in the chat.
