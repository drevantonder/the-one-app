---
name: sandbox
description: Read when an untrusted app can't reach something, or before deciding whether to install an app trusted or untrusted.
metadata:
  subtitle: "Apps you don't know, kept apart"
  about: "Runs apps you don't fully trust walled off from your machines, with only what you let them have, so anyone can try one without a server of their own."
  category: Admin
---

An untrusted app runs behind this machine's own sandbox (bubblewrap on Linux, Seatbelt on a Mac, an AppContainer
on Windows): it reaches only what One Core hands it, and the tools and hosts its `permissions/0` names. When one
can't reach something, it's that its permissions don't name it; deploying it trusted (`--trusted`), which only someone
who may install for whoever it's for may do, runs it with its server's full access instead.
