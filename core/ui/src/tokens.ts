// The design tokens: every colour, length, radius, shadow, material, font, weight, type style and pace the UI library
// and every app draw with, as typed values, each the same everywhere or varying by condition (dark, touch, phone,
// reduced motion: drawing-primitives/renderer.ts's `Condition`). This is their one source. `tokens` are by role, as Raycast's
// model has them (text at 100, 60, 40, 20 and 10%, a background, a selection, a loader, seven accents each with a
// tint, a 4px grid, three radii, a window's shadow and edge, three sizes of text and a large title), and a page's code
// names only those, so their values change here alone. `legacy` are what the UI library's components drawn with CSS
// name, until each is written on the drawing primitives. The UI library's build makes from it the web's
// CSS variables (tokens.css, which isn't edited by hand) and a native host's token table (/ui/client/tokens.json), and
// a style names a token here by reference (`tokens.colour.mutedForeground`), never by its value.
//
// Values are Raycast's, as the round-two mockup measured them beside it (DESIGN.md), light and dark. The typeface is Allo (allo.joshpuckett.me, github.com/joshpuckett/
// allo v1.0, SIL Open Font License with no Reserved Font Name: Allo-LICENSE.txt), served beside the UI library's
// stylesheet so nothing comes from a font host; code is in the device's own monospace, and a script Allo lacks in the
// device's own font. Allo-Variable.woff2 is its AlloVariable.ttf with roundness pinned at 100 and only Latin kept:
//   fonttools varLib.instancer AlloVariable.ttf ROND=100 -o allo.ttf
//   pyftsubset allo.ttf --layout-features='*' --flavor=woff2 --output-file=Allo-Variable.woff2 --unicodes=U+0000-00FF,
//     U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,
//     U+2193,U+2212,U+2215,U+FEFF,U+FFFD
import type { Condition, Length, Varies } from "./drawing-primitives/renderer.ts"

/** A token of a kind, whose `value` a style never reads: it names the token, and the build resolves it. */
export interface Token<Kind extends string = string> {
  kind: Kind
  /** Its id, which is its CSS variable's name: `muted-foreground` is `var(--muted-foreground)`. */
  id: string
  value: unknown
}

export type { Condition }

const colour = (light: string, dark = light): Token<"colour"> => ({ kind: "colour", id: "", value: light === dark ? light : { default: light, dark } })
const length = (value: Varies<Length>): Token<"length"> => ({ kind: "length", id: "", value })
const radius = (value: Varies<Length | { percent: number }>): Token<"radius"> => ({ kind: "radius", id: "", value })
const rem = (rem: number): Length => ({ rem })
const px = (px: number): Length => ({ px })

/** A shadow: its offsets, blur and spread in pixels, its colour, and whether it's inside the edge. */
type Shadow = { x: number; y: number; blur: number; spread: number; colour: string; inset?: boolean }
const shadow = (...shadows: Shadow[]): Token<"shadow"> => ({ kind: "shadow", id: "", value: shadows })
/** A shadow that's one in light and another in dark. */
const lightDark = (light: Shadow[], dark: Shadow[]): Token<"shadow"> => ({ kind: "shadow", id: "", value: { default: light, dark } })
const font = (...families: string[]): Token<"font"> => ({ kind: "font", id: "", value: families })
/** A weight, a little lighter in dark: light text on dark looks heavier. */
const weight = (weight: number): Token<"weight"> => ({ kind: "weight", id: "", value: { default: weight, dark: weight - 25 } })
/** A colour as another at a part of its opacity: text at 60%, an accent's 15% tint. */
const alpha = (of: Token<"colour">, percent: number): Token<"colour"> => ({ kind: "colour", id: "", value: { of, alpha: percent / 100 } })
/** What shows through what has it: what's behind, blurred so far and saturated so much. */
/** What shows through what has it: what's behind, blurred so far and saturated so much; `system`, the system's own
 * material where the host has one, as a launcher panel's glass is under the window (a Mac's), and else blurred as any. */
