// Something to press, a count or word beside something, an icon and a spinner: written once on the drawing
// primitives (#primitives), the same on the DOM and on a native host.
import { children, createContext, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Path, Pressable, Svg, Text, View } from "#primitives"
import { messages } from "../words.ts"
import { collected } from "./collect.ts"
import { createButton } from "../headless-components/button.ts"
import { negative, styles, type Styles } from "../styles.ts"
import { tokens } from "../tokens.ts"
import type { Laid, Located } from "../types/ui.d.ts"
import arrowElbowDownLeft from "@phosphor-icons/core/assets/regular/arrow-elbow-down-left.svg?raw"
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
import caretDown from "@phosphor-icons/core/assets/regular/caret-down.svg?raw"
import deviceMobile from "@phosphor-icons/core/assets/regular/device-mobile.svg?raw"
import devices from "@phosphor-icons/core/assets/regular/devices.svg?raw"
import globe from "@phosphor-icons/core/assets/regular/globe.svg?raw"
import hardDrives from "@phosphor-icons/core/assets/regular/hard-drives.svg?raw"
import robot from "@phosphor-icons/core/assets/regular/robot.svg?raw"
import minus from "@phosphor-icons/core/assets/regular/minus.svg?raw"
import minusCircle from "@phosphor-icons/core/assets/regular/minus-circle.svg?raw"
import plusCircle from "@phosphor-icons/core/assets/regular/plus-circle.svg?raw"
import squaresFour from "@phosphor-icons/core/assets/regular/squares-four.svg?raw"
import x from "@phosphor-icons/core/assets/regular/x.svg?raw"

// The Phosphor icons the UI library draws with, by Phosphor's own names: each one path in a 256 square.
const svgs = {
  "arrow-elbow-down-left": arrowElbowDownLeft,
  "calendar-blank": calendarBlank,
  "caret-down": caretDown,
  "caret-left": caretLeft,
  "caret-right": caretRight,
  "caret-up-down": caretUpDown,
  check,
  copy,
  "corners-in": cornersIn,
  "corners-out": cornersOut,
  "device-mobile": deviceMobile,
  devices,
  globe,
  "hard-drives": hardDrives,
  robot,
  "dots-three": dotsThree,
  file,
  "gear-six": gearSix,
  "magnifying-glass": magnifyingGlass,
  minus,
  "minus-circle": minusCircle,
  paperclip,
  "pencil-simple": pencilSimple,
  plus,
  "plus-circle": plusCircle,
  "squares-four": squaresFour,
  trash,
  "warning-circle": warningCircle,
  x,
}
const paths = Object.fromEntries(Object.entries(svgs).map(([name, svg]) => [name, svg.match(/ d="([^"]+)"/)![1]])) as Record<string, string>

/** The name of one of the UI library's icons. */
export type IconName = keyof typeof svgs

const look = styles({
  icon: { width: tokens.length.icon, height: tokens.length.icon, flexShrink: 0 },
  spinner: { width: tokens.length.icon, height: tokens.length.icon, flexShrink: 0, animationName: "spin", animation: tokens.motion.spin },
  spinnerArc: { fill: "none", stroke: "currentColor", strokeWidth: 2 },
  button: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: tokens.grid[2],
    height: tokens.length.control,
    paddingInline: tokens.grid[4],
    borderRadius: tokens.radius.medium,
    backgroundColor: { default: tokens.colour.label },
    color: tokens.colour.background,
    font: tokens.type.medium,
    fontWeight: tokens.weight.medium,
    whiteSpace: "nowrap",
    cursor: "pointer",
    userSelect: "none",
    opacity: { default: 1, hover: 0.9, pressed: 0.7 },
    boxShadow: { default: "none", focused: tokens.shadow.focus },
    // Pressed, it dims at once, and comes back at once as it's let go, as a Mac's buttons do: what hover and focus
    // change still fades.
    transitionProperty: { default: ["opacity", "backgroundColor", "boxShadow"], pressed: ["backgroundColor", "boxShadow"] },
    transition: tokens.motion.quick,
  },
  secondary: { backgroundColor: { default: tokens.colour.quinaryLabel, hover: tokens.colour.quaternaryLabel }, color: tokens.colour.label, opacity: { default: 1, pressed: 0.7 } },
  outline: { backgroundColor: { default: tokens.colour.field, hover: tokens.colour.quinaryLabel }, color: tokens.colour.label, boxShadow: { default: tokens.shadow.edge, focused: tokens.shadow.focus }, opacity: { default: 1, pressed: 0.7 } },
  ghost: { backgroundColor: { hover: tokens.colour.quinaryLabel }, color: { default: tokens.colour.secondaryLabel, hover: tokens.colour.label }, opacity: { default: 1, pressed: 0.7 } },
  // Red words on a red tint, as Raycast's: the colour says what it does, not how much it matters.
  destructive: { backgroundColor: { default: tokens.colour.redTint, hover: tokens.colour.redTint }, color: tokens.colour.red },
  square: { width: tokens.length.control, paddingInline: 0, flexShrink: 0 },
  capsule: { borderRadius: tokens.radius.full },
  pulled: { marginEnd: negative(tokens.length.iconInset) },
  disabled: { opacity: 0.5, cursor: "default" },
  /** A menu's button, while its menu is open: one with no fill of its own shows it's the one. */
  open: { backgroundColor: tokens.colour.quinaryLabel },
  badge: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    minWidth: tokens.length.key,
    height: tokens.length.key,
    paddingInline: tokens.grid[1.5],
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.label,
    color: tokens.colour.background,
    font: tokens.type.small,
    fontWeight: tokens.weight.semibold,
    whiteSpace: "nowrap",
  },
  badgeSecondary: { backgroundColor: tokens.colour.quinaryLabel, color: tokens.colour.label },
  badgeDestructive: { backgroundColor: tokens.colour.red, color: tokens.colour.onAccent },
  badgeOutline: { backgroundColor: tokens.colour.field, color: tokens.colour.label, boxShadow: tokens.shadow.edge },
})

