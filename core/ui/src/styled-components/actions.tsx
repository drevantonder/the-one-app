// Actions, as Raycast has them on a computer: the floating pills at a window's foot, with what Return does and
// "Actions ⌘K", and the action panel ⌘K opens, a list of what can be done here that's searched as it's typed in.
// Written once on the drawing primitives, the same on the DOM and on a native host.
import { createEffect, createMemo, createSignal, For, onSettled, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Modal, Pressable, Text, TextInput, View } from "#primitives"
import { client, keydown } from "#bridge"
import { Icon, type IconName } from "./button.tsx"
import { messages } from "../words.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"
import type { Keyed } from "../drawing-primitives/primitives.tsx"

/** Something that can be done: what it says, what does it, the keys that do it too (`["mod", "enter"]`: ⌘↵ on a
 * Mac, Ctrl ↵ on a PC), an icon, whether it can't be undone (red), whether it can't be done now, and the section
 * it's in, a line apart from the one before. */
export interface Action {
  label: string
  onAction: () => void
  keys?: string[]
  icon?: IconName
  destructive?: boolean
  disabled?: boolean
  section?: string
  /** Whether it's an item of a row's menu, which the panel lists but Return never does: what a pointer rests on isn't
   * what's meant by a key, as an action of the screen is. */
  menu?: boolean
}

const look = styles({
  // The keys' chips: an outline, as Raycast draws them.
  keys: { flexDirection: "row", alignItems: "center", gap: tokens.grid[1], flexShrink: 0 },
  key: {
    minWidth: tokens.length.key,
    height: tokens.length.key,
    paddingInline: tokens.grid[1],
    alignItems: "center",
    justifyContent: "center",
    borderRadius: tokens.radius.small,
    boxShadow: tokens.shadow.key,
    font: tokens.type.small,
    color: tokens.colour.secondaryLabel,
  },
  // The pills, floating over the window's foot, 10px off its edges.
  bar: { position: "absolute", start: tokens.grid[2.5], end: tokens.grid[2.5], bottom: tokens.grid[2.5], zIndex: 4, flexDirection: "row", alignItems: "center", gap: tokens.grid[2], pointerEvents: "none" },
  pill: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[1],
    minWidth: 0,
    height: tokens.length.pill,
    padding: tokens.grid[1],
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.glass,
    backdropFilter: tokens.material.glass,
    boxShadow: tokens.shadow.pill,
    font: tokens.type.caption,
    color: tokens.colour.secondaryLabel,
    pointerEvents: "auto",
    userSelect: "none",
  },
  start: { flexShrink: 1, paddingInline: tokens.grid[1.5] },
  space: { flexGrow: 1 },
  part: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2],
    height: "100%",
    paddingInline: tokens.grid[2],
    borderRadius: tokens.radius.full,
    backgroundColor: { hover: tokens.colour.quinaryLabel, pressed: tokens.colour.quaternaryLabel },
    cursor: "pointer",
  },
  primary: { color: tokens.colour.label, lineClamp: 1 },
  // The panel, over the pills' end, and its rows.
  catcher: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, pointerEvents: "auto" },
  panel: {
    position: "absolute",
    end: tokens.grid[2.5],
    bottom: tokens.length.footer,
    width: tokens.length.actionPanel,
    maxWidth: "90%",
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.panel,
    backdropFilter: tokens.material.glass,
    boxShadow: tokens.shadow.window,
    color: tokens.colour.label,
    font: tokens.type.medium,
    overflow: "hidden",
    pointerEvents: "auto",
    opacity: { default: 1, entering: 0 },
    scale: { default: 1, entering: 0.97 },
    transitionProperty: ["opacity", "scale"],
    transition: tokens.motion.quick,
  },
  list: { padding: tokens.grid[1.5] },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2.5],
    height: tokens.grid[8],
    paddingInline: tokens.grid[2],
    borderRadius: tokens.radius.medium,
    cursor: "default",
  },
  active: { backgroundColor: tokens.colour.selection },
  off: { opacity: 0.5 },
  glyph: { width: tokens.grid[5], height: tokens.grid[5], flexShrink: 0, alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel, color: tokens.colour.secondaryLabel },
  label: { flexGrow: 1, flexShrink: 1, minWidth: 0, lineClamp: 1 },
  destructive: { color: tokens.colour.red },
  destructiveGlyph: { backgroundColor: tokens.colour.redTint, color: tokens.colour.red },
  separator: { height: tokens.length.hairline, marginBlock: tokens.grid[1], marginInline: tokens.grid[2], backgroundColor: tokens.colour.quinaryLabel },
  none: { paddingInline: tokens.grid[2], paddingBlock: tokens.grid[2], color: tokens.colour.tertiaryLabel },
  search: { height: tokens.grid[10], paddingInline: tokens.grid[3], justifyContent: "center", borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  field: { width: "100%", font: tokens.type.medium, color: tokens.colour.label },
})

