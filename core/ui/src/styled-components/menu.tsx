// A menu: a button that opens a few things to do, over the page under it. Written once on the drawing primitives,
// the same on the DOM and on a native host.
import { children, createContext, createSignal, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Modal, Pressable, Text, View } from "#primitives"
import { Button, Icon, type ButtonProps } from "./button.tsx"
import { messages } from "../words.ts"
import { Choose, Collect, collected } from "./collect.ts"
import { createMenu, type At, type MenuEntry, type MenuProps } from "../headless-components/menu.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

/** What a menu in a row gives the row (list.tsx): what opens it where the row was held, its `destructive` item, which
 * a swipe shows, and what chooses an item as pressing it does. */
export interface RowMenu {
  open: (at: At) => void
  /** Its items, as a computer's action panel lists them for the row chosen. */
  items: () => MenuEntry[]
  destructive: () => { value: string; label: JSX.Element } | undefined
  select: (value: string) => void
}

/// How a menu in a row's `end` tells the row it's there, and that it's gone. A menu anywhere else, as in a screen's
/// bar, finds none.
export const RowMenu = createContext<((menu: RowMenu | undefined) => void) | null>(null)

/** What a menu's parts share: its state, and the destructive item a row's swipe shows. */
const Opened = createContext<ReturnType<typeof createMenu> & { destructive: () => { value: string; label: JSX.Element } | undefined; setDestructive: (item: { value: string; label: JSX.Element } | undefined) => void }>()

const look = styles({
  catcher: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, pointerEvents: "auto" },
  closed: { display: "none" },
  placed: (top: number, left: number) => ({ position: "absolute", top, left, pointerEvents: "auto" }),
  measuring: { position: "absolute", top: 0, left: 0, opacity: 0 },
  content: {
    minWidth: tokens.length.menu,
    maxWidth: tokens.length.alert,
    padding: tokens.grid[1],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.card,
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

/// `<Menu.Root onSelect={(value) => value === "delete" && remove()}>`, then `<Menu.Trigger />`, then
/// `<Menu.Positioner><Menu.Content>` around `<Menu.Item value="rename">Rename</Menu.Item>`s, a `<Menu.Separator />`
/// between groups, `<Menu.Group label="Install for">` around some under a small heading, and `<Menu.Item
/// value="delete" destructive>Delete</Menu.Item>`: a button that opens a few things to do, under it, over the page. The
/// trigger is three dots, named `aria-label` (More unless said); with words in it, it's a button of `variant` (a
/// `Button`'s, `outline` unless said) that says them, and `size="icon"` keeps one with an icon in it to its icon. An
/// item starts with an `<Icon />` if it's given one, and `destructive` is for what can't be undone. A press outside,
/// Escape and choosing an item close it. In a `Row`'s `end`, a long press or a right-click on the row opens it too,
/// and a finger swipes the row to its `destructive` item.
export const Menu = {
  Root: (props: MenuProps & { children?: JSX.Element }) => {
    const [destructive, setDestructive] = createSignal<{ value: string; label: JSX.Element }>()
    return <Opened value={Object.assign(createMenu(props), { destructive, setDestructive })}>{props.children}</Opened>
  },
  Trigger: (props: { variant?: ButtonProps["variant"]; size?: "icon"; shape?: ButtonProps["shape"]; "aria-label"?: string; children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    // In a row, the row opens it too.
    const row = useContext(RowMenu)
    if (row) {
      row({ open: menu.open, items: menu.items, destructive: menu.destructive, select: menu.select })
      onCleanup(() => row(undefined))
    }
    // Where the screen has an action panel, as a plain list's does on a computer, its items are the panel's, the
    // screen's in its bar or the row's where it's chosen, and a right-click on the row opens it: it draws no button
    // there. Anywhere else it's the ⋯ it always was.
    if (useContext(Collect) || (row && useContext(Choose))) return null
    const inside = children(() => props.children)
    const words = () => inside.toArray().length > 0 && props.size !== "icon"
    const trigger = menu.trigger()
    return (
      <Button
        variant={props.variant ?? (words() ? "outline" : "ghost")}
        size={words() ? undefined : "icon"}
        shape={props.shape}
        aria-label={props["aria-label"] ?? (words() ? undefined : messages.more)}
        aria-haspopup={trigger["aria-haspopup"]}
        aria-expanded={trigger["aria-expanded"]}
        autoFocus={trigger.autoFocus}
        onLayout={trigger.onLayout}
        onPress={menu.toggle}
      >
        {inside.toArray().length ? inside() : <Icon name="dots-three" />}
      </Button>
    )
  },
  /** Where a menu's content shows: in a layer over the page, under what opened it, its end at that's end, or above
   * it where there isn't room. Closed, it's kept, unseen, so a row knows its destructive item. */
  Positioner: (props: { children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    const inside = children(() => props.children)
    return (
      <Show when={menu.at()} fallback={<View style={look.closed}>{inside()}</View>}>
        <Modal>
          <Pressable {...menu.outside()} style={look.catcher} />
          <View style={menu.placed() ? look.placed(menu.placed()!.y, menu.placed()!.x) : look.measuring}>{inside()}</View>
        </Modal>
      </Show>
    )
  },
  /** The items, which the keyboard moves through as WAI-ARIA's menu pattern has it: the arrows to the next and the
   * one before, round from the last to the first, Home and End to the first and the last, a letter to the next that
   * starts with it, and Tab out, closing it. */
  Content: (props: { children?: JSX.Element }) => (
    <View {...useContext(Opened)!.content()} style={look.content}>
      {props.children}
    </View>
  ),
  Item: (props: { value: string; destructive?: boolean; disabled?: boolean; children?: JSX.Element }) => {
    const menu = useContext(Opened)!
    if (props.destructive) {
      menu.setDestructive({ value: props.value, label: props.children })
      onCleanup(() => menu.setDestructive(undefined))
    }
    const inside = children(() => props.children)
    const entry = { value: props.value, label: inside.toArray().filter((part) => typeof part === "string").join(" "), disabled: !!props.disabled, destructive: !!props.destructive }
    onCleanup(menu.add(entry))
    collected(() => ({ label: entry.label, onAction: () => menu.select(props.value), destructive: entry.destructive, disabled: entry.disabled }))
    return (
      <Pressable {...menu.item(() => entry)} style={[look.item, props.destructive && look.destructive, props.disabled && look.disabled]}>
        {inside.toArray().map((part) => (typeof part === "string" ? <Text>{part}</Text> : part))}
      </Pressable>
    )
  },
  Separator: (_props: {}) => <View role="separator" style={look.separator} />,
  /** Items under a small heading, as iOS's menus group them. */
  Group: (props: { label: JSX.Element; children?: JSX.Element }) => (
    <View role="group" aria-label={typeof props.label === "string" ? props.label : undefined}>
      <Text aria-hidden="true" style={look.label}>
        {props.label}
      </Text>
      {props.children}
    </View>
  ),
}

