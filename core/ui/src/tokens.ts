// The design tokens: every colour, length, radius, shadow, font, weight, type style and pace the UI library and every
// app draw with, as typed values, each the same everywhere or varying by condition (dark, touch, phone, reduced
// motion: strict/protocol.ts's `Condition`). This is their one source. The UI library's build makes from it the web's
// CSS variables (tokens.css, which isn't edited by hand) and a native host's token table (/ui/client/tokens.json), and
// a style names a token here by reference (`tokens.colour.mutedForeground`), never by its value.
//
// Colours are shadcn's neutral, light and dark. The typeface is Allo (allo.joshpuckett.me, github.com/joshpuckett/
// allo v1.0, SIL Open Font License with no Reserved Font Name: Allo-LICENSE.txt), served beside the UI library's
// stylesheet so nothing comes from a font host; code is in the device's own monospace, and a script Allo lacks in the
// device's own font. Allo-Variable.woff2 is its AlloVariable.ttf with roundness pinned at 100 and only Latin kept:
//   fonttools varLib.instancer AlloVariable.ttf ROND=100 -o allo.ttf
//   pyftsubset allo.ttf --layout-features='*' --flavor=woff2 --output-file=Allo-Variable.woff2 --unicodes=U+0000-00FF,
//     U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,U+2000-206F,U+20AC,U+2122,U+2191,
//     U+2193,U+2212,U+2215,U+FEFF,U+FFFD
import type { Condition, Length, Varies } from "./strict/protocol.ts"

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

/** A shadow: its offsets, blur and spread in pixels, and its colour. */
type Shadow = { x: number; y: number; blur: number; spread: number; colour: string }
const shadow = (...shadows: Shadow[]): Token<"shadow"> => ({ kind: "shadow", id: "", value: shadows })
const font = (...families: string[]): Token<"font"> => ({ kind: "font", id: "", value: families })
/** A weight, a little lighter in dark: light text on dark looks heavier. */
const weight = (weight: number): Token<"weight"> => ({ kind: "weight", id: "", value: { default: weight, dark: weight - 25 } })
const motion = (duration: Varies<number>, easing: Varies<[number, number, number, number]>): Token<"motion"> => ({ kind: "motion", id: "", value: { duration, easing } })

const colours = {
  background: colour("oklch(1 0 0)", "oklch(0.145 0 0)"),
  foreground: colour("oklch(0.145 0 0)", "oklch(0.985 0 0)"),
  /** What's raised off the page: a list, a menu, a sheet. */
  card: colour("oklch(1 0 0)", "oklch(0.205 0 0)"),
  primary: colour("oklch(0.205 0 0)", "oklch(0.922 0 0)"),
  primaryForeground: colour("oklch(0.985 0 0)", "oklch(0.205 0 0)"),
  /** A quiet fill: a secondary button, what's hovered, a skeleton. */
  muted: colour("oklch(0.97 0 0)", "oklch(0.269 0 0)"),
  /** Grey words, and where the keyboard is. */
  mutedForeground: colour("oklch(0.54 0 0)", "oklch(0.708 0 0)"),
  destructive: colour("oklch(0.565 0.245 27.325)", "oklch(0.704 0.191 22.216)"),
  /** Warn: "this needs you", but it can wait. */
  warning: colour("oklch(0.555 0.163 48.998)", "oklch(0.828 0.189 84.429)"),
  /** A hairline, a field's edge, and what a pressed row fills with: iOS's grey, a step darker than a hover. */
  border: colour("oklch(0.922 0 0)", "oklch(1 0 0 / 10%)"),
  /** What dims the page under a sheet. */
  overlay: colour("oklch(0 0 0 / 50%)"),
  /** A sidebar beside what it opens, a step off the page. */
  sidebar: colour("oklch(0.985 0 0)", "oklch(0.205 0 0)"),
  /** Code, coloured as GitHub colours it, and a diff's tints. */
  hlKw: colour("#cf222e", "#ff7b72"),
  hlStr: colour("#0a3069", "#a5d6ff"),
  hlCom: colour("#6e7781", "#8b949e"),
  hlNum: colour("#0550ae", "#79c0ff"),
  hlFn: colour("#8250df", "#d2a8ff"),
  hlType: colour("#953800", "#ffa657"),
  hlTag: colour("#116329", "#7ee787"),
  hlDel: colour("#82071e", "#ffa198"),
  find: colour("#fae17d", "#9e6a03"),
  diffAdd: colour("#e6ffec", "rgb(46 160 67 / 0.15)"),
  diffDel: colour("#ffebe9", "rgb(248 81 73 / 0.15)"),
}