const material = (blur: number, saturate: number, system?: boolean): Token<"material"> => ({ kind: "material", id: "", value: { blur: px(blur), saturate, ...(system ? { system } : {}) } })
const motion = (duration: Varies<number>, easing: Varies<[number, number, number, number]>): Token<"motion"> => ({ kind: "motion", id: "", value: { duration, easing } })
/** A spring, as SwiftUI's `.spring(duration:bounce:)` and Motion's are given: about how long it takes to get there, in
 * milliseconds, and how far past it goes, from 0 (not at all) to 1. A change on its way is turned to the new value
 * from where it is, at the speed it has. `easing` is for a host that draws only Béziers. */
const spring = (duration: number, bounce: number): Token<"motion"> => ({ kind: "motion", id: "", value: { duration, bounce, easing: [0.22, 1, 0.36, 1] } })

// Raycast's values as round two measured them beside it (raycast-mockups-v2): a near-white window, one text colour,
// and Apple's accents but red, orange and blue, which are the mockup's. Dark is true black, an OLED's (20 Nov), with
// what's raised off it neutral greys, no tint. Colours CSS names are written in OKLCH.
const ink = ["oklch(0.224 0.008 285.76)", "oklch(1 0 0)"] as const
const red = ["oklch(0.614 0.204 25.56)", "oklch(0.7 0.191 23.4)"] as const
const orange = ["oklch(0.677 0.164 57.12)", "oklch(0.782 0.171 67.22)"] as const
const blue = ["oklch(0.573 0.199 261.84)", "oklch(0.714 0.139 249.06)"] as const
const colours = {
  background: colour("oklch(0.977 0.003 286.35)", "oklch(0 0 0)"),
  foreground: colour(...ink),
  /** What's raised off the page: a list, a menu, a sheet. */
  card: colour("oklch(1 0 0)", "oklch(0.145 0 0)"),
  primary: colour(...ink),
  primaryForeground: colour("oklch(1 0 0)", "oklch(0 0 0)"),
  /** A quiet fill: a secondary button, what's hovered, a skeleton: the text at 10% over the page. */
  muted: colour("oklch(0.919 0.003 286.35)", "oklch(0.269 0 0)"),
  /** Grey words, and where the keyboard is: the text at 60%. */
  mutedForeground: colour("oklch(0.224 0.008 285.76 / 60%)", "oklch(1 0 0 / 60%)"),
  destructive: colour(...red),
  /** Warn: "this needs you", but it can wait. */
  warning: colour(...orange),
  /** A hairline, a field's edge, and what a pressed row fills with: the text at 10%. */
  border: colour("oklch(0.224 0.008 285.76 / 10%)", "oklch(1 0 0 / 10%)"),
  /** What dims the page under a sheet: a light page is washed, a dark one dimmed. */
  overlay: colour("oklch(1 0 0 / 25%)", "oklch(0 0 0 / 40%)"),
}

/** Lengths. A computer's are rems of 16px; under a finger what's pressed is a 44px target and text a step larger, which
 * a field never shows under 16px, where an iPhone would zoom into it; an iPhone's and iPad's rem is the text size its
 * person chose (Dynamic Type), 17px unless they changed it, and its type styles are iOS's own at that size. */
const lengths = {
  /** Body text and what's typed: Raycast's 13px on a computer. */
  textBase: length({ default: rem(0.8125), touch: rem(1) }),
  textLg: length({ default: rem(0.9375), phone: rem(1) }),
  textTitle: length({ default: rem(1.25), phone: rem(28 / 17) }),
  /** A big number's: 34px, as iOS's widgets draw one. */
  textFigure: length({ default: rem(2.125), phone: rem(34 / 17) }),
  /** Apple's footnote, the same under a finger and a mouse: for a count, code or a hint. */
  textFootnote: length(rem(0.8125)),
  /** What's typed in a field: body, but never under 16px under a finger. */
  textField: length({ default: rem(0.8125), touch: rem(1) }),
  /** How tall what's pressed or typed in is, and an icon in it. */
  control: length({ default: rem(2), touch: rem(2.75), phone: rem(44 / 17) }),
  icon: length({ default: rem(1), touch: rem(1.125), phone: rem(19 / 17) }),
  /** A screen's gutter and the gap between its parts, which on an iPhone stay put as its text grows; and where lists
   * and forms stop growing. */
  gutter: length({ default: rem(1), phone: px(16) }),
  readable: length(rem(42)),
  /** A border: a hairline. */
  hairline: length(px(1)),
}

