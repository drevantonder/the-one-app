---
name: computer-use
description: Read before looking at or acting on a screen (a desktop, an app, a person's own Mac, Linux or Windows desktop, anything that isn't only a web page), by screenshot, click, type, scroll and keys, or when a Mac's screen says macOS doesn't allow Screen Recording or Accessibility.
metadata:
  title: "Computer Use"
  subtitle: "Your AI uses a screen"
  about: "Lets your AI see a screen and use it with clicks, typing and keys, for any app, not only web pages. You can watch what it does."
  category: Agent tools
---

Computer Use is how you see a screen and act on it like a person: take a screenshot, then click, type, scroll or
press keys at what you see. Its person watches in the Computer Use app.

- `screens` lists the screens you can use: each one's name, what it is, its size, and what it was last used for.
  A person's own computer is one (below), and other apps bring more: a container made from the image `desktop` is one
  (Containers' `create`). If you have none, make one first. Two of the same name, from two apps, are named `app/name`. An app that
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

## A person's own computer

Each Mac, Linux or Windows desktop running Computer Use's service is a screen, called what the computer is called: its main
display, the one with the menu bar, with everything on it, signed in as its person. You act as them, with their own
pointer and keyboard, so do only what they asked.

- Never type a password or a code (a one-time code, a passkey's PIN). Ask the person to type it themselves. On a Mac
  `type` refuses while a password field has the focus.
- A screenshot is the display scaled down to what a model sees unshrunk (at most 1568 pixels on its long side and
  about 1.15 megapixels). On a large display text is small: zoom in the app you're using (⌘+ in a browser) rather
  than guessing.
- A drag is a straight line: its `path` is two points, where it starts and where it ends.
- On a Mac, `ctrl` with a letter, digit or punctuation is ⌘, as a shortcut means there (`ctrl+l`, `ctrl+c`,
  `ctrl+t`), and with anything else ⌃ (`ctrl+Tab`). `super`, `cmd` and `meta` are ⌘, `alt` is ⌥, and `control` is
  always ⌃ (`control+c` in Terminal).
- It has no live video yet: `stream` refuses.

Only the computer's person, the user its server is for, and the network's owner may use it. A computer gets it with
`install_service --app computer-use --service ComputerUse.Desktop --server <key>`. The service runs Cua Driver
(trycua/cua) in its person's session: on a Mac a launchd agent, on Linux a systemd user unit that starts with their
desktop (an X11 or XWayland one), on Windows a scheduled task of theirs that starts as they log on. Until they're
signed in to it, `screens` says Cua Driver isn't running there. On Windows it runs as their limited user, as their
server does, so it can't use a window of a program run as administrator.

On a Mac it runs inside a small app, Computer Use for The One App, which macOS lets see the screen only with Screen
Recording and use the mouse and keyboard only with Accessibility, which only the Mac's person can allow. `screens`
says in the Mac's description which is missing, asking macOS nothing. The first time an action needs one, macOS asks
them on the Mac and the action says what's missing. They allow it in System Settings → Privacy & Security → Screen &
System Audio Recording, and → Accessibility, by turning on Computer Use for The One App in each. macOS keeps that
through the app's updates and Cua Driver's, and asks again now and then for Screen Recording, as it does of every
app that records the screen.

## Other apps' screens

An app with screens hooks into `computer-use.screens`: its hook's description (offers, in its code) says what it
answers and what its tool is called with.
