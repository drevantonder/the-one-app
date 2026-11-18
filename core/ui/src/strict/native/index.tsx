// The UI library on a native host, which loads it as `/ui/client/ui.js` in place of the DOM's: the strict UI library
// (../types/ui.d.ts), drawn from the four primitives with the strict protocol's ops (renderer.ts). Its components are
// written in strict themselves. The UI library's typecheck proves each takes what a strict page gives it
// (../check.ts). A component the strict UI library has that isn't here yet throws, naming it.
import { createContext, createSignal, For, Show, useContext } from "solid-js"
import type { Element } from "solid-js"
import { createElement, event, render as draw, spread } from "./renderer.ts"
import { messages, readable, whole } from "../../words.ts"
import type { Props } from "../protocol.ts"
import type * as Strict from "../types/ui.d.ts"
import caretLeft from "@phosphor-icons/core/assets/regular/caret-left.svg?raw"
import caretRight from "@phosphor-icons/core/assets/regular/caret-right.svg?raw"
import dotsThree from "@phosphor-icons/core/assets/regular/dots-three.svg?raw"
import warningCircle from "@phosphor-icons/core/assets/fill/warning-circle-fill.svg?raw"

export { words } from "../../words.ts"
export { createListCollection } from "../../collection.ts"

/** A primitive: a node of its type, with its props. */
const primitive = (type: "box" | "text" | "image" | "input") => (props: object) => {
  const node = createElement(type)
  spread(node, props)
  return node as unknown as Element
}

export const Box: typeof Strict.Box = primitive("box")
export const Text: typeof Strict.Text = primitive("text")
export const Image: typeof Strict.Image = primitive("image")
export const Input: typeof Strict.Input = primitive("input")
export const Textarea: typeof Strict.Textarea = (props) => primitive("input")({ ...props, multiline: true })
export const Search: typeof Strict.Search = (props) => primitive("input")({ ...props, type: "search" })

export const render: typeof Strict.render = (code) => draw(code as never)

// The bridge, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other, through the host.

const host = (globalThis as { host?: { post(message: object): void } }).host
const waiting = new Map<number, (response: { result?: unknown; error?: { message: string } }) => void>()
const listeners = new Set<() => void>()
let requests = 0

;(globalThis as { ui?: object }).ui = {
  event,
  message(message: { id?: number; method?: string; result?: unknown; error?: { message: string } }) {
    if (message.method === "notifications/one/changed") for (const listener of listeners) listener()
    else if (message.method === "ui/notifications/host-context-changed") dress((message as { params?: object }).params)
    else if (message.id !== undefined) waiting.get(message.id)?.(message)
  },
}

function ask<T>(method: string, params: object): Promise<T> {
  const id = ++requests
  return new Promise((done, failed) => {
    waiting.set(id, ({ result, error }) => {
      waiting.delete(id)
      if (error) failed(new Error(error.message))
      else done(result as T)
    })
    host?.post({ jsonrpc: "2.0", id, method, params })
  })
}

/** What the client says of where the page is shown, as the DOM's bridge reads it: whether the window names the app
 * above it (a computer's app window, whose breadcrumb a first screen's title gives way to) and whether its network
 * answers. */
const [breadcrumb, setBreadcrumb] = createSignal(false)
const [connected, setConnected] = createSignal<ReturnType<typeof Strict.connection>>("online")
function dress(context?: { breadcrumb?: boolean; connection?: ReturnType<typeof Strict.connection> }) {
  if (context && "breadcrumb" in context) setBreadcrumb(context.breadcrumb === true)
  if (context?.connection) setConnected(context.connection)
}

/** A client answers a page once it has said what it is, which it does as it loads. */
const greeted = ask<{ hostContext?: object }>("ui/initialize", { protocolVersion: "2026-01-26", appInfo: { name: "", version: "1" }, appCapabilities: {} }).then(({ hostContext }) => {
  dress(hostContext)
  host?.post({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} })
})

export const call: typeof Strict.call = async (tool, inputs = {}, { subject } = {}) => {
  await greeted
  const { content, isError } = await ask<{ content: { text?: string }[]; isError?: boolean }>("tools/call", { name: tool, arguments: inputs, ...(subject ? { _meta: { subject } } : {}) })
  const text = content.map(({ text }) => text ?? "").join("\n")
  if (isError) throw new Error(readable(text), { cause: text })
  return text
}

export const client = (async (change: Strict.ClientChange = {}) => {
  await greeted
  return ask("one/client", change)
}) as typeof Strict.client

export const changed: typeof Strict.changed = (listener) => {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}

// The components, as the DOM's look and read: the same roles and names, from the design tokens.

const svg = (raw: string) => `data:image/svg+xml,${encodeURIComponent(raw)}`
const icons: Partial<Record<Strict.IconName, string>> = { "caret-left": svg(caretLeft), "caret-right": svg(caretRight), "dots-three": svg(dotsThree), "warning-circle": svg(warningCircle) }

export const Icon: typeof Strict.Icon = (props) => {
  const src = icons[props.name]
  if (!src) throw new Error(`No icon named "${props.name}" on a native host yet. There are: ${Object.keys(icons).join(", ")}`)
  return <Image src={src} alt="" aria-hidden="true" style={{ width: "icon", height: "icon" }} />
}

export const Spinner: typeof Strict.Spinner = () => <Box role="status" aria-label={messages.loading} style={{ width: "icon", height: "icon", borderWidth: 1, borderRadius: "full", borderColor: "muted-foreground" }} />

