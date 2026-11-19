// One of several, from a list that opens under it: written once on the drawing primitives (#primitives), the same
// on the DOM and on a native host.
import { createSignal, For, Show } from "solid-js"
import type { CollectionItem, ListCollection } from "@zag-js/collection"
import { Picker, Portal, Pressable, Text, View } from "#primitives"
import { touch, viewport } from "#bridge"
import type { Keyed } from "../drawing-primitives/primitives.tsx"
import type { Located, Rect } from "../types/ui.d.ts"
import { Icon } from "./button.tsx"
import { useField } from "./field.tsx"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  // Under a finger, the device's own picker, see-through over the face, which it opens.
  native: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, width: "100%", height: "100%" },
  select: { minWidth: 0 },
  face: {
    position: "relative",
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2],
    width: "100%",
    minWidth: 0,
    height: tokens.length.control,
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.medium,
    font: tokens.type.field,
    color: tokens.colour.label,
    textAlign: "start",
    cursor: "pointer",
    userSelect: "none",
    boxShadow: { default: tokens.shadow.edge, focused: tokens.shadow.focus },
    transitionProperty: ["boxShadow"],
    transition: tokens.motion.quick,
  },
  wrong: { boxShadow: { default: tokens.shadow.wrongEdge, focused: tokens.shadow.wrongFocus } },
  off: { opacity: 0.5, cursor: "default" },
  chosen: { flexGrow: 1, flexShrink: 1, minWidth: 0, lineClamp: 1 },
  placeholder: { color: tokens.colour.secondaryLabel },
  caret: { color: tokens.colour.secondaryLabel },
  catcher: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, pointerEvents: "auto" },
  anchor: (top: number, start: number, width: number, height: number) => ({ position: "absolute", top, start, width, height }),
  below: { top: "100%", marginTop: tokens.grid[1] },
  above: { bottom: "100%", marginBottom: tokens.grid[1] },
  list: {
    position: "absolute",
    start: 0,
    end: 0,
    pointerEvents: "auto",
    padding: tokens.grid[1],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.card,
    boxShadow: tokens.shadow.window,
    opacity: { default: 1, entering: 0 },
    scale: { default: 1, entering: 0.95 },
    transitionProperty: ["opacity", "scale"],
    transition: tokens.motion.quick,
  },
  item: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2],
    minHeight: tokens.grid[8],
    paddingInline: tokens.grid[2],
    borderRadius: tokens.radius.small,
    font: tokens.type.medium,
    color: tokens.colour.label,
    textAlign: "start",
    cursor: "pointer",
    backgroundColor: { hover: tokens.colour.quinaryLabel },
  },
  highlighted: { backgroundColor: tokens.colour.quinaryLabel },
  itemOff: { opacity: 0.5, cursor: "default" },
  itemText: { flexGrow: 1, flexShrink: 1, minWidth: 0, lineClamp: 1 },
  hidden: { opacity: 0 },
})

/** A `Select.Root`'s props, as Ark UI's select names them. */
export interface SelectProps<T extends CollectionItem = CollectionItem> {
  collection: ListCollection<T>
  value?: string[]
  defaultValue?: string[]
  placeholder?: string
  disabled?: boolean
  "aria-label"?: string
  onValueChange?: (details: { value: string[] }) => void
}

/// `<Field label="Category"><Select.Root collection={categories} value={[category()]} onValueChange={({ value }) => setCategory(value[0])} /></Field>`:
/// one of a few, from a collection (`createListCollection`), in a list that opens under it, as Ark UI's select's
/// props have it: `value` and `onValueChange` (`({ value: ["meals"] })`), and `placeholder` until something's chosen;
/// with none the first is chosen, as a `<select>` does. The list opens over the page, under it or above it where it's
/// in the window's lower half; the arrow keys choose, and Escape and a press elsewhere close it.
export const Select = { Root: SelectRoot }