/** The action bars drawn, the last the one that hears the keys, and the only one that's drawn: a window has one foot. */
const [bars, setBars] = createSignal<object[]>([])

/** Whether it's a Mac, whose keys are ⌘ where a PC's are Ctrl, as its client says. */
const [mac, setMac] = createSignal(false)
let asked = false
/** Asks the client whether it's a Mac, once, for `pressed`. */
export const askMac = () => {
  if (asked) return
  asked = true
  client().then((said) => setMac(!!said.mac), () => {})
}

/** How a key's shown: ⌘ or Ctrl, ⇧ and the rest as a Mac's menus show them, a letter in capitals; Return, which fonts
 * don't all have a glyph for, is an icon (`Keys`). */
const shown = (key: string): string =>
  ({ mod: mac() ? "⌘" : "Ctrl", shift: "⇧", alt: mac() ? "⌥" : "Alt", ctrl: mac() ? "⌃" : "Ctrl", escape: "esc", backspace: "⌫", delete: "⌦", tab: "⇥", up: "↑", down: "↓", left: "←", right: "→" })[key.toLowerCase()] ?? key.toUpperCase()

/** Whether a key pressed is these keys, `mod` being ⌘ on a Mac and Ctrl elsewhere (`askMac` first). */
export const pressed = (keys: string[], event: Keyed) => {
  const want = keys.map((key) => key.toLowerCase())
  const has = (name: string) => want.includes(name)
  const command = has("mod") ? (mac() ? event.metaKey : event.ctrlKey) : false
  const mods = (has("mod") ? command : !(mac() ? event.metaKey : event.ctrlKey)) && !!event.shiftKey === has("shift") && !!event.altKey === has("alt")
  const key = want.find((one) => !["mod", "shift", "alt", "ctrl"].includes(one))
  const named = { enter: "Enter", escape: "Escape", backspace: "Backspace", delete: "Delete", tab: "Tab", up: "ArrowUp", down: "ArrowDown", left: "ArrowLeft", right: "ArrowRight" } as Record<string, string>
  return mods && !!key && (named[key] ?? key) === (key.length === 1 ? event.key.toLowerCase() : event.key)
}

/// `<Keys keys={["mod", "K"]} />`: keys, as chips, as Raycast shows them beside what they do: `mod` is ⌘ on a Mac
/// and Ctrl on a PC, `enter` ↵ (an icon), `shift` ⇧, a letter in capitals.
export function Keys(props: { keys: string[] }) {
  onSettled(askMac)
  return (
    <View aria-hidden="true" style={look.keys}>
      <For each={props.keys}>
        {(key) => (
          <View style={look.key}>
            {key.toLowerCase() === "enter" ? <Icon name="arrow-elbow-down-left" /> : <Text>{shown(key)}</Text>}
          </View>
        )}
      </For>
    </View>
  )
}

