// The UI library on a native host, which loads it as `/ui/client/ui.js` in place of the DOM's: the strict UI library
// (../types/ui.d.ts), drawn from the four primitives with the strict protocol's ops (renderer.ts). Its components are
// written in strict themselves. The UI library's typecheck proves each takes what a strict page gives it
// (../check.ts). A component the strict UI library has that isn't here yet throws, naming it.
import type * as Strict from "../types/ui.d.ts"

export { words } from "../../words.ts"
export { createListCollection } from "../../collection.ts"
export { negative, styles } from "../../styles.ts"
export { tokens } from "../../tokens.ts"

export { Box, EditableText, Image, Layer, Path, render, Scroll, Text } from "./primitives.tsx"
export { Checkbox, Field, Input, RadioGroup, Search, Switch, Textarea } from "../../field.tsx"
export { Select } from "../../select.tsx"
export { call, changed, client, connection, covers, direction, go, keydown, open, pinShortcut, reducedMotion, safeArea, sizeChanged, viewport } from "./bridge.ts"

// The components, as the DOM's look and read: the same roles and names, from the design tokens.

export { Badge, Button, Icon, Spinner } from "../../button.tsx"
export { seen } from "../../seen.ts"

export { Screen, Section, Split } from "../../screen.tsx"
export { Empty, Problem, Skeleton, Status, Steps } from "../../states.tsx"

export { List, Row } from "../../list.tsx"

export { Menu } from "../../menu.tsx"
export { Alert, confirm, Sheet } from "../../sheet.tsx"

/** What a strict page may use that a native host can't draw yet: each throws, naming itself, where it's used. */
const later = (name: string) => () => {
  throw new Error(`${name} isn't drawn on a native host yet (core/ui/src/strict/native/index.tsx)`)
}
export const LiveScreen = later("LiveScreen") as typeof Strict.LiveScreen
export const Tabs = { Root: later("Tabs"), List: later("Tabs"), Trigger: later("Tabs"), Content: later("Tabs"), Indicator: later("Tabs") } as typeof Strict.Tabs
export const toast = later("toast") as typeof Strict.toast
export const leave = later("leave") as typeof Strict.leave
/** A native host doesn't open an app to a link yet. */
export const link: typeof Strict.link = () => undefined
export const component = later("component") as typeof Strict.component
export const showRecoveryKey = later("showRecoveryKey") as typeof Strict.showRecoveryKey
export const useRecoveryKey = later("useRecoveryKey") as typeof Strict.useRecoveryKey
export const share = later("share") as typeof Strict.share
export const haptic: typeof Strict.haptic = () => {}
export const Widget = later("Widget") as typeof Strict.Widget
export const visible: typeof Strict.visible = () => true
