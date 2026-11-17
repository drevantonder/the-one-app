---
name: qa
description: Use before you commit a change, to check it works and doesn't break what it could touch. There are no kept tests: you check your own change.
---

Check your work with a throwaway check and never commit it. Use the cheapest thing that shows it works: call the
tool on a local network (`local-network`), open the screen (`see-it-running`), or run the function in `iex -S mix`
in `core/server/`.

Work out what your change touches, or could touch, including things that look unrelated, and check those too. If
that includes secrets, who may do what, a plugin's app reaching another's, or data kept and restored, try to break
it yourself on a local network. You decide the attacks; there is no fixed list.

A failing check is chased until you know why, and reported. Never skip it, loosen it, or commit "a small fix" on a
guess you haven't reproduced.

## Mistakes already made

- Reading the codebase from scratch. Start from the files your task names; `rg` for the rest.
- Checking again after rebasing onto other people's unrelated commits.
- A check that read a log before the process had created it, and failed for that. Follow a log with `tail -F`, as
  `tooling/network` does, never by reading it once.
- Merging a fix whose check was failing, on the guess that the check was wrong.
