---
name: sign-in
description: Read before inviting someone to a network, when a device or an assistant says it isn't in yet, or to list or remove someone's devices, assistants and passkeys.
---

Sign-in is how people get into a network, and what acts as each of them: their devices, and the assistants they
let in. A person signs in with a passkey at the network's address, and a device or an assistant becomes theirs
when they answer that one question for it.

- `invite` makes a link someone joins by: they open it and make a passkey, and are that user from then on. The
  network's owners invite, and whoever they made an editor of `network/own/sign-in`. The link works once, for a
  week, and is answered once, so pass it on then. Inviting someone who's already in gives them a way back when
  they've lost every passkey.
- `devices` lists your devices, the assistants you let in and the browsers you're signed in on, and your
  passkeys. `remove_device` takes one out: it's nobody's from the next thing it asks. `remove_passkey` removes a
  passkey, but never your last.
- A device that isn't in yet says where to let it in: a link at the network's address. Its person opens that on
  the device, or on any device of theirs, and signs in.
- Letting something in, and taking it out, are a person's decisions. Do them when asked to, and say what you
  did.
