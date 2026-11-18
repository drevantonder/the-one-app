// A menu: a button that opens a few things to do, over the page under it. Written once in strict, on the primitives,
// the same on the DOM and on a native host.
import { children, createContext, createEffect, createSignal, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Layer, Text } from "#primitives"
import { direction, viewport } from "#bridge"
import { Button, Icon, type ButtonProps } from "./button.tsx"
import { messages } from "./words.ts"
import { styles } from "./styles.ts"
import { tokens } from "./tokens.ts"
import type { Laid, Located } from "./strict/types/ui.d.ts"

/** Where something's drawn, as a menu opens under it. */
const rectOf = (event: Laid): At => {
  const { x, y, width, height } = event.currentTarget.getBoundingClientRect()
  return { x, y, width, height }
}

/** Where a menu opens: under what opened it, or where a row was held. */
type At = { x: number; y: number; width: number; height: number }

/** What a menu in a row gives the row (list.tsx): what opens it where the row was held, its `destructive` item, which
 * a swipe shows, and what chooses an item as pressing it does. */
export interface RowMenu {
  open: (at: At) => void
  destructive: () => { value: string; label: JSX.Element } | undefined
  select: (value: string) => void
}

/// How a menu in a row's `end` tells the row it's there, and that it's gone. A menu anywhere else, as in a screen's
/// bar, finds none.
export const RowMenu = createContext<((menu: RowMenu | undefined) => void) | null>(null)

/** An item, as the menu's keys move through them: its value, what it says, and whether it can be chosen. */
type Entry = { value: string; label: string; disabled: boolean }

/** What a menu's parts share. */
const Opened = createContext<{
  at: () => At | undefined
  open: (at: At | undefined) => void
  select: (value: string) => void
  /** Its items, in order, and the one the keyboard is on. */
  items: () => Entry[]
  add: (entry: Entry) => () => void
  active: () => string | undefined
  setActive: (value: string | undefined) => void
  /** Whether it was open and has closed, so its button takes the keyboard back. */
  closed: () => boolean
  /** Where its button is, where it opens when something else opens it. */
  place: (at: At) => void
  destructive: () => { value: string; label: JSX.Element } | undefined
  setDestructive: (item: { value: string; label: JSX.Element } | undefined) => void
}>()

const look = styles({
  catcher: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, pointerEvents: "auto" },
  anchor: { position: "absolute" },
  closed: { display: "none" },
  below: { position: "absolute", top: "100%", end: 0, marginTop: tokens.grid[1], pointerEvents: "auto" },
  above: { position: "absolute", bottom: "100%", end: 0, marginBottom: tokens.grid[1], pointerEvents: "auto" },
  content: {
    minWidth: tokens.length.menu,
    maxWidth: tokens.length.alert,
    padding: tokens.grid[1],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.background,
    color: tokens.colour.label,
    boxShadow: tokens.shadow.window,
    font: tokens.type.medium,
    opacity: { default: 1, entering: 0 },
    scale: { default: 1, entering: 0.97 },
    transitionProperty: ["opacity", "scale"],
    transition: tokens.motion.quick,
  },
  item: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2],
    minHeight: tokens.grid[8],
    paddingInline: tokens.grid[2],
    paddingEnd: tokens.grid[4],
    borderRadius: tokens.radius.small,
    backgroundColor: { hover: tokens.colour.quinaryLabel, focused: tokens.colour.quinaryLabel },
    cursor: "default",
    userSelect: "none",
  },
  destructive: { color: tokens.colour.red, backgroundColor: { hover: tokens.colour.redTint, focused: tokens.colour.redTint } },
  disabled: { opacity: 0.5 },
  separator: { height: tokens.length.hairline, marginBlock: tokens.grid[1], backgroundColor: tokens.colour.quinaryLabel },
  label: { paddingInline: tokens.grid[2], paddingBlock: tokens.grid[1], color: tokens.colour.secondaryLabel, font: tokens.type.small, fontWeight: tokens.weight.medium },
})

