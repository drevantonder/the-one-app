# Building The One App

Read `VISION.md`, `GLOSSARY.md`, `ARCHITECTURE.md`, `STACK.md` and `PRIOR-ART.md` first. Where they leave something open, ask
before choosing.

- Consider yourself the owner of this codebase, and treat it as its maintainer would.
- Dumb beats clever. Delete before adding.
- Choose the simplest implementation that fully meets the requirements. Prefer well-maintained libraries over custom
  code.
- One name per thing. A rename reaches everything in the same change, with no aliases or fallbacks for old names, and
  no internal words where a user reads.
- A special case is a missing design: an `if` for one value or plugin, a parameter for one caller, a swallowed error,
  a retry or a sleep, a skipped test. Fix the design so it goes; keep one only when the outside world needs it, with
  a comment saying what in the world.
- Tests are few, and only for what using it wouldn't show: secrets and who may do what, data kept and restored,
  releases, and where two languages must agree.
- People use apps. The CLI and MCP are for their agents, even for the most technical person on a network, so never
  shape a person's flow around a terminal.
- How it looks is the repo's `DESIGN.md`: our look, the same on every client.

## Docs

- Document only what the code can't say.
- Nothing describes the code but the code: no hand-written references that can drift; generate them or point at the
  source.
- As the code replaces a doc, the doc goes. Deleting a rule or a line of `STACK.md` is part of the change that builds
  it.

## Work

- Push to `main` as you go, pulling with `--rebase` first: several threads on several machines work on it at once.
  Work in your own git worktree, never in a checkout others share, and from its branch push
  `git push -u origin HEAD:main`.
- One commit per change, once it works, as a Conventional Commit: `type(area): what changes, for whoever uses it`. No
  co-author or "Generated with" lines.
- Never print a secret. Check a file that holds them by its names or hashes, never its values.
- Don't write wait loops (sleep, until, polling a log or a remote). Run the command in the foreground, or start it in
  the background and let your harness tell you when it ends.
- Stop what you start once you're done with it, since every thread shares these machines, and remove your worktree.
- The machines here also run the app as it was before, whose command is `one`. Never install over it or stop it.
