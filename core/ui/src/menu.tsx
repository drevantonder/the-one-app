import { children, createContext, createSignal, omit, onSettled, useContext } from "solid-js"
import { Icon } from "./button.tsx"
import { RowMenu } from "./list.tsx"
import { messages } from "./shared.ts"
import { popover } from "./popover.ts"
import { machine, type Part, type Root } from "./zag.tsx"
import type { ButtonProps } from "./button.tsx"
import type * as menu from "@zag-js/menu"

const { Ark, use, live } = machine("menu", () => import("./zag/menu.ts"), null, {
  Trigger: ["button", "getTriggerProps"],
  Indicator: ["div", "getIndicatorProps"],
  Positioner: ["div", "getPositionerProps"],
  Content: ["div", "getContentProps"],
  Item: ["div", "getItemProps", (zag) => zag.itemProps],
  ItemGroup: ["div", "getItemGroupProps", (zag) => zag.itemGroupProps],
  ItemGroupLabel: ["div", "getItemGroupLabelProps", (zag) => zag.itemGroupLabelProps],
  Separator: ["hr", "getSeparatorProps"],
})

/// `<Menu.Root onSelect={({ value }) => value === "delete" && remove()}>`, then `<Menu.Trigger />`, then
/// `<Menu.Positioner><Menu.Content>` around `<Menu.Item value="rename">Rename</Menu.Item>`s, a `<Menu.Separator />`
/// between groups and `<Menu.Item value="delete" destructive>Delete</Menu.Item>`: a button that opens a few things to
/// do. It's Ark UI's menu (ark-ui.com/docs/components/menu), with its parts and props. The trigger is three dots,
/// named `aria-label` (More unless said); with words in it, it's a button of `variant` (a `Button`'s, `outline`
/// unless said) that says them, and `size="icon"` keeps one with an icon in it to its icon. An item starts with an
/// `<Icon />` if it's given one, and `destructive` is for what can't be undone. In a `Row`'s `end`, the row opens it
/// too, and swipes to its `destructive` item.
export const Menu: {
  Root: Root<menu.Props>
  Trigger: Part<Pick<ButtonProps, "variant" | "size">, HTMLButtonElement>
  Indicator: Part
  Positioner: Part
  Content: Part
  Item: Part<menu.ItemProps & { destructive?: boolean }>
  ItemGroup: Part<menu.ItemGroupProps>
  ItemGroupLabel: Part<menu.ItemGroupLabelProps>
  Separator: Part<{}, HTMLHRElement>
} = {
  ...Ark,
  Root: (props) => {
    const [destructive, setDestructive] = createSignal<HTMLElement>()
    const select = (value: string) => props.onSelect?.({ value } as menu.SelectionDetails)
    return (
      <Own value={{ destructive, setDestructive, select }}>
        <Ark.Root {...props} positioning={{ strategy: "fixed", placement: "bottom-end", gutter: 4, ...props.positioning }} />
      </Own>
    )
  },
  Trigger: (props) => {
    // In a row, the row opens it too.
    const row = useContext(RowMenu)
    if (row && live()) {
      const { destructive, select } = useContext(Own)!
      const api = use()
      onSettled(() => {
        row({ api, destructive, select })
        return () => row(undefined)
      })
    }
    const inside = children(() => props.children)
    const words = () => inside.toArray().length > 0 && props.size !== "icon"
    return (
      <Ark.Trigger
        {...omit(props, "variant", "size", "class", "children")}
        class={["ui-button press ring", props.class]}
        data-variant={props.variant ?? (words() ? "outline" : "ghost")}
        data-size={words() ? undefined : "icon"}
        aria-label={props["aria-label"] ?? (words() ? undefined : messages.more)}
      >
        {inside.toArray().length ? inside() : <Icon name="dots-three" />}
      </Ark.Trigger>
    )
  },
  Positioner: (props) => {
    const api = use()
    return <Ark.Positioner {...props} ref={popover(() => api().open, () => api().setOpen(false))} />
  },
  Item: (props) => {
    const own = useContext(Own)
    let item: HTMLElement | undefined
    onSettled(() => {
      if (!props.destructive || !item) return
      own?.setDestructive(item)
      return () => own?.setDestructive((now) => (now === item ? undefined : now))
    })
    return <Ark.Item {...omit(props, "destructive")} ref={(element: HTMLElement) => (item = element)} data-destructive={props.destructive ? "" : undefined} />
  },
}

/// What a menu's parts share beyond Zag's: its `destructive` item, and what selects an item as pressing it does.
const Own = createContext<{ destructive: () => HTMLElement | undefined; setDestructive: (item: HTMLElement | undefined | ((now: HTMLElement | undefined) => HTMLElement | undefined)) => void; select: (value: string) => void }>()
