---
name: home-screen
description: Read before putting an app or a widget on a Home Screen page, arranging one, answering what's on someone's, asking for a plugin to be installed for everyone, or giving a plugin accounts or a part of Settings.
---

Home Screen is the network's launcher: the app every client opens first, which draws the rest. It keeps how each
page is arranged: `page` gives it, `arrange` changes it, `store` lists the plugins that could be added and
`settings` the parts of Settings.

Its words:

- **Home Screen**: where apps and widgets sit, on its pages. A user has their own, and so does each group and the
  network. A user sees theirs, their groups' and the network's.
- A page has the apps of what's installed for whoever it's for, and the apps put on it with `add`: those of
  plugins in use for the network, which anyone may put on their own page and `remove` again. Adding one installs
  nothing.
- Installing a plugin puts its code on a server, so for the network only its owners and those they let
  (`installer`, see `permissions`) do it. Anyone else asks them with `request`: a plugin in the network, as `store`
  lists it, or one from a git repo by `source` (owner/repo) and `name`, with a short `reason`. They're notified,
  `requests` lists the requests, and `approve` or `decline` answers one, which tells the asker. Approving a plugin
  in the network installs it for everyone; approving one from a git repo answers the `one install` command to run
  first, and is done again once it's installed.

## Categories

An app's `category` says where it goes. With none, it's on a Home Screen page. `settings` puts it in Settings
instead, on no page. `launcher` is this one's.

## Accounts

A plugin that signs a person in to an outside account (Gmail, WhatsApp) has accounts. That's three tools, and
nothing more:

- `accounts` lists them, the default first: `[%{account: "sam@example.com", ok: true}]`, and when `ok` is false,
  `why`, a few words saying what's wrong. It may add fields of its own.
- `sign_in` signs one in, or with `account` signs that one in again. It answers `%{url: ...}`, the web address
  the person signs in at. One whose signing in needs a screen of its own (a QR code, a device to pick) gives
  `sign_in` that page as its `ui`, which is opened instead, with the account after a `#` when signing one in again.
- `sign_out` takes `account` and signs it out.

Its other tools take `account`, the first `accounts` lists when it's left out. It gives `title/0` and `icon/0` if
it has no app.

Accounts is one screen, a folder at the end of the first page: a section for each plugin with accounts, its
accounts with Sign In and Sign Out, and Add Account. A plugin with accounts is on a page only if it has an app a
person opens for something else.
