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
/// **Strict mode.** A strict page runs natively on a computer and on Android, and unchanged on the DOM everywhere
/// else (the web, an iPhone, MCP Apps). Every app's page is to be strict. A page is strict when its `tsconfig.json` is
/// `{ "extends": "<ui>/tsconfig.strict.json", "include": ["**/*"] }` (in this repo
/// `../../../core/ui/tsconfig.strict.json`) and its entrypoint says so, with the protocol version it's written for:
/// `%{page: "client/index.html", host: 2}`. It's written with the UI library alone: its components and the primitives,
/// as GPUI's are: `Box` (GPUI's `div`), `Scroll` (a box whose content scrolls), `Text`, `Image`, `Path` (a vector
/// shape) and `Input`, a text field. Each but `Input` is styled by its `class`, in Tailwind's names, which are GPUI's, by the design tokens
/// only: `<Box class="flex-row items-center gap-2 px-4 bg-muted rounded-full hover:bg-accent">`. Lengths are
/// Tailwind's spacing scale (`p-4` is 1rem) or a token (`px-gutter`, `gap-gap`, `h-control`, `size-icon`,
/// `max-w-readable`); colours are tokens (`bg-card`, `text-muted-foreground`, `border-input`, `bg-input/30`); text is
/// a type style (`text-body`, `text-label`, `text-headline`, `text-title`, `text-figure`, `text-subheadline`,
/// `text-footnote`, `text-caption-1`, `text-caption-2`) in `font-sans` or `font-mono`; radii, shadows and paces are
/// tokens (`rounded-lg`, `rounded-icon`, `shadow-md`, `duration-quick ease-settle`). The vocabulary is
/// `core/ui/src/strict/style.ts`: a class outside it is a type error, which names it. There's no element of the DOM's
/// (`<div>`, `<span>`, `<p>`), no stylesheet, and no `document`, `window` or other Web API; `render(() => <App />)`
/// from `ui` draws the page. Words are in a `Text`, or in a component that takes them (a `Row`'s `heading`). A press
/// is `onClick={() => ...}`, what's typed `onInput={(event) => setText(event.currentTarget.value)}`, a key
/// `onKeyDown={(event) => event.key === "Enter" && save()}`, a switch's `onChange={(event) =>
/// set(event.currentTarget.checked)}`. `bun run lint:strict apps/<app>/client`, from the repo's root, checks a page.
/// What a strict page may use is `core/ui/src/strict/types/ui.d.ts`; one that needs what isn't there adds it there,
/// with only what every host can draw (`protocol.ts`), and core/ui's `bun run typecheck` then says whether the DOM's
/// component takes it. A page written for version 1 has `style` objects instead of classes, which still draw.
export { call, changed, client, component, connection, covers, haptic, leave, link, open, pinShortcut, showRecoveryKey, sizeChanged, updateReady, upload, useRecoveryKey, visible } from "./bridge.ts"
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
