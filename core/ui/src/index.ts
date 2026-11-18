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
export { call, changed, client, connection, covers, haptic, leave, open, pinShortcut, showRecoveryKey, sizeChanged, updateReady, upload, useRecoveryKey } from "./bridge.ts"
export { pace, seen, share, words } from "./shared.ts"
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
export { confirm, Sheet } from "./sheet.tsx"
export { SignIn } from "./sign-in.tsx"
export { toast } from "./toast.tsx"
export { Widget } from "./widget.tsx"
