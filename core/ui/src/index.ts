// The UI library: every component an app's UI is written with, and the few things an app asks for in passing.
//
// A `///` comment that starts with an example is the UI library's skill: build.js gathers them, in the order this file
// names the files, as the skill an agent reads before it writes an app with the UI library.

/// An app is written in Solid 2.0 (its skills, `solid-2.0` and `solid-2.0-cheatsheet`, say how), and imports what
/// it draws with from the UI library: `import { Screen, List, Row, call } from "ui"`. The UI library looks the same in every
/// app, light and dark, under a finger and with a mouse, and right to left on `<html dir="rtl">`; an app lays out
/// what's its own with its own CSS, from the UI library's custom properties (`--foreground`, `--muted-foreground`,
/// `--border`, `--warning`, `--radius`, `--gap`), with any hover of its own behind `@media (hover: hover) and
/// (pointer: fine)`, as the UI library's is: a touch screen leaves what was tapped looking hovered. A page shows pictures only from `data:` addresses and its
/// app's own files. Tabs take Ark UI's tabs' parts and props, and Select Ark's select's root props
/// (ark-ui.com/llms-solid.txt); the rest are the UI library's own. An app may be TypeScript (`.tsx`, `.ts`) as well as
/// JavaScript: the UI library's types are its `types/` (`one code --name ui`), which the app's `tsconfig.json` names
/// as `"paths": { "ui": ["<there>/index.d.ts"] }`, and its build doesn't check them. A field's text is never set
/// under 16px on a touch screen, where an iPhone zooms into it: the UI library's fields and its base styles keep every
/// `<input>`, `<textarea>` and `<select>` at 16px or more there, and a field of another size sets `--text-field`.
/// **A page.** An app's page runs natively, on our renderer on a computer and Android, and unchanged on the DOM
/// everywhere else (the web, an iPhone, MCP Apps), where its `tsconfig.json` is `{ "extends":
/// "<ui>/tsconfig.strict.json", "include": ["**/*"] }` (in this repo `../../../core/ui/tsconfig.strict.json`) and its
/// entrypoint says so, with the runtime version it needs: `%{page: "client/index.html", runtime_version: 4}`; every
/// app's page is to be one. It's written with the UI library alone: its components and the drawing primitives,
/// `Box`, `Scroll` (a box whose content scrolls), `Text`, `Image`, `Path` (a vector shape), `EditableText` (bare text to
/// type, which `Input` draws in a field's box), `Video` (a screen's live video, which `LiveScreen` draws) and `Frame`
/// (another app's page, in a sandbox of its own, as an iframe is, which only a trusted app's page shows), each styled by
/// its `style`: styles from `styles()`, beside what uses them, whose values are the design tokens (`tokens`, from
/// core/ui/src/tokens.ts), where a raw colour or length is a warning, "make it a token", and a raw font fails the check:
/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, paddingInline: tokens.length.gutter,
/// backgroundColor: { default: tokens.colour.card, hover: tokens.colour.muted }, font: tokens.type.label } })`, then
/// `<Box style={[look.row, open() && look.open]} />`, the last to say a property winning. The build makes each
/// `styles()` ahead of time, so what it's given is written out: literals, `tokens.…`, `negative(…)`, an app's own tokens
/// (`defineVars`) and functions of values (dynamic styles). A component
/// lets a caller change only what it names: `style?: Styles<"marginTop" | "alignSelf">`. There's no element of the
/// DOM's (`<div>`, `<span>`, `<p>`), no `class`, no stylesheet, and no `document`, `window` or other DOM API. Its JS has
/// the language with `Intl`, and the Web APIs a Worker has (`URL`, `TextEncoder`, `crypto`, `structuredClone`,
/// `AbortController`, events, streams, timers) but `fetch`, as `call()` reaches the network, so an npm package that runs
/// in a Worker runs in it (core/ui/src/types/globals.d.ts says which). `render(() => <App />)` from `ui` draws
/// the page, and a page that throws while drawing shows what failed and Try Again, as `<Errored>` would. Words are in a `Text`, or in a component that takes them (a
/// `Row`'s `heading`). A press is `onClick={() => ...}`, what's typed `onInput={(event) =>
/// setText(event.currentTarget.value)}`, Return in it `onAction={save}`, a key `onKeyDown={(event) => event.key ===
/// "Escape" && close()}`, a switch's `onChange={(event) => set(event.currentTarget.checked)}`. `bun run lint:strict apps/<app>/client`, from the repo's
/// root, checks a page, as a deploy does of one whose entrypoint says its `runtime_version` and refuses it with the errors. What a page may use is `core/ui/src/types/ui.d.ts`: one that needs what isn't
/// there adds it there, with only what every host can draw (`drawing-primitives/renderer.ts`), and core/ui's `bun run typecheck` then says
/// whether the DOM's takes it. A page written for version 1 has one `style` object of token names instead.
export { call, changed, client, component, connection, covers, direction, go, haptic, keydown, leave, link, open, pinShortcut, reducedMotion, safeArea, showRecoveryKey, sizeChanged, updateReady, upload, useRecoveryKey, viewport, visible } from "./bridge.ts"
export { pace, share } from "./shared.ts"
export { seen } from "./seen.ts"
export { words } from "./words.ts"
export { Button, Icon, Spinner } from "./styled-components/button.tsx"
export { Badge } from "./styled-components/badge.tsx"
export { Checkbox, Field, Input, RadioGroup, Search, Switch, Textarea, useField } from "./styled-components/field.tsx"
export { createListCollection } from "./collection.ts"
export { Select } from "./styled-components/select.tsx"
export { createListbox, List, ListStyle, Row, type Listbox } from "./styled-components/list.tsx"
export { Metadata } from "./styled-components/metadata.tsx"
export { History, Screen, Section, Split } from "./styled-components/screen.tsx"
export { LiveScreen } from "./styled-components/live-screen.tsx"
export { Menu } from "./styled-components/menu.tsx"
export { ActionBar, ActionPanel, Keys, type Action } from "./styled-components/actions.tsx"
export { Tabs } from "./styled-components/tabs.tsx"
export { Empty, Problem, Skeleton, Status, Steps } from "./styled-components/states.tsx"
export { Alert, confirm, Sheet } from "./styled-components/sheet.tsx"
export { SignIn } from "./styled-components/sign-in.tsx"
export { toast } from "./styled-components/toast.tsx"
export { Widget } from "./styled-components/widget.tsx"
export { Box, EditableText, Frame, Image, Layer, Path, Scroll, Text, Video } from "./drawing-primitives/primitives.tsx"
export { render } from "./render.tsx"
export { createTheme, defineVars, negative, styles } from "./styles.ts"
export { tokens } from "./tokens.ts"
