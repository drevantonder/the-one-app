import { omit, Show } from "solid-js"
import { said } from "./kit.js"
import calendarBlank from "@phosphor-icons/core/assets/regular/calendar-blank.svg?raw"
import caretLeft from "@phosphor-icons/core/assets/regular/caret-left.svg?raw"
import caretRight from "@phosphor-icons/core/assets/regular/caret-right.svg?raw"
import caretUpDown from "@phosphor-icons/core/assets/regular/caret-up-down.svg?raw"
import check from "@phosphor-icons/core/assets/regular/check.svg?raw"
import dotsThree from "@phosphor-icons/core/assets/regular/dots-three.svg?raw"
import file from "@phosphor-icons/core/assets/regular/file.svg?raw"
import magnifyingGlass from "@phosphor-icons/core/assets/regular/magnifying-glass.svg?raw"
import paperclip from "@phosphor-icons/core/assets/regular/paperclip.svg?raw"
import pencilSimple from "@phosphor-icons/core/assets/regular/pencil-simple.svg?raw"
import plus from "@phosphor-icons/core/assets/regular/plus.svg?raw"
import trash from "@phosphor-icons/core/assets/regular/trash.svg?raw"
import warningCircle from "@phosphor-icons/core/assets/fill/warning-circle-fill.svg?raw"
import x from "@phosphor-icons/core/assets/regular/x.svg?raw"

// The Phosphor icons the kit draws with, by Phosphor's own names, as what's inside each one's <svg>.
const icons = Object.fromEntries(
  Object.entries({
    "calendar-blank": calendarBlank,
    "caret-left": caretLeft,
    "caret-right": caretRight,
    "caret-up-down": caretUpDown,
    check,
    "dots-three": dotsThree,
    file,
    "magnifying-glass": magnifyingGlass,
    paperclip,
    "pencil-simple": pencilSimple,
    plus,
    trash,
    "warning-circle": warningCircle,
    x,
  }).map(([name, svg]) => [name, svg.replace(/^<svg[^>]*>|<\/svg>\s*$/g, "")]),
)

/// `<Icon name="plus" />`: one of the kit's icons, by Phosphor's name for it, the size of the text around it and
/// its colour. A name the kit doesn't have throws the names it has. An app that wants another puts Phosphor's SVG
/// where the icon goes. `along` turns an arrow that points along the line of text where text runs right to left.
export function Icon(props) {
  const paths = () => {
    if (!icons[props.name]) throw new Error(`No icon named "${props.name}". There are: ${Object.keys(icons).join(", ")}`)
    return icons[props.name]
  }
  return <svg class={["ui-icon", { along: props.along }]} viewBox="0 0 256 256" aria-hidden="true" innerHTML={paths()} />
}

/// `<Spinner />`: something's under way. The size of the text around it.
export function Spinner() {
  return (
    <span class="ui-spinner" role="status" aria-label={said.loading}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="9" />
      </svg>
    </span>
  )
}

/// `<Button onClick={save}>Save</Button>`: something to press. `variant` says how much it matters: one `default`
/// to a view, then `secondary`, `outline` and `ghost`, and `destructive` for what can't be undone. `size="icon"`
/// is square, for an icon alone (`<Icon name="plus" />`), named by `aria-label`. `type="submit"` sends the form
/// it's in. `loading` shows it's working and can't be pressed again. Anything else is a `<button>`'s.
export function Button(props) {
  const own = omit(props, "variant", "size", "loading", "children", "type", "disabled")
  return (
    <button
      type={props.type ?? "button"}
      {...own}
      class={["ui-button press ring", props.class]}
      data-variant={props.variant ?? "default"}
      data-size={props.size}
      disabled={props.disabled || props.loading}
      aria-busy={props.loading ? "true" : undefined}
    >
      <Show when={props.loading}>
        <Spinner />
      </Show>
      {props.children}
    </button>
  )
}
