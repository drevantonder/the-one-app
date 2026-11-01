---
name: {{server}}-connectors
description: Accounts in {{name}}'s connectors (Gmail, Google Drive, Calendar, Docs, Sheets, WhatsApp, Messenger, iMessage and the rest) - listing them, signing one in or out, and picking which one a call uses. Use when a call says an account needs signing in, or the person wants to add or remove one.
---

# Connectors and their accounts

A connector is an app that brings something in from outside: an online account (Gmail, a chat's linked phone), a Mac's own app (Calendar, iMessage), or what a device can do (a shell, local models, a computer to use). Agents and other apps use them; the person reads their mail in Gmail's own app as before.

Every connector with accounts has the same operations:

- `<app>_accounts` lists its accounts, the default first, each `{ account, ok }`, with `why` when `ok` is false and it needs signing in again.
- `<app>_sign_in` starts signing in another one. It gives a link for the person to open (in a browser, or on the device a chat app's phone is linked from). Give them the link; never sign in for them, and never ask for their password.
- `<app>_finish_sign_in`, where an app has one, takes the address the browser ended up on, if the person pastes it back.
- `<app>_sign_out` signs one out. It can't be undone, so it takes `confirm: true` once the person asked for it.
- Every other operation takes `account` (an address, a phone number, a browser profile, as `accounts` names it), the default one if you leave it out.

Accounts are kept in the Vault as items, so you never handle their tokens. The Google apps share them: one "Google sign-in" and a "Google account" for each account, so an account signed in once serves every Google app switched on for it; another Google app may ask the person to allow a little more the first time.

A connector over a device (iMessage, Reminders, Calendar and Contacts on a Mac; Shell; Models) has the devices it runs on as its accounts. One with nothing to run on says what it needs, like "Needs a Mac".

On the person's phone or Mac, each page has a Connectors folder listing them, and Add a Connector, which opens the App Store's Connectors.
