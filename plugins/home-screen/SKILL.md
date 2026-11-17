---
name: home-screen
description: Read before putting an app or a widget on a Home Screen page, arranging one, answering what's on someone's, or giving a plugin's app a category or the user it's about.
metadata:
  subtitle: "Your pages of apps"
  about: "Your Home Screen, everyone's and your groups': pages of apps and widgets, arranged your way, and an App Library of every app you have, by category."
  category: Utilities
---

Home Screen is the network's launcher: the app every client opens first, which draws the rest. It keeps how each
owner's pages are arranged: `pages` gives an owner's Home Screen, `arrange` changes its pages, and `add` and `remove`
put an app on them or take it off. A person edits their pages as on a phone: touch and hold, or right-click, then Edit
Home Screen. Plugins are installed with the Plugins app, whose `plugins_marketplace` lists what could be.

Its words:

- **Home Screen**: an owner's pages, then their App Library. Its owner is a user, a group, or the network, which a
  person reads as Everyone. A user sees theirs, everyone's and their groups', and moves between them with the owner
  switcher.
- **Page**: a fixed grid of apps and widgets, four across and six down, which never scrolls: what doesn't fit goes on
  the next. It holds only what was put there. A page can be hidden, kept but not shown, and a page with nothing on it
  goes. An owner's pages are one layout, so arranging everyone's or a group's arranges it for everyone who sees it.
  The user themselves arranges theirs, and a group's or the network's `owner`s theirs (`arranges`).
- **App Library**: every app that runs for an owner, by category, after their pages: what's installed for them and
  for each broader owner they're within. A user's shows theirs, their groups' and everyone's; a group's, its own and
  everyone's; everyone's, only everyone's. Each app is there once, as the narrowest install's, and a group's opens
  for the group. An app installed lands there, not on a page.
- One about whoever opens it (`subject: "user"`) is only in a user's App Library, wherever it's installed, and opens
  for them.
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
