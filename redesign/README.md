# Redesign

The One App designed again from the ground up, starting with what an artifact is. Decided here, not built: the rest
of this repo is the app as it runs today, and its GLOSSARY.md and ARCHITECTURE.md describe that.

- `VISION.md`: the vision, unchanged.
- `GLOSSARY.md`: the redesign's words.
- `ARCHITECTURE.md`: the redesign's rules.
- `AGENTS.md`: how to build it.
- `PRIOR-ART.md`: what exists already, what was set aside and why, and decisions that are easy to miss. Read it
  before designing anything.

Not designed yet: how a client's offline writes reach a server, what signs users in, and what a plugin's code and
its apps are written in.

These docs name no technology on purpose, to avoid overfitting or getting distracted. What exists is in
`PRIOR-ART.md`, and none of it is chosen.

Claims is the pressing need, but a bad platform would mean another rebuild, so the platform comes first.

Later, and not critical: bringing data across from the app as it runs today and from the old PTC app.

Parked, to keep in mind: the public as a who, anyone at all and not signed in, as a `viewer` of what an owner makes
public. Filling in something like a form, and visitors without an account, come after that.