/// `<Icon name="plus" />`: one of the UI library's icons, by Phosphor's name for it, an icon's size, in the colour of
/// the text around it. A name the UI library doesn't have throws the names it has. An app that wants another draws
/// Phosphor's path with a `Path`.
export function Icon(props: { name: IconName; along?: boolean }) {
  const d = () => {
    if (!paths[props.name]) throw new Error(`No icon named "${props.name}". There are: ${Object.keys(paths).join(", ")}`)
    return paths[props.name]
  }
  return <Svg viewBox="0 0 256 256" aria-hidden="true" style={look.icon}><Path d={d()} /></Svg>
}

/// `<Spinner />`: something's under way, an icon's size.
export function Spinner() {
  return (
    <View role="status" aria-label={messages.loading}>
      <Svg viewBox="0 0 24 24" style={look.spinner}><Path d="M12 3a9 9 0 1 0 9 9" style={look.spinnerArc} /></Svg>
    </View>
  )
}

/** A `Button`'s props. */
export interface ButtonProps {
  shape?: "capsule"
  variant?: "default" | "secondary" | "outline" | "ghost" | "destructive"
  size?: "icon"
  loading?: boolean
  disabled?: boolean
  "aria-label"?: string
  "aria-haspopup"?: "menu" | "dialog" | "listbox"
  "aria-expanded"?: "true" | "false"
  autoFocus?: boolean
  /** What pressing it does, as React Native's `onPress`. */
  onPress?: (event: Located) => void
  /** Told where it's drawn as it's laid out, as a menu's button is, which its menu opens under. */
  onLayout?: (event: Laid) => void
  /** Where it sits in what it's in, the only part of its look a caller changes. */
  style?: Styles<"marginTop" | "marginBottom" | "marginStart" | "marginEnd" | "alignSelf" | "flexGrow">
  children?: JSX.Element
}

/** Whether what's in it ends a screen's bar, where an icon with no edge of its own lines up with what's under it, as
 * Back does at the bar's start. */
export const BarEnd = createContext(false)

/// `<Button onPress={save}>Save</Button>`: something to press. `variant` says how much it matters: one `default`
/// to a view, then `secondary`, `outline` and `ghost`, and `destructive` for what can't be undone. `size="icon"`
/// is square, for an icon alone (`<Icon name="plus" />`), named by `aria-label`. `shape="capsule"` rounds its ends
/// fully, as SwiftUI's capsule border shape, for one that is a pill of its own (the Home Screen's owner pill). `loading` shows it's working and
/// can't be pressed again. `style` places it, and changes nothing else of its look.
export function Button(props: ButtonProps) {
  const button = createButton({
    get disabled() {
      return props.disabled
    },
    get loading() {
      return props.loading
    },
    get onPress() {
      return props.onPress
    },
  })
  // What it says, made once: words in a text, or what it's given (an icon) as it is.
  const inside = children(() => props.children)
  const end = useContext(BarEnd)
  // In a computer's screen's bar, it's one of the screen's actions, in its footer and its action panel, and draws
  // nothing there itself.
  const words = () => inside.toArray().filter((part) => typeof part === "string").join(" ") || props["aria-label"] || ""
  if (collected(() => ({ label: words(), onAction: () => props.onPress?.(undefined as never), destructive: props.variant === "destructive", disabled: button.off() }))) return null
  return (
    <Pressable
      {...button.root()}
      aria-label={props["aria-label"]}
      aria-haspopup={props["aria-haspopup"]}
      aria-expanded={props["aria-expanded"]}
      autoFocus={props.autoFocus}
      onLayout={props.onLayout}
      style={[look.button, props.variant && props.variant !== "default" && look[props.variant], props.size === "icon" && look.square, props.shape === "capsule" && look.capsule, end && props.variant === "ghost" && props.size === "icon" && look.pulled, button.off() && look.disabled, props["aria-expanded"] === "true" && (props.variant === "outline" || props.variant === "ghost") && look.open, props.style]}
    >
      <Show when={props.loading}>
        <Spinner />
      </Show>
      {inside.toArray().map((part) => (typeof part === "string" ? <Text>{part}</Text> : part))}
    </Pressable>
  )
}

/** A `Badge`'s props. */
export interface BadgeProps {
  variant?: "default" | "secondary" | "destructive" | "outline"
  "aria-label"?: string
  children?: JSX.Element
}

const badges = { default: undefined, secondary: look.badgeSecondary, destructive: look.badgeDestructive, outline: look.badgeOutline }

/// `<Badge variant="destructive" aria-label="3 waiting">3</Badge>`: a short count or word beside what it's about, as
/// shadcn's Badge. `destructive` is the red count of what waits for someone, as on an iPhone's icons; `default`,
/// `secondary` and `outline` are for a word that only says what something is.
export function Badge(props: BadgeProps) {
  return (
    <View aria-label={props["aria-label"]} style={[look.badge, badges[props.variant ?? "default"]]}>
      <Text>{props.children}</Text>
    </View>
  )
}