const radii = {
  sm: radius(rem(0.375)),
  md: radius(rem(0.5625)),
  xl: radius(rem(0.875)),
  /** A pill, or a circle. */
  full: radius(px(9999)),
  /** An app's icon, as iOS rounds one. */
  icon: radius({ percent: 22.5 }),
}

const shadows = {
  xs: shadow({ x: 0, y: 1, blur: 2, spread: 0, colour: "oklch(0 0 0 / 5%)" }),
  lg: shadow({ x: 0, y: 10, blur: 15, spread: -3, colour: "oklch(0 0 0 / 10%)" }, { x: 0, y: 4, blur: 6, spread: -4, colour: "oklch(0 0 0 / 10%)" }),
}

const fonts = {
  sans: font("Allo", "-apple-system", "BlinkMacSystemFont", "Segoe UI Variable", "Segoe UI", "system-ui", "sans-serif"),
  mono: font("ui-monospace", "SF Mono", "Cascadia Mono", "monospace"),
}

/** Weights between Allo's named ones. */
const weights = {
  body: weight(400),
  label: weight(500),
  title: weight(600),
}

/** A type style (DESIGN.md: text is one of a few): body; label, for what's pressed, named or a row's heading;
 * headline, a section's or a card's title; title, a screen's, a dialog's and a big widget text; figure, a big number;
 * and Apple's smaller ones. Big titles and figures are tracked in, and figures are tabular, as numbers that change
 * or line up are. Allo's optical size follows the text's size by itself. */
const type = (size: Token<"length">, weight: Token<"weight">, { lineHeight = 1.5, tracking = 0, tabular = false } = {}): Token<"type"> => ({ kind: "type", id: "", value: { font: fonts.sans, size, lineHeight, weight, tracking, tabular } })
const types = {
  body: type(lengths.textBase, weights.body),
  label: type(lengths.textBase, weights.label),
  headline: type(lengths.textLg, weights.title, { tracking: -0.015 }),
  title: type(lengths.textTitle, weights.title, { lineHeight: 1.2, tracking: -0.015 }),
  /** A big number, as an iOS widget's and Raycast's large numbers: 34px, tracked in, its figures tabular. */
  figure: type(lengths.textFigure, weights.title, { lineHeight: 1.1, tracking: -0.03, tabular: true }),
  footnote: type(lengths.textFootnote, weights.body),
}

/** How things move. Quick: what goes, what's small (a press, a fade, a menu). Settle: what arrives or shifts in
 * place. Travel: what crosses the screen (a sheet), which for whoever asked for less motion fades instead, as briefly
 * as what settles. */
const out: [number, number, number, number] = [0.22, 1, 0.36, 1]
const motions = {
  quick: motion(120, out),
  settle: motion(200, out),
  travel: motion({ default: 300, reducedMotion: 200 }, { default: [0.32, 0.72, 0, 1], reducedMotion: out }),
  /** Springs, by SwiftUI's names and with its values (`Animation.snappy`, `.smooth`: half a second). Snappy: what
   * changes size or place and can be turned back midway, a little past where it goes. Smooth: what fades or settles
   * beside it, with no bounce. */
  snappy: spring(500, 0.15),
  smooth: spring(500, 0),
  /** Pages settling where a swipe let them go, as UIScrollView's paging, SpringBoard's pages', does: its curve recorded
   * on iOS 26's simulator and fitted, a spring from the finger's speed (a stiffness of 420 and damping of 39.5), at
   * rest in a third of a second. */
  paging: spring(307, 0.036),
}

// By role.

