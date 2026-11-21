// The UI library: every component an app's UI is written with, and the few things an app asks for in passing.
//
// A `///` comment that starts with an example is the UI library's skill: build.js gathers them, in the order this file
// names the files, as the skill an agent reads before it writes an app with the UI library.

/// An app is written in Solid 2.0 (its skills, `solid-2.0` and `solid-2.0-cheatsheet`, say how), and imports what
/// it draws with from the UI library: `import { Screen, List, Row, call } from "ui"`. The UI library looks the same in every
/// app, light and dark, under a finger and with a mouse, and right to left. A page shows pictures only from `data:`
/// addresses and its app's own files. An app may be TypeScript (`.tsx`, `.ts`) as well as JavaScript.
/// **A page.** An app's page runs on our renderer: natively on a computer, an iPhone and Android, and as the web's in
/// a browser and in MCP Apps. Its `tsconfig.json` is `{ "extends": "<ui>/tsconfig.app.json",
/// "include": ["**/*"] }` (in this repo `../../../core/ui/tsconfig.app.json`; a deploy checks it against its server's
/// own copy), and its entrypoint names it with the runtime version it needs, the one native hosts have now:
/// `%{page: "client/index.html", runtime_version: 5}`. It's written with the UI library alone: start with its styled
/// components (`styled-components/`), and where an app needs a look of its own with standard behaviour and
/// accessibility, build on its headless components and behaviours (`createListbox`, `createVelocityTracker`,
/// `projected`; others in `core/ui/src/headless-components/` and `behavior-primitives/` can be exported through
/// `types/ui.d.ts` and `index.ts` as an app needs them) over the drawing primitives,
/// named and shaped as React Native's core components and their props: `View`, `Pressable` (a view that's pressed),
/// `ScrollView` (a view whose content scrolls), `VirtualizedList` (a long list, of which only the rows near the window
/// are drawn, which `List each` draws), `Text`, `Image`, `TextInput` (bare text to type, which `Input` draws in
/// a field's box), `Modal` and `Portal` (what's drawn over the page), react-native-svg's `Svg` and `Path` (a vector
/// shape), `Video` (a screen's live video, which `LiveScreen` draws) and `Frame` (another app's page, in a sandbox of
/// its own, as an iframe is, which only a trusted app's page shows), each styled by its `style`: styles from `styles()`, beside what uses them, whose values are the design tokens (`tokens`, from
/// core/ui/src/tokens.ts), where a raw colour or length is a warning, "make it a token", and a raw font fails the check:
/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, paddingInline: tokens.length.gutter,
/// backgroundColor: { default: tokens.colour.card, hover: tokens.colour.muted }, font: tokens.type.label } })`, then
/// `<View style={[look.row, open() && look.open]} />`, the last to say a property winning. The build makes each
/// `styles()` ahead of time, so what it's given is written out: literals, `tokens.…`, `negative(…)`, an app's own tokens
/// (`defineVars`) and functions of values (dynamic styles). A component
/// lets a caller change only what it names: `style?: Styles<"marginTop" | "alignSelf">`. There's no element of the
/// DOM's (`<div>`, `<span>`, `<p>`), no `class`, no stylesheet, and no `document`, `window` or other DOM API. Its JS has
/// the language with `Intl`, and the Web APIs a Worker has (`URL`, `TextEncoder`, `crypto`, `structuredClone`,
/// `AbortController`, events, streams, timers) but `fetch`, as `call()` reaches the network, so an npm package that runs
/// in a Worker runs in it (core/ui/src/types/globals.d.ts says which). `render(() => <App />)` from `ui` draws
/// the page, and a page that throws while drawing says it can't show it, with Try Again, as `<Errored>` would. Words are in a `Text`, or in a component that takes them (a
/// `Row`'s `heading`). A press is `onPress={() => ...}`, a `Pressable`'s, a `Button`'s and a `Row`'s, as React Native names it; what's
/// typed `onChangeText={setText}`, Return in it `onSubmitEditing={save}`, a key `onKeyDown={(event) => event.key ===
/// "Escape" && close()}`, and a choice, a switch's, a select's, a radio group's or tabs', `onValueChange={setChosen}`, told
/// the value chosen. `bun run lint:app apps/<app>/client`, from the repo's
/// root, checks a page, as a deploy does of one whose entrypoint says its `runtime_version` and refuses it with the errors. What a page may use is `core/ui/src/types/ui.d.ts`: one that needs what isn't
/// there adds it there, with only what every host can draw (`drawing-primitives/renderer.ts`), and core/ui's `bun run typecheck` then says
/// whether the UI library's components take it.
export { call, changed, client, memoryWarning, component, connection, covers, direction, go, haptic, keydown, leave, link, open, pickFiles, pinShortcut, reducedMotion, safeArea, share, showRecoveryKey, sizeChanged, upload, useRecoveryKey, viewport, visible } from "./bridge.ts"
export { seen, since, size } from "./format.ts"
export { Badge, Button, Icon, Spinner } from "./styled-components/button.tsx"
export { Checkbox, Field, Input, RadioGroup, Search, Switch, Textarea } from "./styled-components/field.tsx"
export { createVelocityTracker, projected } from "./behavior-primitives/velocity.ts"
export { Select } from "./styled-components/select.tsx"
export { List, ListStyle, Row } from "./styled-components/list.tsx"
export { createListbox, type Listbox } from "./headless-components/listbox.ts"
export { Metadata } from "./styled-components/metadata.tsx"
export { History, Screen, Section, Shelf, Split } from "./styled-components/screen.tsx"
export { LiveScreen } from "./styled-components/live-screen.tsx"
export { Menu } from "./styled-components/menu.tsx"
export { ActionBar, ActionPanel, Keys, type Action } from "./styled-components/actions.tsx"
export { Tabs } from "./styled-components/tabs.tsx"
export { Empty, Problem, Skeleton, Status, Steps } from "./styled-components/states.tsx"
export { Alert, confirm, Sheet } from "./styled-components/sheet.tsx"
export { SignIn } from "./styled-components/sign-in.tsx"
export { toast } from "./styled-components/toast.tsx"
export { Widget } from "./styled-components/widget.tsx"
export { Frame, Image, Modal, Path, Portal, Pressable, ScrollView, Svg, Text, TextInput, Video, View, VirtualizedList } from "./drawing-primitives/primitives.tsx"
export { render } from "./render.tsx"
export { createTheme, defineVars, negative, styles } from "./styles.ts"
export { tokens } from "./tokens.ts"