/// `<ActionPanel open={acting()} onOpenChange={setActing} actions={actions()} />`: what can be done here, over a
/// window's foot at its end, as Raycast's action panel is: a list of `actions`, each with its keys, in sections a
/// line apart, red where it can't be undone, and a search at its foot, which has the keyboard and narrows the list
/// as it's typed in. The arrows move through it, Return does the one chosen, and Escape, ⌘K, a press outside it and
/// doing one close it. `ActionBar` draws one, with ⌘K to open it.
export function ActionPanel(props: { open: boolean; onOpenChange: (open: boolean) => void; actions: Action[] }) {
  const [typed, setTyped] = createSignal("")
  const [active, setActive] = createSignal(0)
  const found = createMemo(() => {
    const words = typed().trim().toLowerCase()
    return props.actions.filter((action) => !words || action.label.toLowerCase().includes(words))
  })
  const usable = () => found().filter((action) => !action.disabled)
  createEffect(() => props.open, (open) => void (open && (setTyped(""), setActive(0))))
  createEffect(typed, () => void setActive(0))
  const close = () => props.onOpenChange(false)
  const run = (action: Action | undefined) => {
    if (!action || action.disabled) return
    close()
    action.onAction()
  }
  const keys = (event: Keyed) => {
    const count = usable().length
    if (event.key === "ArrowDown" && count) (event.preventDefault(), setActive((active() + 1) % count))
    else if (event.key === "ArrowUp" && count) (event.preventDefault(), setActive((active() - 1 + count) % count))
    else if (event.key === "Enter") (event.preventDefault(), run(usable()[active()]))
    else if (event.key === "Escape" || pressed(["mod", "k"], event)) (event.preventDefault(), close())
  }
  return (
    <Show when={props.open}>
      <Modal>
        <Pressable style={look.catcher} onPress={close} />
        <View role="dialog" aria-label={messages.actions} onKeyDown={keys} style={look.panel}>
          <View role="listbox" style={look.list}>
            <For each={found()} fallback={<Text style={look.none}>{messages.noActions}</Text>}>
              {(action, at) => (
                <>
                  <Show when={at() > 0 && action.section !== found()[at() - 1].section}>
                    <View role="separator" style={look.separator} />
                  </Show>
                  <Pressable
                    role="option"
                    aria-selected={usable()[active()] === action ? "true" : "false"}
                    aria-disabled={action.disabled ? "true" : undefined}
                    onPointerEnter={() => !action.disabled && setActive(usable().indexOf(action))}
                    onPress={() => run(action)}
                    style={[look.row, usable()[active()] === action && look.active, action.destructive && look.destructive, action.disabled && look.off]}
                  >
                    <Show when={action.icon}>
                      {(icon) => (
                        <View style={[look.glyph, action.destructive && look.destructiveGlyph]}>
                          <Icon name={icon()} />
                        </View>
                      )}
                    </Show>
                    <Text style={look.label}>{action.label}</Text>
                    <Show when={action.keys?.length}>
                      <Keys keys={action.keys!} />
                    </Show>
                  </Pressable>
                </>
              )}
            </For>
          </View>
          <View style={look.search}>
            <TextInput
              aria-label={messages.searchActions}
              placeholder={messages.searchActions}
              value={typed()}
              autoFocus
              onChangeText={(text) => setTyped(text)}
              style={look.field}
            />
          </View>
        </View>
      </Modal>
    </Show>
  )
}

/// `<ActionBar start={<Text>Vault</Text>} actions={[{ label: "Save", keys: ["mod", "enter"], onAction: save }, …]} />`:
/// a window's foot on a computer, as Raycast's: floating pills over what's under it, `start` in one on the left (what's
/// shown, or where), and on the right the first of `actions` that can be done, with its keys (↵ unless it says), and
/// "Actions ⌘K", which opens the `ActionPanel` of them all. Each action's keys do it, ⌘K opens and closes the panel, and
/// what's under the pills keeps `tokens.length.footer` clear at its foot. With no actions there's no right pill.
export function ActionBar(props: { start?: JSX.Element; actions?: Action[] }) {
  const [open, setOpen] = createSignal(false)
  const actions = () => props.actions ?? []
  const primary = () => actions().find((action) => !action.disabled && !action.menu)
  // The first that can be done is Return's unless it says otherwise.
  const keysOf = (action: Action) => action.keys ?? (action.label === primary()?.label ? ["enter"] : [])
  onSettled(askMac)
  // Only the one drawn last hears the keys, and is drawn: in a list beside what it opens, what's open's.
  const me = {}
  onSettled(() => {
    setBars((now) => [...now, me])
    return () => setBars((now) => now.filter((one) => one !== me))
  })
  const last = () => bars().at(-1) === me
  onSettled(() =>
    keydown((event) => {
      if (!actions().length || !last()) return
      if (pressed(["mod", "k"], event)) return void (event.preventDefault(), setOpen(!open()))
      if (open()) return
      const action = actions().find((one) => !one.disabled && keysOf(one).length && pressed(keysOf(one), event))
      if (action) (event.preventDefault(), action.onAction())
    }),
  )
  return (
    <Show when={last()}>
      <View role="toolbar" style={look.bar}>
        <Show when={props.start}>
          <View style={[look.pill, look.start]}>{props.start}</View>
        </Show>
        <View style={look.space} />
        <Show when={actions().length}>
          <View style={look.pill}>
            <Show when={primary()}>
              {(action) => (
                <Pressable role="button" onPress={() => action().onAction()} style={look.part}>
                  <Text style={look.primary}>{action().label}</Text>
                  <Keys keys={keysOf(action())} />
                </Pressable>
              )}
            </Show>
            <Pressable role="button" aria-haspopup="dialog" aria-expanded={open() ? "true" : "false"} onPress={() => setOpen(!open())} style={look.part}>
              <Text>{messages.actions}</Text>
              <Keys keys={["mod", "K"]} />
            </Pressable>
          </View>
        </Show>
      </View>
      <ActionPanel open={open()} onOpenChange={setOpen} actions={actions()} />
    </Show>
  )
}
