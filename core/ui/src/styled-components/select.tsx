// One of several, from a list that opens under it: written once on the drawing primitives (#primitives), the same
// on the DOM and on a native host.
import { For, Show } from "solid-js"
import { Picker, Portal, Pressable, Text, View } from "#primitives"
import { touch } from "#bridge"
import type { Located } from "../types/ui.d.ts"
import { createSelect, type SelectProps } from "../headless-components/select.ts"
import { Icon } from "./button.tsx"
import { useField } from "../headless-components/field.ts"
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
  // As wide as its face, under it or above it, once it's measured.
  placed: (top: number, left: number, width: number) => ({ top, left, width, pointerEvents: "auto" }),
  measuring: (width: number) => ({ top: 0, left: 0, width, opacity: 0 }),
  list: {
    position: "absolute",
    padding: tokens.grid[1],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.panel,
    backdropFilter: tokens.material.glass,
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

/// `<Field label="Category"><Select.Root items={[{ value: "meals", label: "Meals" }, ...]} value={category()} onValueChange={setCategory} /></Field>`:
/// one of a few, from `items` (one `disabled: true` can't be chosen), in a list that opens under it, as React Native's
/// `Picker` has them: `value` and `onValueChange`, told the value chosen, and `placeholder` until something's chosen;
/// with none the first is chosen, as a `<select>` does. The list opens over the page, under it or above it where it's
/// in the window's lower half; the arrow keys choose, and Escape and a press elsewhere close it.
export const Select = { Root: SelectRoot }

function SelectRoot(props: SelectProps) {
  const field = useField()
  const select = createSelect(props, { disabled: () => !!field?.disabled() })
  return (
    <View style={look.select}>
      <Pressable
        {...select.face()}
        aria-label={props["aria-label"] ?? field?.label()}
        aria-description={field?.message()}
        aria-invalid={field?.invalid() ? "true" : undefined}
        onPress={(event: Located) => !touch() && select.toggle(event)}
        style={[look.face, field?.invalid() && look.wrong, select.off() && look.off]}
      >
        <Text style={[look.chosen, !select.text() && look.placeholder]}>{select.text() || props.placeholder || " "}</Text>
        <View style={look.caret}>
          <Icon name="caret-up-down" />
        </View>
        <Show when={touch()}>
          <Picker
            aria-label={props["aria-label"] ?? field?.label()}
            enabled={!select.off()}
            selectedValue={select.chosen()}
            onValueChange={(chose) => {
              const item = select.items().find((one) => select.value(one) === chose)
              if (item) select.choose(item)
            }}
            style={look.native}
          >
            <For each={select.items()}>{(item) => <Picker.Item value={select.value(item)} label={select.label(item)} enabled={!select.itemOff(item)} />}</For>
          </Picker>
        </Show>
      </Pressable>
      <Show when={select.at()}>
        {(face) => (
          <Portal>
            <Pressable {...select.outside()} style={look.catcher} />
            <View {...select.list()} style={[look.list, select.placed() ? look.placed(select.placed()!.y, select.placed()!.x, face().width) : look.measuring(face().width)]}>
              <For each={select.items()}>
                {(item, i) => (
                  <Pressable {...select.option(item, i)} style={[look.item, i() === select.highlighted() && look.highlighted, select.itemOff(item) && look.itemOff]}>
                    <Text style={look.itemText}>{select.label(item)}</Text>
                    <View style={[select.chosen() !== select.value(item) && look.hidden]}>
                      <Icon name="check" />
                    </View>
                  </Pressable>
                )}
              </For>
            </View>
          </Portal>
        )}
      </Show>
    </View>
  )
}
