---
name: home-screen
description: Read before putting an app or a widget on a Home Screen page, arranging one, answering what's on someone's, or giving a plugin's app a category, accounts or the user it's about.
metadata:
  subtitle: "Your pages of apps"
  about: "Your Home Screen, everyone's and your groups': pages of apps and widgets, arranged your way, and an App Library of every app you have, by category."
---

Home Screen is the network's launcher: the app every client opens first, which draws the rest. It keeps how each
owner's pages are arranged: `pages` gives an owner's Home Screen, `arrange` changes its pages, `add` and `remove` put
an app on them or take it off, and `store` lists the plugins that could be added. A person edits their pages as on a
phone: touch and hold, or right-click, then Edit Home Screen.

Its words:

- **Home Screen**: an owner's pages, then their App Library. Its owner is a user, a group, or the network, which a
  person reads as Everyone. A user sees theirs, everyone's and their groups', and moves between them with the owner
  switcher.
- **Page**: a fixed grid of apps and widgets, four across and six down, which never scrolls: what doesn't fit goes on
  the next. It holds only what was put there. A page can be hidden, kept but not shown, and a page with nothing on it
  goes. An owner's pages are one layout, so arranging everyone's or a group's arranges it for everyone who sees it.
  The user themselves arranges theirs, and a group's or the network's `owner`s theirs (`arranges`).
- **App Library**: every app installed for an owner, by category, after their pages. An app installed lands there,
  not on a page.
- An app is in the App Library of whoever it's installed for, and in no other. One about whoever opens it
  (`subject: "user"`) is in the user's own App Library wherever it's installed, and opens for them. A plugin a user
  installed for themselves too is in theirs and not in everyone's; a group's own install keeps the group's data, so
  it's in the group's.
- Installing a plugin puts its code on a server, so for the network only its owners and those they let (`installer`,
  see `permissions`) do it, and for a group, its owners. Whoever can't asks one of them themselves.

## Categories

An app says its `category`, one of these, as a desktop's menu entry does:

| `category` | Shown as |
|---|---|
| `coding` | Coding |
| `device-automation` | Device Automation |
| `admin` | Admin |
| `appearance` | Appearance |
| `agents` | Agents |
| `devices` | Devices |
| `access` | Access |

An App Library shows them in that order. An app that says none of these is in Other, last. `launcher` is this one's.

## Who an app is for

An app opens for whoever's Home Screen it's on, or whose App Library it's in. One about whoever opens it, as Vault,
Devices and Notifications are, says `subject: "user"`: it's in the user's own App Library and opens for them,
wherever its plugin is installed.

## Accounts

An account is a person's account at an outside service (a Gmail address, a WhatsApp number) that they sign a
plugin in to, so its tools act there as them. A plugin that does has accounts, and says so with three tools:

- `accounts` lists them, the default first: `%{accounts: [%{account: "sam@example.com", ok: true}]}`, and when `ok` is false,
  `why`, a few words saying what's wrong. It may add fields of its own.
- `sign_in` signs one in, or with `account` signs that one in again. It answers `%{url: ...}`, the web address
  the person signs in at. One whose signing in needs a screen of its own (a QR code, a device to pick) gives
  `sign_in` that page as its `ui`, named for what it is (`client/sign-in.html`), which is opened instead, with the
  account after a `#` when signing one in again.
- `sign_out` takes `account` and signs it out.

Its other tools take `account`, the first `accounts` lists when it's left out. It gives its own icon (`icon/0`)
and, with no app to take it from, its name (`title/0`): the plugin's, which Accounts and the App Store show.

Accounts, a folder at the end of your pages, lists them: a section for each plugin with accounts, by its name and
icon, its accounts with Sign In and Sign Out, and Add Account. Neither an account nor a plugin with accounts is an
app, so `pages` gives them apart from its apps, as `plugins_with_accounts`. A plugin with accounts has an app only
for a screen a person opens for something else.
