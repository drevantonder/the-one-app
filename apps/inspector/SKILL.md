---
name: inspector
description: Read before checking or driving an app's window (one drawn natively, not in a web view) on one of the user's devices, to read its tree, tap, type, snap a picture or control its clock.
metadata:
  title: "Inspector"
  subtitle: "Your AI checks an app's screen"
  about: "Lets your AI read an app's screen on your devices as a test would, and tap, type and take pictures of it, to check it works."
  category: Developer tools
---

An app's window on a computer is drawn natively, and Inspector reads and drives it the way a test does: by
what's in it, not by pixels. It's faster and surer than Computer Use for these windows; use Computer Use for anything
else.

- `windows` lists the native windows open on your devices: each one's `device`, `window`, the `entrypoint` it shows
  (`notifications`), its size, and its clock. Every other tool takes `device` and `window` from there, and needs
  neither when only one is open.
- `tree` gives the semantic tree, each node's id, role, name, value, states, actions and `rect` (CSS pixels from the
  window's top left). `query` finds nodes by `role`, `name` (exact), `text` (words in the name), `key` or `node` (its
  id), with their rects and styles: start here rather than with a picture.
- `tap` presses a node by the same selector, or what's at `x` and `y`. Tapping a row's text presses the row.
- `type` types `text` into an input (by `role`, `name`, `key` or `node`), or the one with the keyboard. `key` presses
  `Enter`, `Escape`, `Tab`, `Backspace`, a letter.
- `settle` waits until the app has heard back from whatever it asked and its timers are done, and says what's still
  pending if its `deadline` passes first. Settle after an action before you read or snap.
- `snap` takes a PNG of the window. `clock` pauses, steps or seeks the app's clock, for what moves or waits on time.

A device is its person's: you inspect only theirs. Tapping and typing act as they would, so don't send, buy or delete
there unless they asked.
