---
name: computer-use
description: Read before looking at or acting on a screen (a desktop, an app, anything that isn't only a web page), by screenshot, click, type, scroll and keys.
metadata:
  title: "Computer Use"
  subtitle: "Your AI uses a screen"
  about: "Lets your AI see a screen and use it with clicks, typing and keys, for any app, not only web pages. You can watch what it does."
  category: Agent tools
---

Computer Use is how you see a screen and act on it like a person: take a screenshot, then click, type, scroll or
press keys at what you see. Its person watches in the Computer Use app.

- `screens` lists the screens you can use: each one's name, what it is, its size, and what it was last used for.
  Other apps bring the screens: a container made from the image `desktop` is one (Containers' `create`). If
  you have none, make one first. Two of the same name, from two apps, are named `app/name`. An app that
  couldn't list its screens (its machine is asleep, say) is in `failed`, with why, and the rest are still listed.
- `screenshot` shows the screen. x and y in every other tool are pixels of that picture, from its top left.
- `click` (button left, right or middle), `double_click`, `move`, `drag` (a path of points), `scroll` (up, down,
  left or right, by steps), `type` (text where the focus is), `key` (`ctrl+l`, `Return`, `ctrl+shift+t`; several,
  space between) act on it, and answer in a few words, not with a picture: take a `screenshot` to see what changed.
  `wait` waits up to 10 seconds, for a page to load, and gives a screenshot.
- Say what you're doing in `doing` ("Booking the 3pm train"): it's what your person sees you using it for.

Each step costs a screenshot, about a thousand tokens and a second or two, so it's slow and costly next to a tool
that reads. Use Computer Use for what isn't a web page, or a site that blocks automation. For a web page, the
Chrome app reads the page's text and clicks things by name: faster and cheaper. Press keys over hunting with the
pointer (`ctrl+l` then type a URL and `Return`, not clicking the address bar), and look again after anything that
loads.

A screen is its owner's: you use only your person's, and whatever is signed in on it is theirs. Don't sign in,
buy, send or delete there without them asking for it.

An app with screens hooks into `computer-use.screens`: its hook's description (offers, in its code) says what it
answers and what its tool is called with.
