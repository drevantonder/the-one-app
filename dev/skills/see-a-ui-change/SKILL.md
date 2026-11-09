---
name: see-a-ui-change
description: Seeing a change to screens, @one/ui, the web client or the auth pages in The One App, running.
---

# See a UI change

Did you change Rust? Then build first, and use `target/debug/one-dev throwaway` as AGENTS.md says.

Only UI files changed? No Rust build:

1. `bun run build` in `src/screen/ui` if you changed what it builds.
2. From your checkout: `one-dev throwaway --ui .` runs the installed build with your checkout's UI.
3. `one-dev throwaway restart` after each change to the Workers or web client. A node reads screens fresh each time.
4. `one-dev throwaway client web` gives you a browser and a picture. Look at the picture.
5. `one-dev throwaway stop` when you're done.

An app's own screens: `one-dev throwaway deploy apps/<name>`.
