// The UI library on a native host, which loads it as `/ui/client/ui.js` in place of the DOM's: the strict UI library
// (../types/ui.d.ts), drawn from the four primitives with the strict protocol's ops (renderer.ts). Its components are
// written in strict themselves. The UI library's typecheck proves each takes what a strict page gives it
// (../check.ts). A component the strict UI library has that isn't here yet throws, naming it.
import type * as Strict from "../types/ui.d.ts"
import { unsupported } from "./renderer.ts"

export { words } from "../../words.ts"
export { createListCollection } from "../../collection.ts"
export { negative, styles } from "../../styles.ts"
export { tokens } from "../../tokens.ts"

export { Box, EditableText, Image, Layer, Path, render, Scroll, Text } from "./primitives.tsx"
export { Checkbox, Field, Input, RadioGroup, Search, Switch, Textarea } from "../../field.tsx"
export { Select } from "../../select.tsx"
export { call, changed, client, component, connection, covers, direction, leave, link, go, keydown, open, pinShortcut, haptic, reducedMotion, safeArea, share, showRecoveryKey, sizeChanged, useRecoveryKey, viewport } from "./bridge.ts"

// The components, as the DOM's look and read: the same roles and names, from the design tokens.

export { Badge, Button, Icon, Spinner } from "../../button.tsx"
export { seen } from "../../seen.ts"

export { Screen, Section, Split } from "../../screen.tsx"
export { Empty, Problem, Skeleton, Status, Steps } from "../../states.tsx"

export { List, Row } from "../../list.tsx"
export { Metadata } from "../../metadata.tsx"

export { Menu } from "../../menu.tsx"
export { ActionBar, ActionPanel, Keys } from "../../actions.tsx"
export { Tabs } from "../../tabs.tsx"
export { Alert, confirm, Sheet } from "../../sheet.tsx"
export { toast } from "../../toast.tsx"

/** What a strict page may use that a native host can't draw yet: each tells the host, which opens the page another
 * way (in a web view), and throws, naming itself, where it's used. */
const later = (name: string) => () => {
  unsupported(name)
  throw new Error(`${name} isn't drawn on a native host yet (core/ui/src/strict/native/index.tsx)`)
}
export const LiveScreen = later("LiveScreen") as typeof Strict.LiveScreen
export const Widget = later("Widget") as typeof Strict.Widget
export const visible: typeof Strict.visible = () => true
