import { children, createContext, For, omit, Show, useContext } from "solid-js"
import { Icon } from "./button.jsx"
import { useField } from "./field.jsx"
import { touch } from "./shared.js"
import { popover } from "./popover.js"
import { machine } from "./zag.jsx"

const { Ark, use, live, draw } = machine("select", () => import("./zag/select.js"), ["div", "getRootProps"], {
  Label: ["label", "getLabelProps"],
  Control: ["div", "getControlProps"],
  Trigger: ["button", "getTriggerProps"],
  ValueText: ["span", "getValueTextProps"],
  Indicator: ["div", "getIndicatorProps"],
  ClearTrigger: ["button", "getClearTriggerProps"],
  Positioner: ["div", "getPositionerProps"],
  Content: ["div", "getContentProps"],
  ItemGroup: ["div", "getItemGroupProps", (zag) => zag.itemGroupProps],
  ItemGroupLabel: ["div", "getItemGroupLabelProps", (zag) => zag.itemGroupLabelProps],
}, {
  defaults: () => {
    const field = useField()
    return () => ({
      ids: { trigger: touch() ? undefined : field?.id, label: field?.label },
      invalid: field?.invalid(),
      positioning: { strategy: "fixed", placement: "bottom-start", sameWidth: true, gutter: 4 },
    })
  },
  // Until its machine is here, its face says what's chosen, and nothing opens.
  still: (root) => {
    const value = root.value ?? root.defaultValue ?? []
    return {
      collection: root.collection,
      value,
      valueAsString: value.map((one) => root.collection.stringifyItem(root.collection.find(one))).join(", "),
      disabled: root.disabled,
    }
  },
})

// An item's parts read which item it is.
const Item = createContext()

/// `<Field label="Category"><Select.Root name="category" collection={categories} /></Field>`: one of a few, from a
/// collection (`createListCollection`). It's Ark UI's select (ark-ui.com/docs/components/select), with its parts
/// and props: `value` and `onValueChange` (`({ value: ["meals"] })`), and `placeholder` until something's chosen;
/// with none the first is chosen, as a `<select>` does. Given no parts, it draws them all, and then under a finger
/// it opens the device's own picker. In a form it's sent as `name`.
export const Select = {
  ...Ark,
  Root: (props) => {
    const inside = children(() => props.children)
    return (
      <Ark.Root
        // Nothing chosen and no placeholder: the first is, as the device's own `<select>` has it.
        defaultValue={props.placeholder ? undefined : [props.collection.firstValue].filter((value) => value != null)}
        {...omit(props, "children", "placeholder")}
        class={["ui-select", props.class]}
      >
        {inside.toArray().length ? inside() : <Whole placeholder={props.placeholder} />}
      </Ark.Root>
    )
  },
  /// What's chosen, or the `placeholder` while nothing is.
  ValueText: (props) => {
    const api = use()
    const text = () => api().valueAsString
    return (
      <Ark.ValueText {...omit(props, "placeholder")} data-placeholder-shown={text() ? undefined : ""}>
        {props.children ?? (text() || props.placeholder)}
      </Ark.ValueText>
    )
  },
  Positioner: (props) => {
    const api = use()
    return <Ark.Positioner {...props} ref={popover(() => api().open, () => api().setOpen(false))} />
  },
  Item: (props) => {
    const api = use()
    const item = () => ({ item: props.item, persistFocus: props.persistFocus })
    return <Item value={item}>{draw("div", () => api().getItemProps(item()), omit(props, "item", "persistFocus"))}</Item>
  },
  ItemText: (props) => {
    const api = use()
    const item = useContext(Item)
    return draw("span", () => api().getItemTextProps(item()), props)
  },
  ItemIndicator: (props) => {
    const api = use()
    const item = useContext(Item)
    return draw("span", () => api().getItemIndicatorProps(item()), props)
  },
  /// The `<select>` a form is sent, which Zag has choose what the select chooses.
  HiddenSelect: (props) => {
    const api = use()
    return (
      <Show when={live()}>
        <select {...omit(props, "children")} {...api().getHiddenSelectProps()}>
          <For each={api().collection.items}>{(item) => <option value={api().collection.getItemValue(item)}>{api().collection.stringifyItem(item)}</option>}</For>
        </select>
      </Show>
    )
  },
}

/// Every part of a select, as the UI library draws one. Under a finger the device's own `<select>` lies over the face,
/// unseen, so a tap opens the device's own picker; with a mouse the face opens ours.
function Whole(props) {
  const api = use()
  const field = useField()
  return (
    <>
      <Select.Control class="ui-picker">
        <Select.Trigger class="ui-box ring" aria-describedby={field?.message}>
          <Select.ValueText placeholder={props.placeholder} />
          <Icon name="caret-up-down" />
        </Select.Trigger>
        <Show when={touch() && live()}>
          <select
            class="native"
            id={field?.id}
            aria-describedby={field?.message}
            aria-invalid={field?.invalid() ? "true" : undefined}
            disabled={api().disabled}
            onChange={(event) => api().setValue(event.target.value ? [event.target.value] : [])}
          >
            <Show when={props.placeholder}>
              <option value="" disabled hidden selected={!api().value.length}>{props.placeholder}</option>
            </Show>
            <For each={api().collection.items}>
              {(item) => {
                const value = () => api().collection.getItemValue(item)
                return <option value={value()} disabled={api().collection.getItemDisabled(item)} selected={api().value.includes(value())}>{api().collection.stringifyItem(item)}</option>
              }}
            </For>
          </select>
        </Show>
      </Select.Control>
      <Show when={!touch()}>
        <Select.Positioner>
          <Select.Content>
            <For each={api().collection.items}>
              {(item) => (
                <Select.Item item={item}>
                  <Select.ItemText>{api().collection.stringifyItem(item)}</Select.ItemText>
                  <Select.ItemIndicator>
                    <Icon name="check" />
                  </Select.ItemIndicator>
                </Select.Item>
              )}
            </For>
          </Select.Content>
        </Select.Positioner>
      </Show>
      <Select.HiddenSelect />
    </>
  )
}
