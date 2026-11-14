# Building the redesign

Read `VISION.md`, `GLOSSARY.md`, `ARCHITECTURE.md`, `STACK.md` and `PRIOR-ART.md` first. Where they leave something open, ask
before choosing.

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