/** Lengths. A computer's are rems of 16px; under a finger what's pressed is a 44px target and text a step larger, which
 * a field never shows under 16px, where an iPhone would zoom into it; an iPhone's and iPad's rem is the text size its
 * person chose (Dynamic Type), 17px unless they changed it, and its type styles are iOS's own at that size. */
const lengths = {
  /** Body text and what's typed. */
  textBase: length({ default: rem(0.875), touch: rem(1) }),
  textLg: length({ default: rem(1.0625), phone: rem(1) }),
  textTitle: length({ default: rem(1.75), phone: rem(28 / 17) }),
  /** Apple's smaller text styles, the same under a finger and a mouse: subheadline, for a section's toggle;
   * footnote, for a count, code or a hint; caption 1, for an icon's name; caption 2, the smallest, for a key or a
   * small heading. */
  textSubheadline: length(rem(0.9375)),
  textFootnote: length(rem(0.8125)),
  textCaption1: length(rem(0.75)),
  textCaption2: length(rem(0.6875)),
  /** What's typed in a field: body, but never under 16px under a finger. */
  textField: length({ default: rem(0.875), touch: rem(1) }),
  /** How tall what's pressed or typed in is, and an icon in it. */
  control: length({ default: rem(2.25), touch: rem(2.75), phone: rem(44 / 17) }),
  icon: length({ default: rem(1), touch: rem(1.125), phone: rem(19 / 17) }),
  /** A screen's gutter and the gap between its parts, which on an iPhone stay put as its text grows; and where lists
   * and forms stop growing. */
  gutter: length({ default: rem(1), phone: px(16) }),
  readable: length(rem(42)),
}

/** Space between and around things, a scale of steps from 4px. */
const space = {
  xs: length(rem(0.25)),
  sm: length(rem(0.5)),
  md: length(rem(0.75)),
  lg: length(rem(1)),
  xl: length(rem(1.5)),
  xxl: length(rem(2)),
  xxxl: length(rem(3)),
}

const radii = {
  sm: radius(rem(0.375)),
  md: radius(rem(0.5)),
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
  body: weight(450),
  label: weight(550),
  title: weight(650),
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
  figure: type(lengths.textTitle, weights.title, { lineHeight: 1.2, tracking: -0.025, tabular: true }),
  subheadline: type(lengths.textSubheadline, weights.body),
  footnote: type(lengths.textFootnote, weights.body),
  caption1: type(lengths.textCaption1, weights.body),
  caption2: type(lengths.textCaption2, weights.body),
}

/** How things move. Quick: what goes, what's small (a press, a fade, a menu). Settle: what arrives or shifts in
 * place. Travel: what crosses the screen (a sheet), which for whoever asked for less motion fades instead, as briefly
 * as what settles. */
const out: [number, number, number, number] = [0.22, 1, 0.36, 1]
const motions = {
  quick: motion(120, out),
  settle: motion(200, out),
  travel: motion({ default: 300, reducedMotion: 200 }, { default: [0.32, 0.72, 0, 1], reducedMotion: out }),
}

/** Each token's id, as its kind's group names it: a colour by itself (`muted-foreground`), a length as it's known
 * (`text-base`, `control`), and the rest by their group (`radius-md`, `shadow-xs`, `font-sans`, `label-weight`,
 * `type-label`, `move-quick`). */
function named<T extends Record<string, Token>>(group: T, id: (key: string) => string): T {
  for (const [key, token] of Object.entries(group)) token.id = id(key.replace(/[A-Z0-9]/g, (letter) => `-${letter.toLowerCase()}`).replace(/^x(x+)l$/, (_, more) => `${more.length + 1}xl`))
  return group
}

/** Every token, by kind. */
export const tokens = {
  colour: named(colours, (key) => key),
  length: named(lengths, (key) => key),
  space: named(space, (key) => `space-${key}`),
  radius: named(radii, (key) => `radius-${key}`),
  shadow: named(shadows, (key) => `shadow-${key}`),
  font: named(fonts, (key) => `font-${key}`),
  weight: named(weights, (key) => `${key}-weight`),
  type: named(types, (key) => `type-${key}`),
  motion: named(motions, (key) => `move-${key}`),
}
