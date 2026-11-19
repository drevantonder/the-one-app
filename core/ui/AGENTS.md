# The UI library

The reference for using it is the `ui` skill, built from the `///` comments (build.js). This is how it's built.

## Layers

Four, each a folder of `src/`, each depending only on the ones above it here:

1. `drawing-primitives/`: what the renderer draws, named and shaped as React Native's core components.
2. `behavior-primitives/`: behaviour and accessibility that draw nothing: focus, selection, typeahead, dismissing,
   anchoring, drag, announcements.
3. `headless-components/`: parts with their state, and no look.
4. `styled-components/`: our look, from the tokens, and each component's anatomy.

Never draw in a behavior, and never put behaviour in a drawing primitive. A styled component holds no behaviour of
its own: it spreads its headless component's parts (`<Pressable {...tabs.trigger(value)} style={…}>`) and adds its
look. A part's props are getters, kept by `merge` and JSX's spread, never by an object spread, which reads them once.

What the host already does, a behavior doesn't do again: a press, by a pointer, Enter or Space (`Pressable`'s
`onPress`); hover, pressed and focus-visible (a style's `hover`, `pressed` and `focused`); a modal's inert outside
and the keyboard held in it (`Modal`, `aria-modal`). The keyboard is moved by a node's `autoFocus`, which takes it each
time it's set, and a node says it has it by `onFocus`: the host's in place of the DOM's `focus()`.

`src/` itself holds what every layer uses: the tokens (`tokens.ts`), styles (`styles.ts`), the host bridge
(`bridge.ts`, `calls.ts`) and words; and what's built from all of it: `index.ts`, `render.tsx` and `types/`, the UI
library as a page's type check sees it. A `.native` file is a native host's twin of the file it's named after, as React
Native's platform files are; build.js swaps them in for the native build.

## Adding

- A drawing primitive is renderer work, and rare: a host type in Rust (`clients/app/renderer`) and in
  `drawing-primitives/renderer.ts`, with a `RUNTIME_VERSION` bump. First see whether it's a composition of the host's
  types, as `Pressable`, `Svg`, `Modal` and `Portal` are. It takes React Native's core component's name and props,
  else the standard community package's (react-native-svg, @react-native-picker/picker), and our own only where
  neither has one. Copy React Native's names and shapes, not its mistakes: where it has two ways, take the simpler,
  current one, and the web's where React Native has moved toward it; say which in the primitive's doc comment.
- A behavior or a headless component is copied from Vercel's UI SDK (Apache-2.0, `src/UI-SDK-LICENSE.txt`), with the upstream file and commit in
  its header, its DOM parts replaced by our host's (the drawing primitives, the bridge). Copy only what a component
  uses; pure logic (which item a key moves to, where a popup goes, how fast a finger was going) as it is. Then its styled component,
  from the tokens, a scene (`gallery/*.scenes.tsx`), and `///` comments that start with an example, which are the
  `ui` skill.
- Updating from upstream: diff each copied file's upstream from the commit in its header to now, and apply that.

## Motion

Motion is the renderer's, on its own thread, never a turn of a page's JS a frame (`clients/app/renderer/src/view/transition.rs`,
`motion.rs`). A style says it, by the ecosystem's names:

- `transitionProperty` and `transition` (a motion token, eased or a spring): a change moves, and a spring turned
  midway keeps its speed, as SwiftUI's and Motion's do.
- `entering` (CSS's `@starting-style`) and `exiting` (Motion's `exit`): what's added starts from one, and what's taken
  out is drawn moving to the other before it's gone. Nothing keeps a node mounted in JS to animate it out.
- `drag`, `dragConstraints`, `dragElastic` and `onDragEnd` on a View (Motion's): it follows the finger where the input
  arrives, and let go, springs from there at the finger's speed to wherever its style then puts it. A gesture that
  settles somewhere (a sheet closing, a page turned) sets that style in `onDragEnd`; it doesn't move the node itself.

The DOM build draws what CSS has (`transition`, `@starting-style`) and leaves out the rest.
