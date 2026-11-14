# The One App

The One App designed again from the ground up, starting with what an artifact is. The app as it ran before is at the
tag `before-redesign` and on the branch `v1`.

- `VISION.md`: the vision, unchanged.
- `GLOSSARY.md`: the redesign's words.
- `ARCHITECTURE.md`: the redesign's rules.
- `AGENTS.md`: how to build it.
- `STACK.md`: what it's built with, what isn't chosen yet, and what to test first.
- `PRIOR-ART.md`: what exists already, what was set aside and why, and decisions that are easy to miss. Read it
  before designing anything.

Not designed yet: how a client's offline writes reach a server, and what signs users in.

The glossary and the rules name no technology on purpose, to avoid overfitting or getting distracted. What's chosen is
in `STACK.md`, and what else exists is in `PRIOR-ART.md`.

Claims is the pressing need, but a bad platform would mean another rebuild, so the platform comes first.

Later, and not critical: bringing data across from the app as it runs today and from the old PTC app.

Parked, to keep in mind: the public as a who, anyone at all and not signed in, as a `viewer` of what an owner makes
public. Filling in something like a form, and visitors without an account, come after that.

Left open until the rest lands: what starts a network's cloud server when no other server answers for a user or group.
It's a big choice, and fewer other things should be moving first.
