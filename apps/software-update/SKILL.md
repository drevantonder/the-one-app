---
name: software-update
description: Read when the network's edge is behind its servers' build, when Software Update says the edge went back, or before setting up a network's edge to update itself.
metadata:
  title: "Software Update"
  subtitle: "Keeps your network's edge current"
  about: "Updates your network's web address, sign-in pages and web app to the build your servers run, by itself, after every update."
  category: Admin
---

The network's update takes its servers to each build, and then has one of them deploy that build's edge, the part
of the network at its address (`core/edge/updater/updater.ts`). This app is what deploys it there: with it installed
for the network, a server says it deploys the edge (`deploys_edge`, in `one builds`' servers), and the update
tells one ready server on the build to, as `updating_edge`. The deploy checks the edge answers as that build and
rolls every Worker back if not; one that went back says why under `went_back`, and the update tries again as it
does for a server.

It deploys with the network's Cloudflare API token: the field `api_token` of the item `Cloudflare edge deploy` in the
network's Vault, switched on for this app, a token that can edit Workers and read the zone of the network's address.
Only a server that opens the network's Vault reads it, and it never leaves that server. To set it up, keep the token
there (`vault set`, as a plain tool on a device, `ONE_SUBJECT=network`, `secret: true`), and ask its person to switch
it on for this app with `vault request` (`app: "software-update"`), whose link they open and Allow.