function SelectRoot<T extends CollectionItem>(props: SelectProps<T>) {
  const field = useField()
  const items = () => props.collection.items
  const value = (item: T) => props.collection.getItemValue(item) ?? ""
  const [own, setOwn] = createSignal<string[]>()
  const chosen = () => props.value ?? own() ?? props.defaultValue ?? (props.placeholder ? [] : [props.collection.firstValue].filter((one): one is string => one != null))
  // Where its face is drawn while its list is open, which the list opens from.
  const [at, setAtRect] = createSignal<Rect>()
  const open = () => !!at()
  const setOpen = (now: false) => setAtRect(now || undefined)
  const [highlighted, setHighlighted] = createSignal(0)
  const off = () => !!props.disabled || !!field?.disabled()
  const itemOff = (item: T) => props.collection.getItemDisabled(item)
  const text = () => chosen().map((one) => props.collection.stringifyItem(props.collection.find(one))).filter(Boolean).join(", ")
  const choose = (item: T) => {
    if (itemOff(item)) return
    setOpen(false)
    if (chosen()[0] === value(item)) return
    setOwn([value(item)])
    props.onValueChange?.({ value: [value(item)] })
  }
  const show = (face: Rect) => {
    if (off()) return
    setHighlighted(Math.max(0, items().findIndex((item) => value(item) === chosen()[0])))
    setAtRect({ x: face.x, y: face.y, width: face.width, height: face.height })
  }
  // The next that can be chosen, `by` one way or the other from `from`.
  const step = (from: number, by: number) => {
    const all = items()
    for (let next = from + by; next >= 0 && next < all.length; next += by) if (!itemOff(all[next])) return next
  }
  const keyed = (event: Keyed) => {
    if (off()) return
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault()
      const by = event.key === "ArrowDown" ? 1 : -1
      // Closed, the arrows choose the next one, as a Mac's pop-up button's do; open, they move through the list.
      if (!open()) {
        const next = step(items().findIndex((item) => value(item) === chosen()[0]), by)
        if (next !== undefined) choose(items()[next])
      } else setHighlighted((now) => step(now, by) ?? now)
    } else if (open() && (event.key === "Enter" || event.key === " ")) {
      event.preventDefault()
      const item = items()[highlighted()]
      if (item) choose(item)
    } else if (open() && (event.key === "Escape" || event.key === "Tab")) {
      if (event.key === "Escape") event.preventDefault()
      setOpen(false)
    }
  }
  return (
    <View style={look.select}>
      <Pressable
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open() ? "true" : "false"}
        aria-disabled={off() ? "true" : undefined}
        aria-label={props["aria-label"] ?? field?.label()}
        aria-description={field?.message()}
        aria-invalid={field?.invalid() ? "true" : undefined}
        onPress={(event: Located) => !touch() && (open() ? setOpen(false) : show(event.currentTarget.getBoundingClientRect()))}
        onKeyDown={keyed}
        onBlur={() => setOpen(false)}
        style={[look.face, field?.invalid() && look.wrong, off() && look.off]}
      >
        <Text style={[look.chosen, !text() && look.placeholder]}>{text() || props.placeholder || " "}</Text>
        <View style={look.caret}>
          <Icon name="caret-up-down" />
        </View>
        <Show when={touch()}>
          <Picker
            aria-label={props["aria-label"] ?? field?.label()}
            enabled={!off()}
            selectedValue={chosen()[0]}
            onValueChange={(chose) => {
              const item = items().find((one) => value(one) === chose)
              if (item) choose(item)
            }}
            style={look.native}
          >
            <For each={items()}>{(item) => <Picker.Item value={value(item)} label={props.collection.stringifyItem(item) ?? ""} enabled={!itemOff(item)} />}</For>
          </Picker>
        </Show>
      </Pressable>
      <Show when={at()}>
        {(face) => (
          <Portal>
            <Pressable style={look.catcher} onPress={() => setOpen(false)} />
            <View style={look.anchor(face().y, face().x, face().width, face().height)}>
              <View role="listbox" style={[look.list, face().y + face().height / 2 > viewport().height / 2 ? look.above : look.below]}>
                <For each={items()}>
                  {(item, i) => {
                    const on = () => chosen().includes(value(item))
                    return (
                      <Pressable
                        role="option"
                        aria-selected={on() ? "true" : "false"}
                        aria-disabled={itemOff(item) ? "true" : undefined}
                        tabIndex={-1}
                        onPress={() => choose(item)}
                        onPointerEnter={() => setHighlighted(i())}
                        style={[look.item, i() === highlighted() && look.highlighted, itemOff(item) && look.itemOff]}
                      >
                        <Text style={look.itemText}>{props.collection.stringifyItem(item)}</Text>
                        <View style={[!on() && look.hidden]}>
                          <Icon name="check" />
                        </View>
                      </Pressable>
                    )
                  }}
                </For>
              </View>
            </View>
          </Portal>
        )}
      </Show>
    </View>
  )
}
