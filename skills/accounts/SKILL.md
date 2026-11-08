---
name: {{server}}-accounts
description: Accounts in {{name}}'s apps (Gmail, Google Drive, Calendar, Docs, Sheets, WhatsApp, Messenger, Repos' GitHub and the rest) - listing them, signing one in or out, and picking which one a call uses. Use when a call says an account needs signing in, or the user wants to add or remove one.
---

# Apps and their accounts

Some apps sign in to an online account and bring it in (Gmail, the Google apps, a chat's linked phone, GitHub in Repos). Most have only tools, for agents and other apps: the user reads their mail in Gmail's own app as before.

Every app with accounts has the same tools:

- `<app>_accounts` lists its accounts, the default first, each `{ account, ok }`, with `why` when `ok` is false and it needs signing in again. With none, it says so, and what signs one in (a script gets an empty list).
- `<app>_sign_in` starts signing in another one. It gives a link for the user to open (in a browser, or on the device a chat app's phone is linked from). Give them the link; never sign in for them, and never ask for their password.
- `<app>_finish_sign_in`, where an app has one, takes the address the browser came back to, only if signing in didn't finish by itself.
- `<app>_sign_out` signs one out. It can't be undone, so it takes `confirm: true` once the user asked for it.
- Every other tool takes `account` (an address, a phone number, a browser profile, as `accounts` names it), the default one if you leave it out.

Accounts are kept in the Vault as items, so you never handle their tokens. The Google apps share them: one "Google sign-in", which an admin sets up once in Everyone's Vault for everyone (for anyone else, `vault_request` says which admin to ask, and that's all to tell them), and a "Google account" for each account, so an account signed in once serves every Google app switched on for it; another Google app may ask the user to allow a little more the first time.

An app over a device (iMessage, Reminders, Calendar and Contacts on a Mac; Shell; Models; Computer Use) has no accounts: `apps_devices` with its `app` says where it runs and which nodes it can, `apps_move` runs it on another, and `apps_remove` takes it off (`confirm: true`, once the user asked). Its other tools take `device` to pick one. With none to run on, a call says what it needs, like "Needs a Mac", and how to add it.

On the user's devices, each page has Accounts, in the sidebar and as a folder on its home page: a section for each app with accounts, listing them, with Add Account. Its Add Account for Another App opens the App Store's apps that sign in to an account. An app over a device isn't there: it's in the App Store.