/** Raycast's window shadow, light and dark, and its edge inside, as round two measured them. */
const windowShadow: Record<"light" | "dark", Shadow[]> = {
  light: [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 16%)" }, { x: 0, y: 4, blur: 8, spread: 0, colour: "oklch(0 0 0 / 6%)" }, { x: 0, y: 12, blur: 32, spread: 0, colour: "oklch(0 0 0 / 14%)" }, { x: 0, y: 16, blur: 48, spread: 8, colour: "oklch(0 0 0 / 6%)" }],
  dark: [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 75%)" }, { x: 0, y: 4, blur: 8, spread: 0, colour: "oklch(0 0 0 / 40%)" }, { x: 0, y: 12, blur: 32, spread: 0, colour: "oklch(0 0 0 / 60%)" }, { x: 0, y: 16, blur: 48, spread: 8, colour: "oklch(0 0 0 / 20%)" }],
}
const edge: Record<"light" | "dark", Shadow[]> = {
  light: [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 10%)", inset: true }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 90%)", inset: true }],
  dark: [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(1 0 0 / 14%)", inset: true }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 12%)", inset: true }],
}

const label = colour(...ink)
const accents = {
  red: colour(...red),
  orange: colour(...orange),
  yellow: colour("#ffcc00", "#ffd60a"),
  green: colour("#34c759", "#30d158"),
  blue: colour(...blue),
  purple: colour("#af52de", "#bf5af2"),
  magenta: colour("#ff2d55", "#ff375f"),
}
/** Colours: one text colour at five strengths, a background and the second of its gradient, what's selected, what's
 * loading, and the accents, each with its tint. */
const roles = {
  label,
  secondaryLabel: alpha(label, 60),
  tertiaryLabel: alpha(label, 40),
  quaternaryLabel: alpha(label, 20),
  quinaryLabel: alpha(label, 10),
  background: colours.background,
  /** What's raised off the page: a group of rows, a menu, a problem, what's loading in place of a list. */
  card: colours.card,
  backgroundGradient: colour("oklch(0.962 0.004 286.32)", "oklch(0 0 0)"),
  selection: alpha(label, 10),
  /** Every other row of metadata: the text at 2.5%. */
  stripe: alpha(label, 2.5),
  /** What dims the page under a sheet or an alert. */
  overlay: colours.overlay,
  /** A panel of glass, over the window or the desktop: the action panel, an alert, the launcher. Tinted as macOS's
   * widgets and Control Center are, so text stays readable over any window behind it: secondary text holds WCAG AA's
   * 4.5:1 over a white one in dark. */
  panel: colour("oklch(0.991 0.001 286.38 / 92%)", "oklch(0.185 0 0 / 92%)"),
  /** Liquid Glass's fill, over what's blurred behind it: mostly white in light, a grey a little lighter than black in
   * dark, a fifth see-through, so it reads over anything. */
  glass: colour("oklch(1 0 0 / 80%)", "oklch(0.26 0 0 / 80%)"),
  /** What veils a page behind something opened over it, over what's blurred. */
  veil: alpha(colours.background, 55),
  /** A tile's tint, as an App Library category's square is. */
  tile: alpha(label, 14),
  /** What's typed in: a field's fill, a little lighter than what it's on. */
  field: colour("oklch(1 0 0 / 70%)", "oklch(1 0 0 / 6%)"),
  /** Nothing: what has no fill of its own where something else has one. */
  transparent: colour("oklch(0 0 0 / 0%)"),
  /** Words and icons on an accent. */
  onAccent: colour("oklch(1 0 0)"),
  loader: alpha(label, 40),
  ...accents,
  redTint: alpha(accents.red, 15),
  orangeTint: alpha(accents.orange, 15),
  yellowTint: alpha(accents.yellow, 15),
  greenTint: alpha(accents.green, 15),
  blueTint: alpha(accents.blue, 15),
  purpleTint: alpha(accents.purple, 15),
  magentaTint: alpha(accents.magenta, 15),
}

/** The 4px grid, by its steps: `grid[3]` is 12px. */
const grid = Object.fromEntries([0.5, 1, 1.5, 2, 2.5, 3, 4, 5, 6, 7, 8, 10, 12, 16, 20, 24].map((step) => [step, length(rem(step / 4))])) as Record<0.5 | 1 | 1.5 | 2 | 2.5 | 3 | 4 | 5 | 6 | 7 | 8 | 10 | 12 | 16 | 20 | 24, Token<"length">>

