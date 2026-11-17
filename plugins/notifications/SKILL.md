---
name: notifications
description: Read before telling a person something when they aren't looking, or when asked about their notifications, their inbox, or which apps are allowed to notify them.
metadata:
  subtitle: "Alerts on all your devices"
  about: "Notifications from your apps and your AI, on every device you use, kept in your inbox until you clear them. Choose which apps may notify you."
  category: Communication
---

Notifications reach a person on their devices and stay in their inbox until they clear them.

- `send` notifies whoever you're asking for, or `to` a user, a group you're in, or the network. `app` is the
  plugin whose app a tap opens; leave it out when nothing should open. It says who was notified, who silenced
  that app, and any phone that wasn't woken.
- A plugin notifies with `OneCore.Call.plugin(call, "notifications").send(call, %{"title" => ...})`, after
  `requires/0` names `notifications`. It's sent as that plugin's app.
- Notify when something needs the person, or they asked to hear of it: a claim to approve, a long job done,
  something they're waiting on. Don't notify about what they just did, what they're looking at, routine work
  that went fine, or the same thing twice. If in doubt, don't: every one costs their attention.
- The text is short. The title is the one thing that happened ("Sam approved your claim"), not the app's name,
  which shows beside it. `text` adds only what they need to act ("Lunch with Sam, $42"), or is left out. One idea
  a sentence, no "Notification:" or emoji, and never a secret: it passes through Apple's and Google's servers.
- `inbox` lists a person's notifications, newest first, with how many are unread. `read` and `clear` take an
  `id`, or do all of them without one.
- `apps` lists the apps that have notified a person and whether each is allowed, and `allow` mutes one or
  brings it back. A muted app's notifications aren't sent to them at all. These are the person's choices:
  change them when they ask.
- `register`, `unregister` and `devices` are for the app on a phone or in a browser, which registers where its
  push service reaches it, and its notification key: what it's sent is encrypted to that key, so Apple, Google
  and the network's edge carry it without reading it, and the phone opens it before it shows. A computer's app shows notifications while it runs, and needs nothing registered.