/// `<Menu.Root onSelect={({ value }) => value === "delete" && remove()}>`, then `<Menu.Trigger />`, then
/// `<Menu.Positioner><Menu.Content>` around `<Menu.Item value="rename">Rename</Menu.Item>`s, a `<Menu.Separator />`
/// between groups, `<Menu.Group label="Install for">` around some under a small heading, and `<Menu.Item
/// value="delete" destructive>Delete</Menu.Item>`: a button that opens a few things to do, under it, over the page. The
/// trigger is three dots, named `aria-label` (More unless said); with words in it, it's a button of `variant` (a
/// `Button`'s, `outline` unless said) that says them, and `size="icon"` keeps one with an icon in it to its icon. An
/// item starts with an `<Icon />` if it's given one, and `destructive` is for what can't be undone. A press outside,
/// Escape and choosing an item close it. In a `Row`'s `end`, a long press or a right-click on the row opens it too,
/// and a finger swipes the row to its `destructive` item.
export const Menu = {
  Root: (props: { open?: boolean; onOpenChange?: (details: { open: boolean }) => void; onSelect?: (details: { value: string }) => void; children?: JSX.Element }) => {
    const [where, setWhere] = createSignal<At>()
    const [button, setButton] = createSignal<At>()
    const [own, setOwn] = createSignal(false)
    const [destructive, setDestructive] = createSignal<{ value: string; label: JSX.Element }>()
    const [items, setItems] = createSignal<Entry[]>([])
    const [active, setActive] = createSignal<string>()
    const [closed, setClosed] = createSignal(false)
    // Open as its `open` says, where that's said (as Ark's is), or as it was pressed; where it was opened, or under
    // its button.
    const shown = () => props.open ?? own()
    const at = () => (shown() ? (where() ?? button()) : undefined)
    // Opened, the keyboard is on its first item; closed, back on its button.
    createEffect(shown, (open) => {
      setActive(open ? items().find((item) => !item.disabled)?.value : undefined)
      if (!open) setWhere(undefined)
    })
    const open = (to: At | undefined) => {
      setClosed(shown() && !to)
      setWhere(to)
      setOwn(!!to)
      props.onOpenChange?.({ open: !!to })
    }
    const select = (value: string) => (open(undefined), props.onSelect?.({ value }))
    const add = (entry: Entry) => {
      setItems((now) => [...now, entry])
      return () => setItems((now) => now.filter((item) => item !== entry))
    }
    return <Opened value={{ at, open, select, destructive, setDestructive, items, add, active, setActive, closed, place: setButton }}>{props.children}</Opened>
  },
  Trigger: (props: { variant?: ButtonProps["variant"]; size?: "icon"; "aria-label"?: string; children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    // In a row, the row opens it too.
    const row = useContext(RowMenu)
    if (row) {
      row({ open: menu.open, destructive: menu.destructive, select: menu.select })
      onCleanup(() => row(undefined))
    }
    const inside = children(() => props.children)
    const words = () => inside.toArray().length > 0 && props.size !== "icon"
    return (
      <Button
        variant={props.variant ?? (words() ? "outline" : "ghost")}
        size={words() ? undefined : "icon"}
        aria-label={props["aria-label"] ?? (words() ? undefined : messages.more)}
        aria-haspopup="menu"
        aria-expanded={menu.at() ? "true" : "false"}
        autofocus={menu.closed()}
        {...({ onLayout: (event: Laid) => menu.place(rectOf(event)) } as {})}
        onClick={(event: Located) => {
          const rect = event?.currentTarget.getBoundingClientRect()
          menu.open(menu.at() || !rect ? undefined : { x: rect.x, y: rect.y, width: rect.width, height: rect.height })
        }}
      >
        {inside.toArray().length ? inside() : <Icon name="dots-three" />}
      </Button>
    )
  },
  /** Where a menu's content shows: in a layer over the page, under what opened it, its end at that's end, or above
   * it where that's in the window's lower half. Closed, it's kept, unseen, so a row knows its destructive item. */
  Positioner: (props: { children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    const above = () => menu.at()!.y + menu.at()!.height / 2 > viewport().height / 2
    const inside = children(() => props.children)
    return (
      <Show when={menu.at()} fallback={<Box style={look.closed}>{inside()}</Box>}>
        {(at) => (
          <Layer modal>
            <Box style={look.catcher} onClick={() => menu.open(undefined)} />
            <Box style={[look.anchor, { top: at().y, start: direction() === "rtl" ? viewport().width - at().x - at().width : at().x, width: at().width, height: at().height }]}>
              <Box style={above() ? look.above : look.below} onKeyDown={(event) => event.key === "Escape" && menu.open(undefined)}>
                {inside()}
              </Box>
            </Box>
          </Layer>
        )}
      </Show>
    )
  },
  /** The items, which the keyboard moves through as WAI-ARIA's menu pattern has it: the arrows to the next and the
   * one before, round from the last to the first, Home and End to the first and the last, a letter to the next that
   * starts with it, and Tab out, closing it. */
  Content: (props: { children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    const keys = (event: { key: string; preventDefault(): void }) => {
      const usable = menu.items().filter((item) => !item.disabled)
      if (!usable.length) return
      const at = usable.findIndex((item) => item.value === menu.active())
      const to = (index: number) => (event.preventDefault(), menu.setActive(usable[(index + usable.length) % usable.length].value))
      if (event.key === "ArrowDown") to(at + 1)
      else if (event.key === "ArrowUp") to(at < 0 ? -1 : at - 1)
      else if (event.key === "Home") to(0)
      else if (event.key === "End") to(-1)
      else if (event.key === "Tab") menu.open(undefined)
      else if (event.key.length === 1 && event.key !== " ") {
        const letter = event.key.toLowerCase()
        const after = [...usable.slice(at + 1), ...usable.slice(0, at + 1)]
        const found = after.find((item) => item.label.toLowerCase().startsWith(letter))
        if (found) (event.preventDefault(), menu.setActive(found.value))
      }
    }
    return (
      <Box role="menu" tabindex={-1} onKeyDown={keys} style={look.content}>
        {props.children}
      </Box>
    )
  },
  Item: (props: { value: string; destructive?: boolean; disabled?: boolean; children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    if (props.destructive) {
      menu.setDestructive({ value: props.value, label: props.children })
      onCleanup(() => menu.setDestructive(undefined))
    }
    const inside = children(() => props.children)
    onCleanup(menu.add({ value: props.value, label: typeof props.children === "string" ? props.children : "", disabled: !!props.disabled }))
    return (
      <Box
        role="menuitem"
        tabindex={-1}
        autofocus={menu.active() === props.value}
        onPointerEnter={() => !props.disabled && menu.setActive(props.value)}
        aria-disabled={props.disabled ? "true" : undefined}
        onClick={() => !props.disabled && menu.select(props.value)}
        onKeyDown={(event) => (event.key === "Enter" || event.key === " ") && (event.preventDefault(), !props.disabled && menu.select(props.value))}
        style={[look.item, props.destructive && look.destructive, props.disabled && look.disabled]}
      >
        {inside.toArray().map((part) => (typeof part === "string" ? <Text>{part}</Text> : part))}
      </Box>
    )
  },
  Separator: (_props: {}) => <Box role="separator" style={look.separator} />,
  /** Items under a small heading, as iOS's menus group them. */
  Group: (props: { label: JSX.Element; children?: JSX.Element }) => (
    <Box role="group" aria-label={typeof props.label === "string" ? props.label : undefined}>
      <Text aria-hidden="true" style={look.label}>
        {props.label}
      </Text>
      {props.children}
    </Box>
  ),
}

