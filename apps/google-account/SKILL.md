---
name: google-account
description: Read before signing a Google account in to Gmail, Google Calendar, Drive, Docs or Sheets, or out of one, and when one of those apps says an account needs signing in.
metadata:
  subtitle: "Sign in to Google once"
  about: "Signs your Google accounts in to Gmail, Google Calendar, Drive, Docs and Sheets, and out again. Each account you add is yours alone."
  category: Connectors
---

Each Google app (gmail, google-calendar, google-drive, google-docs, google-sheets) uses its person's Google
accounts through this one. `apps` lists the ones the person has.

- A Google account is one item of their Vault, "Google <address>", which every Google app they have uses. It's
  listed under Accounts in the Vault, which is where a person adds one (Add Account) and deletes one.
- `google-account_sign_in`, or a Google app's own `<app>_sign_in`, gives a link: give it to the person to open
  in their browser. Google signs them in, for every Google app they have, and sends them back to the network's
  address, which finishes it. Nothing is pasted back. With `account`, Google suggests that one: use it for an account
  that needs signing in again, which the Vault shows with a dot and Sign In.
- Signing out is deleting the account's item, which soft delete keeps for 30 days.
- Every other tool of a Google app takes `account`: an address, or a part only it has. Left out, it's the first
  `<app>_accounts` lists.

The first sign-in needs the item "Google client" in the person's Vault: `client_id` (not secret) and
`client_secret` of a Google Cloud OAuth client of the Web application kind, whose authorized redirect URI is the
network's address followed by `/vault/callback`, with `oauth2.googleapis.com` its site, where its secret goes. Ask
for it with `vault_request` (`hosts: ["oauth2.googleapis.com"]`); never ask for it in the chat.
