// The UI library on a native host, which loads it as `/ui/client/ui.js` in place of the DOM's: the strict UI library
// (../types/ui.d.ts), drawn from the four primitives with the strict protocol's ops (renderer.ts). Its components are
// written in strict themselves. The UI library's typecheck proves each takes what a strict page gives it
// (../check.ts). A component the strict UI library has that isn't here yet throws, naming it.
import { createContext, createSignal, For, Show, useContext } from "solid-js"
import type { Element } from "solid-js"
import { messages, readable, whole } from "../../words.ts"
import type { Props, Type } from "../protocol.ts"
import type * as Strict from "../types/ui.d.ts"

export { words } from "../../words.ts"
export { createListCollection } from "../../collection.ts"
export { negative, styles } from "../../styles.ts"
export { tokens } from "../../tokens.ts"

export { Box, Image, Input, Layer, Path, render, Scroll, Search, Text, Textarea } from "./primitives.tsx"
import { Box, Image, Text } from "./primitives.tsx"
import { breadcrumb } from "./bridge.ts"
export { call, changed, client, connection, covers, direction, open, pinShortcut, reducedMotion, safeArea, sizeChanged, viewport } from "./bridge.ts"

// The components, as the DOM's look and read: the same roles and names, from the design tokens.

export { Badge, Button, Icon, Spinner } from "../../button.tsx"
import { Button, Icon, Spinner } from "../../button.tsx"

export const Screen: typeof Strict.Screen = (props) => (
  <Box style={{ flexGrow: 1, backgroundColor: "background", color: "foreground" }}>
    <Box role="banner" style={{ flexDirection: "row", alignItems: "center", gap: 8, paddingInline: "gutter", paddingBlock: 10 }}>
      <Show when={props.back}>
        <Button variant="ghost" size="icon" aria-label={messages.back} onClick={() => props.onBack?.()}>
          <Icon name="caret-left" />
        </Button>
      </Show>
      <Show when={props.back || !breadcrumb()} fallback={<Box style={{ flexGrow: 1 }} />}>
        <Text role="heading" aria-level={1} style={{ font: "title", flexGrow: 1 }}>
          {props.heading}
        </Text>
      </Show>
      {props.action}
    </Box>
    <Box role="main" style={{ flexGrow: 1, overflow: "scroll", gap: "gutter", paddingInline: "gutter", paddingBottom: "gutter" }}>
      {props.children}
    </Box>
  </Box>
)

export const Section: typeof Strict.Section = (props) => (
  <Box style={{ gap: 8 }}>
    <Show when={props.heading}>
      <Text role="heading" aria-level={2} style={{ font: "footnote", color: "muted-foreground" }}>
        {props.heading}
      </Text>
    </Show>
    {props.children}
  </Box>
)

export { List, Row } from "../../list.tsx"

const state = { alignItems: "center", justifyContent: "center", gap: 12, paddingBlock: 48, textAlign: "center" } as const

export const Empty: typeof Strict.Empty = (props) => (
  <Box style={state}>
    <Text style={{ color: "muted-foreground" }}>{props.children}</Text>
    {props.action}
  </Box>
)

export const Skeleton: typeof Strict.Skeleton = (props) => (
  <Box role="status" aria-busy="true" aria-label={messages.loading} style={{ gap: 12, maxWidth: "readable" }}>
    <For each={Array.from({ length: props.rows ?? 3 })}>{() => <Box style={{ height: "control", borderRadius: "md", backgroundColor: "muted" }} />}</For>
  </Box>
)

export const Problem: typeof Strict.Problem = (props) => {
  const said = () => (typeof props.text === "string" || props.text instanceof Error ? readable(whole(props.text)) : props.text)
  return (
    <Box role="alert" style={state}>
      <Icon name="warning-circle" />
      <Text style={{ font: "label", color: props.warn ? "warning" : "destructive" }}>{props.heading}</Text>
      <Show when={props.text}>
        <Text style={{ color: "muted-foreground" }}>{said() as Element}</Text>
      </Show>
      {props.action}
    </Box>
  )
}

export const Status: typeof Strict.Status = (props) => (
  <Box role="status" style={state}>
    <Spinner />
    <Text style={{ font: "label" }}>{props.heading}</Text>
    <Show when={props.text}>
      <Text style={{ color: "muted-foreground" }}>{props.text}</Text>
    </Show>
  </Box>
)

export { Menu } from "../../menu.tsx"
export { Alert, confirm, Sheet } from "../../sheet.tsx"

/** What a strict page may use that a native host can't draw yet: each throws, naming itself, where it's used. */
const later = (name: string) => () => {
  throw new Error(`${name} isn't drawn on a native host yet (core/ui/src/strict/native/index.tsx)`)
}
export const Split = later("Split") as typeof Strict.Split
export const Steps = later("Steps") as typeof Strict.Steps
export const Field = later("Field") as typeof Strict.Field
export const Checkbox = later("Checkbox") as typeof Strict.Checkbox
export const Switch = later("Switch") as typeof Strict.Switch
export const LiveScreen = later("LiveScreen") as typeof Strict.LiveScreen
export const RadioGroup = later("RadioGroup") as typeof Strict.RadioGroup
export const Select = { Root: later("Select") } as typeof Strict.Select
export const Tabs = { Root: later("Tabs"), List: later("Tabs"), Trigger: later("Tabs"), Content: later("Tabs"), Indicator: later("Tabs") } as typeof Strict.Tabs
export const toast = later("toast") as typeof Strict.toast
export const leave = later("leave") as typeof Strict.leave
/** A native host doesn't open an app to a link yet. */
export const link: typeof Strict.link = () => undefined
export const component = later("component") as typeof Strict.component
export const showRecoveryKey = later("showRecoveryKey") as typeof Strict.showRecoveryKey
export const useRecoveryKey = later("useRecoveryKey") as typeof Strict.useRecoveryKey
export const go: typeof Strict.go = (change) => change()
export const share = later("share") as typeof Strict.share
export const seen = later("seen") as typeof Strict.seen
export const haptic: typeof Strict.haptic = () => {}
export const Widget = later("Widget") as typeof Strict.Widget
export const keydown = later("keydown") as typeof Strict.keydown
export const visible: typeof Strict.visible = () => true
