---
name: plugins
description: Read before showing a person which plugins they could install, or answering what's in the network's marketplace.
metadata:
  subtitle: "Find, install and manage plugins"
  about: "Every plugin your network can install, by what it's for, and what's installed for you, everyone and your groups: install, update, uninstall and bring back."
  category: Utilities
---

Plugins is the app a person installs and removes plugins with, as VS Code's Extensions view is: Marketplace, every
plugin the network can install, and Installed, what's installed for each owner they see. `marketplace` lists the
first: every plugin anyone in the network deployed, then the plugin directory's (one.drevan.me/plugins) that aren't
among them, each with what it requires and why. Installing and the rest are One Core's own tools (`install`,
`uninstall`, `plugins`, `plugins_update`, `plugins_retry`, `plugins_trust`, `plugins_restore`), which an agent calls
itself.

A plugin only the directory has is a stranger's code, not yet in the network: the skill installing-plugins says how
to check it for the person and install it (`one install <source> --plugin <name>`), which the app leaves to their AI.
