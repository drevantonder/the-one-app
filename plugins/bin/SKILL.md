---
name: bin
description: Read when someone wants back something they deleted, asks what they removed, or before a plugin deletes something a person made.
---

The bin keeps what's removed for 30 days, as it was, and then erases it for good. Anything an app made for someone
is named by where it is, like `users/alice/made/repos/notes`, and its owner removes and restores it.

- `list` lists what you own that was removed in the last 30 days, the latest first, with who removed it and when.
  For a group's things, ask for the group (`ONE_FOR=groups/<name>`).
- `restore --artifact <name>` brings one back as it was. If something new was made at the same name since, it says
  so: that has to be removed first.
- `remove --artifact <name>` removes one. An app's own way of removing, such as `repos remove`, does the same and
  keeps its own list true, so use the app's when it has one. Remove only when asked.
- `erase` is the clock's, each day: don't call it yourself.

A plugin removes what someone asked it to delete with `OneCore.Call.tool(call, "bin", "remove", %{"artifact" =>
name})`, never blob by blob, so it can be brought back. One that keeps a list of what it made hears `removed`,
which says when one was removed, restored or erased, and keeps its list true from it.
