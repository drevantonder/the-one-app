# The UI library

The reference for using it is the `ui` skill, built from the `///` comments (build.js). This is how it's built.

## Layers

Four, each a folder of `src/`, each depending only on the ones above it here:

1. `drawing-primitives/`: what the renderer draws, named and shaped as React Native's core components.
2. `behavior-primitives/`: behaviour and accessibility that draw nothing: focus, selection, typeahead, dismissing,
   anchoring, drag, announcements.
3. `headless-components/`: parts with their state, and no look.
4. `styled-components/`: our look, from the tokens, and each component's anatomy.

Never draw in a behavior, and never put behaviour in a drawing primitive. Until the behaviors and headless components
come from UI SDK, the styled components hold their own, and those two folders don't exist yet.

`src/` itself holds what every layer uses: the tokens (`tokens.ts`), styles (`styles.ts`), the host bridge
(`bridge.ts`, `calls.ts`) and words; and what's built from all of it: `index.ts`, `render.tsx` and `types/`, the UI
library as a page's type check sees it. A `.native` file is a native host's twin of the file it's named after, as React
Native's platform files are; build.js swaps them in for the native build.

## Adding

- A drawing primitive is renderer work, and rare: a host type in Rust (`clients/app/strict`) and in
  `drawing-primitives/renderer.ts`, with a `RUNTIME_VERSION` bump. First see whether it's a composition of the host's
  types, as `Pressable`, `Svg`, `Modal` and `Portal` are. It takes React Native's core component's name and props,
  else the standard community package's (react-native-svg, @react-native-picker/picker), and our own only where
  neither has one. Copy React Native's names and shapes, not its mistakes: where it has two ways, take the simpler,
  current one, and the web's where React Native has moved toward it; say which in the primitive's doc comment.
- A behavior or a headless component is copied from Vercel's UI SDK (Apache-2.0), with the upstream file and commit in
  its header, its DOM parts replaced by our host's (the drawing primitives, the bridge). Then its styled component,
  from the tokens, a scene (`gallery/*.scenes.tsx`), and `///` comments that start with an example, which are the
  `ui` skill.
- Updating from upstream: diff each copied file's upstream from the commit in its header to now, and apply that.