/** Sizes of text: large, medium and small, and a phone's large title. */
const sizes = {
  large: length({ default: rem(1.125), phone: rem(1) }),
  medium: length({ default: rem(0.8125), touch: rem(0.9375), phone: rem(15 / 17) }),
  small: length({ default: rem(0.6875), touch: rem(0.8125), phone: rem(13 / 17) }),
  /** A computer's bar's title: Raycast's 15px. */
  barTitle: length(rem(0.9375)),
  /** A row's heading in grouped cards and a sidebar on a computer, as Raycast's Settings draws them: 13.5px and 14px;
   * under a finger, a row's own. */
  groupedRow: length({ default: rem(0.84375), touch: rem(0.9375), phone: rem(15 / 17) }),
  sidebarRow: length({ default: rem(0.875), touch: rem(0.9375), phone: rem(15 / 17) }),
  /** A footer pill's and a toast's: Raycast's 12px. */
  caption: length({ default: rem(0.75), touch: rem(0.8125), phone: rem(13 / 17) }),
  largeTitle: length({ default: rem(1.25), phone: rem(34 / 17) }),
}
const weightRoles = {
  regular: weights.body,
  medium: weights.label,
  semibold: weights.title,
}

/** What the UI library's components drawn with CSS name, as their CSS variables, until each is written on the drawing primitives. */
export const legacy = {
  colour: named(colours, (key) => key),
  length: named(lengths, (key) => key),
  radius: named(radii, (key) => `radius-${key}`),
  shadow: named(shadows, (key) => `shadow-${key}`),
  weight: named(weights, (key) => `${key}-weight`),
  type: named(types, (key) => `type-${key}`),
}

