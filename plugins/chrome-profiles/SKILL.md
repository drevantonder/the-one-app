---
name: chrome-profiles
description: Read before using a website as your person, signed in as them (their Personal or Work profile), or when a site in a Chrome browser asks you to sign in.
---

A profile, like Personal or Work, is what a Chrome browser is signed in to: its cookies and local storage, kept
encrypted in your person's own Vault. `open` starts a Chrome browser on one, already signed in as them; then use
Chrome's tools with its session, as with any browser.

- `list` shows the profiles: each one's name, the sites it was used on and when, and when it was last saved.
  Never what's signed in itself: nobody reads that back, you included.
- `open` takes the `profile`, a `session` and what it's for. Chrome's `close` saves what changed (new sign-ins,
  renewed cookies) back to the profile. Say `save: false` for a throwaway, where nothing you do should stay. Two
  browsers on one profile each start from its last save, and whichever closes last is what's kept. A browser
  left to stop by itself isn't saved.
- Signing in is your person's, once per site. Never ask for, read out or type their password, a code or a
  passkey, even if they offer. When a site wants them signed in, `sign_in` with the profile and the site's URL
  opens it for them, and you tell them: open Chrome Profiles on any of their devices and tap the profile;
  `notifications_send` with `app: "chrome-profiles"` reaches their devices. They sign in there themselves, and it's
  saved when they tap Done, or after 15 minutes. Then `open` the profile again.
- `create`, `rename` and `remove` are for when your person asks. Removing one signs it out of everything, for good.

A profile is its person's alone: nobody else lists or opens it, the network's owner included. Saved is only what a
site keeps in the browser; a site that signs out after a while needs signing in to again.
