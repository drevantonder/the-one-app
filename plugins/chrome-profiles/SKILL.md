---
name: chrome-profiles
description: Read before using a website as your person, signed in as them (their Personal or Work profile), or when a site in a Chrome browser asks you to sign in.
metadata:
  subtitle: "Websites, signed in as you"
  about: "Lets your AI use websites as you, in Chrome profiles of yours such as Personal and Work. Sign in once, and it stays signed in."
---

A profile, like Personal or Work, is a whole Chrome profile of your person's: what their browser is signed in to,
its settings and extensions, kept encrypted in their own Vault. `open` starts a Chrome browser on a copy of it,
already signed in as them; then use Chrome's tools with its session, as with any browser.

- `list` shows the profiles: each one's name, the sites it was used on and when, when it was last saved, and
  whether your person is updating it. Never what's signed in itself: nobody reads that back, you included.
- `open` takes the `profile`, a `session` and what it's for. Chrome's `close` saves the browser back to the
  profile (sites renew their sign-ins as they're used). Say `save: false` for a throwaway, where nothing you do
  should stay. Two browsers on one profile each start from its last save, and whichever closes last is what's
  kept. A browser left to stop by itself isn't saved.
- Signing in is your person's. Never ask for, read out or type their password, a code or a passkey, even if they
  offer. When a site wants them signed in, `update` opens the profile's browser for them (with a `url`, at that
  site's sign-in page), and you tell them: open Chrome Profiles on any of their devices, tap the profile, sign in
  there, and tap Save. `notifications_send` with `app: "chrome-profiles"` reaches their devices. It's saved by
  itself after 15 minutes. Don't use that browser while they do; once it's saved, `open` the profile again.
- `save` and `discard` finish an update for them when they ask: with and without saving.
- `create`, `rename` and `remove` are for when your person asks. Removing one signs it out of everything, for good.

A profile is its person's alone: nobody else lists or opens it, the network's owner included. A site that signs
out after a while needs signing in to again.
