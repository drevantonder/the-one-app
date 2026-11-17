---
name: sandbox
description: Read when an untrusted plugin can't reach something, or before deciding whether to install a plugin trusted or untrusted.
metadata:
  subtitle: "Plugins you don't know, kept apart"
  about: "Runs plugins you don't fully trust walled off from your machines, with only what you let them have, so anyone can try one without a server of their own."
  category: "Privacy & Security"
---

An untrusted plugin runs behind this machine's own sandbox (bubblewrap on Linux, Seatbelt on a Mac, an AppContainer
on Windows): it reaches only what One Core hands it, and the tools and hosts its `permissions/0` names. When one
can't reach something, it's that its permissions don't name it; deploying it trusted (`--trusted`), which only someone
who may install for whoever it's for may do, runs it with its server's full access instead.