const looks: Record<NonNullable<Strict.ButtonProps["variant"]>, Props["style"]> = {
  default: { backgroundColor: "primary", color: "primary-foreground" },
  secondary: { backgroundColor: "secondary", color: "secondary-foreground" },
  outline: { borderWidth: 1, borderColor: "input", backgroundColor: "background" },
  ghost: {},
  destructive: { backgroundColor: "destructive", color: "primary-foreground" },
}

export const Button: typeof Strict.Button = (props) => (
  <Box
    role="button"
    aria-label={props["aria-label"]}
    aria-disabled={props.disabled || props.loading ? "true" : undefined}
    aria-busy={props.loading ? "true" : undefined}
    onClick={() => !props.disabled && !props.loading && props.onClick?.()}
    style={{ flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8, height: "control", minWidth: "control", paddingInline: props.size === "icon" ? 0 : 16, borderRadius: "md", ...looks[props.variant ?? "default"] }}
  >
    <Show when={props.loading}>
      <Spinner />
    </Show>
    <Text style={{ font: "label" }}>{props.children}</Text>
  </Box>
)

export const Badge: typeof Strict.Badge = (props) => (
  <Box aria-label={props["aria-label"]} style={{ paddingInline: 8, borderRadius: "full", ...(props.variant === "destructive" ? { backgroundColor: "destructive", color: "primary-foreground" } : props.variant === "outline" ? { borderWidth: 1 } : { backgroundColor: props.variant === "secondary" ? "secondary" : "primary", color: props.variant === "secondary" ? "secondary-foreground" : "primary-foreground" }) }}>
    <Text style={{ font: "caption-1", fontVariantNumeric: "tabular-nums" }}>{props.children}</Text>
  </Box>
)

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
    <Box role="main" style={{ flexGrow: 1, overflow: "scroll", gap: "gap", paddingInline: "gutter", paddingBottom: "gutter" }}>
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

export const List: typeof Strict.List = (props) => (
  <Box role="list" style={{ maxWidth: "readable", borderWidth: 1, borderRadius: "xl", backgroundColor: "card", color: "card-foreground", overflow: "hidden" }}>
    {props.children}
  </Box>
)

export const Row: typeof Strict.Row = (props) => {
  const inside = () => (
    <>
      <Show when={props.start}>{props.start}</Show>
      <Box style={{ flexGrow: 1, flexShrink: 1 }}>
        <Text style={{ font: "label", lineClamp: 1 }}>{props.heading}</Text>
        <Show when={props.text}>
          <Text style={{ color: "muted-foreground", lineClamp: 2 }}>{props.text}</Text>
        </Show>
      </Box>
      <Show when={props.end}>{props.end}</Show>
    </>
  )
  const row = { flexDirection: "row", alignItems: "center", gap: 12, flexGrow: 1, paddingBlock: 10 } as const
  return (
    <Box role="listitem" style={{ flexDirection: "row", alignItems: "center", gap: 12, paddingInline: 16, minHeight: "control", borderBottomWidth: 1, backgroundColor: props.current ? "accent" : undefined }}>
      <Show when={props.control}>{props.control}</Show>
      <Show when={props.onClick} fallback={<Box style={row}>{inside()}</Box>}>
        <Box role="button" aria-current={props.current ? "true" : undefined} onClick={() => props.onClick?.()} style={row}>
          {inside()}
        </Box>
      </Show>
      <Show when={props.action}>{props.action}</Show>
    </Box>
  )
}

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

/** What a menu's parts share: whether it's open, and what picking an item does. */
const Opened = createContext<{ open: () => boolean; setOpen: (open: boolean) => void; select: (value: string) => void }>()

export const Menu: typeof Strict.Menu = {
  Root: (props) => {
    const [open, setOpen] = createSignal(false)
    return <Opened value={{ open, setOpen, select: (value) => (setOpen(false), props.onSelect?.({ value })) }}>{props.children}</Opened>
  },
  Trigger: (props) => {
    const menu = useContext(Opened)!
    return (
      <Box role="button" aria-label={props["aria-label"] ?? (props.children ? undefined : messages.more)} aria-expanded={menu.open() ? "true" : "false"} onClick={() => menu.setOpen(!menu.open())} style={{ height: "control", minWidth: "control", alignItems: "center", justifyContent: "center", borderRadius: "md" }}>
        {props.children ?? <Icon name="dots-three" />}
      </Box>
    )
  },
  Positioner: (props) => <>{props.children}</>,
  Content: (props) => {
    const menu = useContext(Opened)!
    return (
      <Show when={menu.open()}>
        <Box role="menu" style={{ paddingBlock: 4, borderWidth: 1, borderRadius: "md", backgroundColor: "popover", color: "popover-foreground" }}>
          {props.children}
        </Box>
      </Show>
    )
  },
  Item: (props) => {
    const menu = useContext(Opened)!
    return (
      <Box role="menuitem" aria-disabled={props.disabled ? "true" : undefined} onClick={() => !props.disabled && menu.select(props.value)} style={{ paddingInline: 8, paddingBlock: 6, borderRadius: "sm", color: props.destructive ? "destructive" : undefined }}>
        <Text>{props.children}</Text>
      </Box>
    )
  },
  Separator: () => <Box role="separator" style={{ borderBottomWidth: 1, marginBlock: 4 }} />,
}

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
export const Sheet = later("Sheet") as typeof Strict.Sheet
export const Alert = later("Alert") as typeof Strict.Alert
export const Tabs = { Root: later("Tabs"), List: later("Tabs"), Trigger: later("Tabs"), Content: later("Tabs"), Indicator: later("Tabs") } as typeof Strict.Tabs
export const toast = later("toast") as typeof Strict.toast
export const confirm = later("confirm") as typeof Strict.confirm
export const open = later("open") as typeof Strict.open
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
export const connection: typeof Strict.connection = () => connected()
export const visible: typeof Strict.visible = () => true
