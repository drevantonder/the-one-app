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
/// plugin's own files. Menu, Select, DatePicker, NumberInput and Tabs are Ark UI's, with Ark's parts and props
/// (ark-ui.com/llms-solid.txt); the rest are the UI library's own.
export { call, changed, client, leave, open, showRecoveryKey, upload, useRecoveryKey } from "./bridge.ts"
export { seen, share, words } from "./shared.ts"
export { Button, Icon, Spinner } from "./button.tsx"
export { Checkbox, Field, File, fitted, Input, RadioGroup, Search, Switch, Textarea, useField } from "./field.tsx"
export { NumberInput } from "./number-input.jsx"
export { createListCollection } from "./collection.ts"
export { Select } from "./select.jsx"
export { DatePicker } from "./date-picker.jsx"
export { parseDate } from "./day.ts"
export { List, Row } from "./list.tsx"
export { go, Screen, Section, Split } from "./screen.tsx"
export { Menu } from "./menu.jsx"
export { Tabs } from "./tabs.jsx"
export { Empty, Problem, Skeleton } from "./states.tsx"
export { confirm, Sheet } from "./sheet.tsx"
export { SignIn } from "./sign-in.tsx"
export { toast } from "./toast.tsx"
export { Widget } from "./widget.tsx"