export const tokens = {
  colour: named(roles, (key) => key),
  grid: named(grid, (key) => `grid-${key.replace(".", "-")}`),
  length: named({
    control: lengths.control,
    icon: lengths.icon,
    /** How far an icon sits inside the edge of a square button of it, which a button with no edge of its own at the
     * end of a row pulls out by, so the icon lines up with what's under it: half of `control` less `icon`. */
    iconInset: length({ default: rem(0.5), touch: rem(0.8125), phone: rem(12.5 / 17) }),
    /** How tall a screen's bar is at least: a control's height and a little, Raycast's 52px on a computer. */
    bar: length({ default: rem(3.25), touch: rem(3.5), phone: rem(44 / 17 + 0.75) }),
    gutter: lengths.gutter,
    readable: lengths.readable,
    hairline: lengths.hairline,
    /** A split's list beside what it opens: its widest and its narrowest. */
    column: length(rem(24)),
    columnMin: length(rem(18)),
    /** A window's sidebar, as macOS's and Raycast's Settings': 240px. */
    sidebar: length(rem(15)),
    /** How tall a block of code grows before it scrolls. */
    excerpt: length(rem(12)),
    /** What the device keeps for itself at each edge of the window (a status bar, a notch, a home indicator), which
     * the host sets as it changes: on the web the client says it (`safeAreaInsets`) and the bridge sets it. */
    safeTop: length(px(0)),
    safeBottom: length(px(0)),
    safeLeft: length(px(0)),
    safeRight: length(px(0)),
    /** Where a computer's window has its title row over the page, which its top bar is (`titleBar` in its host
     * context): where in it the page's own may start, after the window's controls and breadcrumb, and how far from
     * its end, before the controls there. With none, a gutter's. The host sets them as they change. */
    titleBarStart: length(rem(1)),
    titleBarEnd: length(rem(1)),
    sheet: length({ default: rem(34), touch: rem(28) }),
    alert: length(rem(18.75)),
    menu: length(rem(10)),
    /** A sheet's grabber, which says under a finger that it can be pulled down, and is nothing under a mouse. */
    grabber: length({ default: px(0), touch: rem(0.25) }),
    grabberGap: length({ default: px(0), touch: rem(0.5) }),
    /** A footer's pill, and what a window's foot keeps clear for its pills: Raycast's 36px, 10px off the edge. */
    pill: length(rem(2.25)),
    footer: length(rem(3.5)),
    /** A form's labels on a computer, beside what they name, as Raycast's and macOS's forms have them. */
    formLabel: length(rem(9.5)),
    /** A key's chip, as ⌘ and K are shown beside what they do. */
    key: length(rem(1.125)),
    /** The action panel's width. */
    actionPanel: length(rem(20)),
    /** How tall a row in a list is at least: what's pressed, and a little room. */
    row: length({ default: rem(2.375), touch: rem(3.25), phone: rem(52 / 17) }),
    /** A row in grouped cards, as Raycast's Settings and iOS's Settings draw them: 46px on a computer. */
    groupedRow: length({ default: rem(2.875), touch: rem(3.25), phone: rem(52 / 17) }),
    /** A sidebar's row on a computer, as macOS's and Raycast's Settings' sidebars draw them. */
    sidebarRow: length({ default: rem(2.0625), touch: rem(3.25), phone: rem(52 / 17) }),
    /** An icon at a row's start in a sidebar and in grouped cards, as Raycast's Settings: 22px and 28px on a computer,
     * 30px under a finger. */
    sidebarIcon: length({ default: rem(1.375), touch: rem(1.875), phone: rem(30 / 17) }),
    groupedIcon: length({ default: rem(1.75), touch: rem(1.875), phone: rem(30 / 17) }),
  }, (key) => key),
  size: named(sizes, (key) => `size-${key}`),
  radius: named({ small: radius(px(6)), medium: radius(px(10)), large: radius(px(18)), window: radius(px(26)), full: radii.full, icon: radii.icon }, (key) => `radius-${key}`),
  shadow: named({
    /** Liquid Glass's: a hairline rim, a line of light along its top, and a soft shadow under it. */
    glass: lightDark(
      [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 12%)" }, { x: 0, y: 1, blur: 3, spread: 0, colour: "oklch(0 0 0 / 6%)" }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 90%)", inset: true }, { x: 0, y: 8, blur: 24, spread: 0, colour: "oklch(0 0 0 / 18%)" }],
      [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(1 0 0 / 14%)", inset: true }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 18%)", inset: true }, { x: 0, y: 8, blur: 24, spread: 0, colour: "oklch(0 0 0 / 25%)" }],
    ),
    /** What's carried by a finger: a soft drop shadow. */
    lifted: shadow({ x: 0, y: 12, blur: 28, spread: -6, colour: "oklch(0 0 0 / 30%)" }),
    /** Where the keyboard is: a blue edge and a ring of its tint. */
    focus: lightDark(
      [{ x: 0, y: 0, blur: 0, spread: 1, colour: "oklch(0.573 0.199 261.84)", inset: true }, { x: 0, y: 0, blur: 0, spread: 3, colour: "oklch(0.573 0.199 261.84 / 30%)" }],
      [{ x: 0, y: 0, blur: 0, spread: 1, colour: "oklch(0.714 0.139 249.06)", inset: true }, { x: 0, y: 0, blur: 0, spread: 3, colour: "oklch(0.714 0.139 249.06 / 30%)" }],
    ),
    /** A window's, a sheet's or a menu's, as Raycast's: a hairline outside, three soft shadows under it, and its edge. */
    window: lightDark([...windowShadow.light, ...edge.light], [...windowShadow.dark, ...edge.dark]),
    /** A floating pill's, as Raycast's footer has them: a hairline round it and a line of light along its top. */
    pill: lightDark(
      [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 12%)" }, { x: 0, y: 1, blur: 3, spread: 0, colour: "oklch(0 0 0 / 6%)" }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 90%)", inset: true }],
      [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(1 0 0 / 12%)", inset: true }, { x: 0, y: 0.5, blur: 0, spread: 0, colour: "oklch(1 0 0 / 18%)", inset: true }],
    ),
    /** A field's edge: a half-pixel line inside. */
    field: lightDark([{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(0 0 0 / 14%)", inset: true }], [{ x: 0, y: 0, blur: 0, spread: 0.5, colour: "oklch(1 0 0 / 12%)", inset: true }]),
    /** A key's chip: an outline, as Raycast draws ⌘ and K. */
    key: lightDark([{ x: 0, y: 0, blur: 0, spread: 1, colour: "oklch(0 0 0 / 16%)", inset: true }], [{ x: 0, y: 0, blur: 0, spread: 1, colour: "oklch(1 0 0 / 20%)", inset: true }]),
    /** A raised thing's edge inside: a half-pixel line, lit along the top. */
    edge: lightDark(edge.light, edge.dark),
    /** A field's edge, and where the keyboard is in it, where what's in it is wrong. */
    wrongEdge: lightDark([{ x: 0, y: 0, blur: 0, spread: 1, colour: red[0], inset: true }], [{ x: 0, y: 0, blur: 0, spread: 1, colour: red[1], inset: true }]),
    wrongFocus: lightDark([{ x: 0, y: 0, blur: 0, spread: 3, colour: "oklch(0.614 0.204 25.56 / 30%)" }], [{ x: 0, y: 0, blur: 0, spread: 3, colour: "oklch(0.7 0.191 23.4 / 30%)" }]),
  }, (key) => `shadow-${key}`),
  material: named({ glass: material(20, 1.6), thin: material(18, 1.2), window: material(30, 1.8, true) }, (key) => `material-${key}`),
  font: named(fonts, (key) => `font-${key}`),
  weight: named(weightRoles, (key) => `weight-${key}`),
  type: named({
    largeTitle: type(sizes.largeTitle, weights.title, { lineHeight: 1.2, tracking: -0.015 }),
    /** A big number: a widget's stat. */
    figure: types.figure,
    /** A bar's title, which runs to a second line before it's cut. */
    title: type(sizes.large, weights.title, { lineHeight: 1.3, tracking: -0.015 }),
    /** A computer's bar's title, deeper in an app, beside its Back: Raycast's 15px. */
    barTitle: type(sizes.barTitle, weights.label, { lineHeight: 1.3, tracking: -0.01 }),
    large: type(sizes.large, weights.label),
    medium: type(sizes.medium, weights.body, { lineHeight: 1.35 }),
    groupedRow: type(sizes.groupedRow, weights.body, { lineHeight: 1.35 }),
    sidebarRow: type(sizes.sidebarRow, weights.body, { lineHeight: 1.35 }),
    small: type(sizes.small, weights.body),
    caption: type(sizes.caption, weights.body, { lineHeight: 1.35 }),
    /** What's typed in a field: medium, but never under 16px under a finger, where an iPhone would zoom into it. */
    field: type(lengths.textField, weights.body),
  }, (key) => `type-${key}`),
  motion: named({ ...motions, spin: motion(1000, [0, 0, 1, 1]), hold: motion(500, out), pulse: motion(2000, [0.4, 0, 0.6, 1]) }, (key) => `move-${key}`),
}


/** Each token's id, its CSS variable's name, as its kind's group names it: a colour by itself (`secondary-label`), a
 * length as it's known (`control`), and the rest by their group (`grid-3`, `radius-small`, `type-medium`,
 * `move-quick`). A token in two groups keeps the first's: `legacy`'s, which its CSS names, until it goes. */
function named<T extends Record<string, Token>>(group: T, id: (key: string) => string): T {
  for (const [key, token] of Object.entries(group)) token.id ||= id(key.replace(/([a-z])([A-Z0-9])/g, (_, before, letter) => `${before}-${letter.toLowerCase()}`).replace(/^x(x+)l$/, (_, more) => `${more.length + 1}xl`))
  return group
}

/** How long a motion token takes by default, in milliseconds, for what a page times itself as it moves. */
export const duration = (token: Token<"motion">): number => {
  const value = (token.value as { duration: number | { default: number } }).duration
  return typeof value === "number" ? value : value.default
}

/** How long a length token is by default, in CSS pixels, at the root's 16px to a rem, for what a page places itself:
 * a popup beside what opened it. */
export const pixels = (token: Token<"length">): number => {
  const value = token.value as Varies<Length>
  const length = "default" in value ? value.default : value
  return "px" in length ? length.px : length.rem * 16
}
