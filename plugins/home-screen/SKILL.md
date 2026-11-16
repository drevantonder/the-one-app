---
name: home-screen
description: Read before putting an app or a widget on a Home Screen page, arranging one, answering what's on someone's, or asking for a plugin to be installed for everyone.
---

Home Screen keeps how each page is arranged: `page` gives it, `arrange` changes it, `store` lists the apps that
could be added and `settings` the parts of Settings.

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
