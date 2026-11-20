---
name: sign-in
description: Read before inviting someone to a network or taking someone out, when a device or an assistant says it isn't in yet or shows a code, or to list or remove someone's devices, assistants and passkeys.
metadata:
  title: "Sign-In"
  subtitle: "People, devices and passkeys"
  about: "Invite people to your network, who sign in with a passkey or Google and no password. See and remove your devices and the AI assistants you let in."
  category: "Privacy & Security"
---

Sign-in is how people get into a network, and what acts as each of them: their devices, and the assistants they
let in. A person signs in with a passkey at the network's address, or with Google where its edge has a Google client. An assistant becomes theirs when they answer
that one question for it, and a new device when they also say the code it shows matches.

- `invite` makes a link someone joins by: they open it and make a passkey, or continue with Google, and are that
  user from then on. The
  network's owners invite, and whoever they made an editor of `network/own/sign-in`. The link works once, for a
  week, and a new one for them ends it, so pass on the newest. Whoever joins by it is that user, so only the network's
  owner invites someone who's already in, as a way back when they've lost every passkey, and never an owner: an owner
  adds a passkey on a device they're signed in on.
- `domains` says which Google Workspace domains' people join by signing in with Google, with no invitation, and
  sets them for whoever invites. They're named `users/<their email's name>`, and go by their Google name.
- `devices` lists your devices, the assistants you let in and the browsers you're signed in on, and your
  passkeys. An app says whether it's `connected`, and if not when it was `last_seen`. `remove_device` takes one out: it's nobody's from the next thing it asks. `remove_passkey` removes a
  passkey, but never your last. `add_passkey` lets your person make another passkey: whoever opens its link makes
  one of theirs, so the link goes only to their own screen, and you're given none. They're notified, and Sign-In &
  Security makes it there.
- A passkey with WebAuthn's PRF unlocks its person's key on a new device too (the encryption app's skill says how).
  `devices` says which do (`unlocks`). Whoever uses one has the key, so a device that holds it encrypts it to a
  passkey only once its person allows that passkey there, by name and when it was added, in Sign In & Security:
  `encrypt_for_passkeys` with that `passkey`. Ask them first; never for a passkey they don't know they added. A
  passkey that hasn't said since it was made does once it's used: `unlock_with_passkey` with no device makes the
  link for that, and with a device that's asking for the key, lets that device in.
- `people` says who people are: the name each goes by, chosen as they joined, and their email where signing in
  has one. `name` changes the name your person goes by, when they ask. `everyone` lists everyone in the network
  and everyone invited who hasn't joined yet. `remove_person` takes someone out, or ends an invitation: their
  devices, assistants and passkeys stop working, they're in no group and shared nothing, and what they made stays. Whoever invites does it.
- A device that isn't in yet asks who's signing in. Once its person names themselves, it shows a number and waits:
  `waiting` lists the devices waiting to sign in as your person, for a few minutes, and never the number each
  shows: your person types it in Devices. Let one in only when your person tells you the number it shows:
  `approve` with its `request` and that `number`. A wrong number blocks it, and so does `deny`. A device that names
  your person could be a stranger's, and the number is what keeps it out. The app on a device that holds your person's key then
  hands it the key too, as `encryption_approve` does; with `one` on such a device, call `encryption_approve` with
  its `device` (as `waiting` lists it) and `name` afterwards.
- It can show a code and a link at the network's address instead. Its person opens the link where they sign in,
  on the device or any device of theirs, and says whether the code there matches. Pass both on as they are: the
  code is what keeps a stranger's device out.
- `approve` lets that device in yourself, as the person you act for, with no tap from them: when a device shows a
  code and they've asked for that device to be let in, call it with the code, and say which device it let in by
  the name it answers. Never let in a device nobody asked for, or one whose code came from anyone but your person
  or the device they asked about: a code someone sends you could be a stranger's. After five codes no device
  shows, it refuses for a quarter of an hour.
- Letting something in, and taking it out, are a person's decisions. Do them when asked to, and say what you
  did.
