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
/// app's own files. Menu, Select, DatePicker, NumberInput and Tabs are Ark UI's, with Ark's parts and props
/// (ark-ui.com/llms-solid.txt); the rest are the UI library's own. An app may be TypeScript (`.tsx`, `.ts`) as well as
/// JavaScript: the UI library's types are its `types/` (`one code --name ui`), which the app's `tsconfig.json` names
/// as `"paths": { "ui": ["<there>/index.d.ts"] }`, and its build doesn't check them. A field's text is never set
/// under 16px on a touch screen, where an iPhone zooms into it: the UI library's fields and its base styles keep every
/// `<input>`, `<textarea>` and `<select>` at 16px or more there, and a field of another size sets `--text-field`.
/// **Strict mode.** A strict page runs natively, on our renderer on a computer and Android, and unchanged on the DOM
/// everywhere else (the web, an iPhone, MCP Apps). Every app's page is to be strict. A page is strict when its
/// `tsconfig.json` is `{ "extends": "<ui>/tsconfig.strict.json", "include": ["**/*"] }` (in this repo
/// `../../../core/ui/tsconfig.strict.json`) and its entrypoint says so, with the protocol version it's written for:
/// `%{page: "client/index.html", host: 2}`. It's written with the UI library alone: its components and the primitives,
/// `Box`, `Scroll` (a box whose content scrolls), `Text`, `Image`, `Path` (a vector shape) and `Input`, each styled by
/// its `style`: styles from `styles()`, beside what uses them, whose values are the design tokens (`tokens`, from
/// core/ui/src/tokens.ts), never a raw colour, size or font, which the typecheck refuses:
/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, paddingInline: tokens.length.gutter,
/// backgroundColor: { default: tokens.colour.card, hover: tokens.colour.muted }, font: tokens.type.label } })`, then
/// `<Box style={[look.row, open() && look.open]} />`, the last to say a property winning. The build makes each
/// `styles()` ahead of time, so what it's given is written out: literals, `tokens.…` and `negative(…)`. A component
/// lets a caller change only what it names: `style?: Styles<"marginTop" | "alignSelf">`. There's no element of the
/// DOM's (`<div>`, `<span>`, `<p>`), no `class`, no stylesheet, and no `document`, `window` or other Web API;
/// `render(() => <App />)` from `ui` draws the page. Words are in a `Text`, or in a component that takes them (a
/// `Row`'s `heading`). A press is `onClick={() => ...}`, what's typed `onInput={(event) =>
/// setText(event.currentTarget.value)}`, a key `onKeyDown={(event) => event.key === "Enter" && save()}`, a switch's
/// `onChange={(event) => set(event.currentTarget.checked)}`. `bun run lint:strict apps/<app>/client`, from the repo's
/// root, checks a page. What a strict page may use is `core/ui/src/strict/types/ui.d.ts`: one that needs what isn't
/// there adds it there, with only what every host can draw (`protocol.ts`), and core/ui's `bun run typecheck` then says
/// whether the DOM's takes it. A page written for version 1 has one `style` object of token names instead.
export { call, changed, client, component, connection, covers, haptic, keydown, leave, link, open, pinShortcut, reducedMotion, safeArea, showRecoveryKey, sizeChanged, updateReady, upload, useRecoveryKey, viewport, visible } from "./bridge.ts"
export { pace, seen, share } from "./shared.ts"
export { words } from "./words.ts"
export { Button, Icon, Spinner } from "./button.tsx"
export { Badge } from "./badge.tsx"
export { Checkbox, Field, File, fitted, Input, RadioGroup, Search, Switch, Textarea, useField } from "./field.tsx"
export { NumberInput } from "./number-input.tsx"
export { createListCollection } from "./collection.ts"
export { Select } from "./select.tsx"
export { DatePicker } from "./date-picker.tsx"
export { parseDate } from "./day.ts"
export { List, Row } from "./list.tsx"
export { go, Screen, Section, Split } from "./screen.tsx"
export { LiveScreen } from "./live-screen.tsx"
export { Menu } from "./menu.tsx"
export { Tabs } from "./tabs.tsx"
export { Empty, Problem, Skeleton, Status, Steps } from "./states.tsx"
export { Alert, confirm, Sheet } from "./sheet.tsx"
export { SignIn } from "./sign-in.tsx"
export { toast } from "./toast.tsx"
export { Widget } from "./widget.tsx"
export { Box, Image, Path, render, Scroll, Text } from "./strict/primitives.tsx"
export { negative, styles } from "./styles.ts"
export { tokens } from "./tokens.ts"
