import { omit, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { messages } from "./shared.ts"
import calendarBlank from "@phosphor-icons/core/assets/regular/calendar-blank.svg?raw"
import caretLeft from "@phosphor-icons/core/assets/regular/caret-left.svg?raw"
import caretRight from "@phosphor-icons/core/assets/regular/caret-right.svg?raw"
import caretUpDown from "@phosphor-icons/core/assets/regular/caret-up-down.svg?raw"
import check from "@phosphor-icons/core/assets/regular/check.svg?raw"
import copy from "@phosphor-icons/core/assets/regular/copy.svg?raw"
import cornersIn from "@phosphor-icons/core/assets/regular/corners-in.svg?raw"
import cornersOut from "@phosphor-icons/core/assets/regular/corners-out.svg?raw"
import dotsThree from "@phosphor-icons/core/assets/regular/dots-three.svg?raw"
import file from "@phosphor-icons/core/assets/regular/file.svg?raw"
import gearSix from "@phosphor-icons/core/assets/fill/gear-six-fill.svg?raw"
import magnifyingGlass from "@phosphor-icons/core/assets/regular/magnifying-glass.svg?raw"
import paperclip from "@phosphor-icons/core/assets/regular/paperclip.svg?raw"
import pencilSimple from "@phosphor-icons/core/assets/regular/pencil-simple.svg?raw"
import plus from "@phosphor-icons/core/assets/regular/plus.svg?raw"
import trash from "@phosphor-icons/core/assets/regular/trash.svg?raw"
import warningCircle from "@phosphor-icons/core/assets/fill/warning-circle-fill.svg?raw"
import x from "@phosphor-icons/core/assets/regular/x.svg?raw"

// The Phosphor icons the UI library draws with, by Phosphor's own names, as what's inside each one's <svg>.
const svgs = {
  "calendar-blank": calendarBlank,
  "caret-left": caretLeft,
  "caret-right": caretRight,
  "caret-up-down": caretUpDown,
  check,
  copy,
  "corners-in": cornersIn,
  "corners-out": cornersOut,
  "dots-three": dotsThree,
  file,
  "gear-six": gearSix,
  "magnifying-glass": magnifyingGlass,
  paperclip,
  "pencil-simple": pencilSimple,
  plus,
  trash,
  "warning-circle": warningCircle,
  x,
}
const icons = Object.fromEntries(Object.entries(svgs).map(([name, svg]) => [name, svg.replace(/^<svg[^>]*>|<\/svg>\s*$/g, "")])) as Record<string, string>

/** The name of one of the UI library's icons. */
export type IconName = keyof typeof svgs

/// `<Icon name="plus" />`: one of the UI library's icons, by Phosphor's name for it, the size of the text around it and
/// its colour. A name the UI library doesn't have throws the names it has. An app that wants another puts Phosphor's SVG
/// where the icon goes. `along` turns an arrow that points along the line of text where text runs right to left.
export function Icon(props: { name: IconName; along?: boolean }) {
  const paths = () => {
    if (!icons[props.name]) throw new Error(`No icon named "${props.name}". There are: ${Object.keys(icons).join(", ")}`)
    return icons[props.name]
  }
  return <svg class={["ui-icon", { along: props.along }]} viewBox="0 0 256 256" aria-hidden="true" innerHTML={paths()} />
}

/// `<Spinner />`: something's under way. The size of the text around it.
export function Spinner() {
  return (
    <span class="ui-spinner" role="status" aria-label={messages.loading}>
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="9" />
      </svg>
    </span>
  )
}

/** A `Button`'s props: a `<button>`'s, and how it looks. */
export interface ButtonProps extends JSX.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "secondary" | "outline" | "ghost" | "destructive"
  size?: "icon"
  loading?: boolean
}

/// `<Button onClick={save}>Save</Button>`: something to press. `variant` says how much it matters: one `default`
/// to a view, then `secondary`, `outline` and `ghost`, and `destructive` for what can't be undone. `size="icon"`
/// is square, for an icon alone (`<Icon name="plus" />`), named by `aria-label`. `type="submit"` sends the form
/// it's in. `loading` shows it's working and can't be pressed again. Anything else is a `<button>`'s.
export function Button(props: ButtonProps) {
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
