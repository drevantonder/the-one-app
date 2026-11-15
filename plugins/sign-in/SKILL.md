---
name: sign-in
description: Read before inviting someone to a network or taking someone out, when a device or an assistant says it isn't in yet or shows a code, or to list or remove someone's devices, assistants and passkeys.
---

Sign-in is how people get into a network, and what acts as each of them: their devices, and the assistants they
let in. A person signs in with a passkey at the network's address. An assistant becomes theirs when they answer
that one question for it, and a new device when they also say the code it shows matches.

- `invite` makes a link someone joins by: they open it and make a passkey, and are that user from then on. The
  network's owners invite, and whoever they made an editor of `network/own/sign-in`. The link works once, for a
  week, and a new one for them ends it, so pass on the newest. Inviting someone who's already in gives them a way back when
  they've lost every passkey.
- `devices` lists your devices, the assistants you let in and the browsers you're signed in on, and your
  passkeys. An app's `seen` is `on` while it's connected, or when it last was. `remove_device` takes one out: it's nobody's from the next thing it asks. `remove_passkey` removes a
  passkey, but never your last. `add_passkey` makes a link your person makes another passkey by, in a browser, within a quarter of an hour:
  open it for them, and pass it to nobody.
- `people` says who people are: the name each goes by, chosen as they joined, and their email where signing in
  has one. `name` changes the name your person goes by, when they ask. `everyone` lists everyone in the network
  and everyone invited who hasn't joined yet. `remove_person` takes someone out, or ends an invitation: their
  devices, assistants and passkeys stop working, and what they made stays. Whoever invites does it.
- A device that isn't in yet shows a code, and a link at the network's address, and waits. Its person opens the
  link where they sign in, on the device or any device of theirs, and says whether the code there matches. Pass
  both on as they are: the code is what keeps a stranger's device out.
- `let_in` lets that device in yourself, as the person you act for, with no tap from them: when a device shows a
  code and they've asked for that device to be let in, call it with the code, and say which device it let in by
  the name it answers. Never let in a device nobody asked for, or one whose code came from anyone but your person
  or the device they asked about: a code someone sends you could be a stranger's. After five codes no device
  shows, it refuses for a quarter of an hour.
- Letting something in, and taking it out, are a person's decisions. Do them when asked to, and say what you
  did.
