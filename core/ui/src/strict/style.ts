// The strict style vocabulary: what a strict node's `class` may say, which every host draws. It's GPUI's `Styled`
// vocabulary, which is Tailwind's (GPUI's `.flex_col().gap_2().px_4()` is Tailwind's `flex-col gap-2 px-4`), named as
// Tailwind names it, and by the design tokens only (core/ui/src/tokens.css): a colour, a radius, a shadow, a font, a
// type style, a pace is a token's name, and a length is a step of Tailwind's spacing scale (`--spacing`, 0.25rem) or
// a token's (`gutter`, `gap`, `control`, `icon`, `readable`). Nothing is arbitrary: no `[…]` values and no colours of
// their own. On the DOM, Tailwind itself draws it (theme.css); a native host draws the same classes with its own
// elements, as GPUI's methods of the same names do.
//
// A class may be prefixed with variants, as Tailwind's are: `hover:bg-accent`, `dark:bg-input/30`,
// `aria-disabled:opacity-50`, `@3xl:flex-row`. `classes` is every class and `variants` every variant, which the
// UI library's build gives a host as `/ui/client/strict.json`.

/** Every `${a}-${b}`. */
const each = <const A extends string, const B extends string>(as: readonly A[], bs: readonly B[]) => as.flatMap((a) => bs.map((b) => `${a}-${b}` as `${A}-${B}`))

/** Steps of the spacing scale, each `--spacing` (0.25rem) times the number, as GPUI's and Tailwind's are, and the
 * tokens' lengths: a screen's `gutter` and `gap`, how tall what's pressed is (`control`), an icon (`icon`), and where
 * lists stop growing (`readable`). */
const whole = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "14", "16", "20", "24", "28", "32", "36", "40", "44", "48", "52", "56", "60", "64", "72", "80", "96"] as const
const steps = [...whole, "px", "0.5", "1.5", "2.5", "3.5", "gutter", "gap", "control", "icon", "readable"] as const
/** Parts of the room. */
const fractions = ["1/2", "1/3", "2/3", "1/4", "3/4", "1/5", "2/5", "3/5", "4/5", "1/6", "5/6", "full"] as const
/** A colour, by its token. `current` is the text's colour, as SVG's `currentColor`. */
const colours = [
  "background", "foreground", "card", "card-foreground", "popover", "popover-foreground", "primary", "primary-foreground", "secondary",
  "secondary-foreground", "muted", "muted-foreground", "accent", "accent-foreground", "pressed", "destructive", "warning", "border", "input",
  "ring", "overlay", "sidebar", "sidebar-foreground", "sidebar-primary", "sidebar-primary-foreground", "sidebar-accent",
  "sidebar-accent-foreground", "sidebar-border", "sidebar-ring", "white", "black", "transparent", "current",
] as const
/** How much of a colour shows, as Tailwind's `/50`: `bg-input/30`. */
const alphas = ["5", "10", "15", "20", "25", "30", "40", "50", "60", "70", "75", "80", "90", "95"] as const
const twelve = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12"] as const
const radii = ["none", "sm", "md", "lg", "xl", "icon", "full"] as const
const painted = ["bg", "text", "border", "fill", "stroke", "outline"] as const
const placed = ["inset", "inset-x", "inset-y", "top", "right", "bottom", "left", "start", "end"] as const
const margins = ["m", "mx", "my", "mt", "mr", "mb", "ml", "ms", "me"] as const

export const classes = [
  // Layout.
  "flex", "grid", "hidden", "flex-row", "flex-col", "flex-row-reverse", "flex-col-reverse", "flex-wrap", "flex-nowrap", "flex-1", "flex-auto",
  "flex-initial", "flex-none", "grow", "grow-0", "shrink", "shrink-0",
  ...each(["basis"], [...steps, ...fractions, "auto"]),
  ...each(["items"], ["start", "end", "center", "baseline", "stretch"]),
  ...each(["self"], ["auto", "start", "end", "center", "baseline", "stretch"]),
  ...each(["justify", "content"], ["start", "end", "center", "between", "around", "evenly", "stretch"]),
  // As many columns as fit, each at least so wide, filling the room: `grid-cols-fill-20`.
  ...each(["grid-cols"], twelve), ...each(["grid-cols-fill"], whole), ...each(["col-span"], [...twelve, "full"]),
  ...each(["gap", "gap-x", "gap-y", "p", "px", "py", "pt", "pr", "pb", "pl", "ps", "pe"], steps),
  ...each([...margins, ...margins.map((margin) => `-${margin}` as const)], steps), ...each(margins, ["auto"]),
  ...each(["w", "h", "size", "min-w", "min-h"], [...steps, ...fractions, "auto", "fit", "min", "max"]),
  ...each(["max-w", "max-h"], [...steps, ...fractions, "none", "fit", "min", "max"]),
  ...each(["aspect"], ["square", "video", "auto"]),
  "relative", "absolute",
  ...each([...placed, ...placed.map((place) => `-${place}` as const)], [...steps, ...fractions]), ...each(placed, ["auto"]),
  ...each(["z"], ["0", "10", "20", "30", "40", "50", "auto"]),
  ...each(["overflow", "overflow-x", "overflow-y"], ["hidden", "visible", "clip", "auto"]),
  "@container",
  // How it looks.
  ...each(painted, colours), ...each(painted, colours).flatMap((colour) => alphas.map((alpha) => `${colour}/${alpha}` as const)),
  "border", "border-0", "border-2", ...each(["border"], ["t", "r", "b", "l", "s", "e", "x", "y"]),
  ...each(["border-t", "border-r", "border-b", "border-l", "border-s", "border-e", "border-x", "border-y"], ["0", "2"]),
  "border-solid", "border-dashed", "outline", "outline-0", "outline-2", "outline-none",
  ...each(["rounded", ...each(["rounded"], ["t", "r", "b", "l", "s", "e", "tl", "tr", "br", "bl", "ss", "se", "ee", "es"])], radii),
  ...each(["shadow"], ["xs", "md", "lg", "none"]),
  ...each(["opacity"], ["0", ...alphas, "100"]),
  "fill-none", "stroke-none", ...each(["stroke"], ["0", "1", "2"]),
  ...each(["translate-x", "translate-y", "-translate-x", "-translate-y"], [...steps, ...fractions]),
  ...each(["scale", "-scale"], ["0", "50", "75", "90", "95", "100", "105", "110"]), "-scale-x-100", "-scale-y-100",
  ...each(["cursor"], ["default", "pointer", "text", "grab", "grabbing", "not-allowed"]),
  "pointer-events-none", "pointer-events-auto", "select-none", "select-text",
  // Text: a font, a type style (DESIGN.md: text is one of a few), each a size, a weight, a tracking and a line height
  // from the tokens, and a weight, a tracking and a line height of the type styles'.
  "font-sans", "font-mono",
  ...each(["text"], ["body", "label", "headline", "title", "figure", "subheadline", "footnote", "caption-1", "caption-2"]),
  ...each(["font"], ["normal", "medium", "semibold", "bold"]),
  ...each(["leading"], ["none", "tight", "normal"]),
  ...each(["tracking"], ["tighter", "tight", "normal"]),
  ...each(["text"], ["left", "center", "right", "start", "end"]),
  "truncate", ...each(["line-clamp"], ["1", "2", "3", "4", "5", "6", "none"]),
  ...each(["whitespace"], ["normal", "nowrap", "pre", "pre-wrap"]), "wrap-break-word", "break-all",
  "tabular-nums", "italic", "underline", "no-underline",
  // Motion, at the tokens' paces (DESIGN.md): quick, settle and travel.
  "transition", "transition-colors", "transition-opacity", "transition-transform", "transition-all", "transition-none",
  ...each(["duration"], ["quick", "settle", "travel", "0"]),
  ...each(["ease"], ["quick", "settle", "travel", "linear"]),
  ...each(["animate"], ["spin", "pulse", "none"]),
] as const

/** What a class may be prefixed with, as Tailwind's variants: a state of the node (`hover:`, only under a mouse,
 * `active:` while it's pressed, `focus-visible:` where the keyboard is, `disabled:` an input that is, an ARIA state's
 * `aria-…:`), where it is among its siblings (`first:`, `last:`), the colour mode (`dark:`), how the device is pointed
 * at (`pointer-coarse:`, a finger), where text runs right to left (`rtl:`), how it starts as it's added, to transition
 * from (`starting:`), a field's grey words (`placeholder:`), and how wide the nearest `@container` is (`@md:`, at
 * least 28rem, as Tailwind's container sizes are). */
export const variants = [
  "hover", "active", "focus-visible", "disabled", "first", "last", "dark", "pointer-coarse", "pointer-fine", "rtl", "starting", "placeholder",
  ...each(["aria"], ["disabled", "selected", "checked", "expanded", "pressed", "current", "invalid", "busy"]),
  ...(["3xs", "2xs", "xs", "sm", "md", "lg", "xl", "2xl", "3xl", "4xl", "5xl"] as const).map((size) => `@${size}` as const),
] as const

/** One class, without variants. */
export type Class = (typeof classes)[number]
export type Variant = (typeof variants)[number]

/** Whether one class, with its variants, is in the vocabulary. */
type Known<S extends string> = S extends `${Variant}:${infer Rest}` ? Known<Rest> : S extends Class ? true : false

/** `S`, a `class` written out, where every class in it is in the vocabulary; otherwise what isn't, as the type
 * error's words. */
export type Classes<S extends string, All extends string = S> = S extends ""
  ? All
  : S extends ` ${infer Rest}`
    ? Classes<Rest, All>
    : S extends `${infer One} ${infer Rest}`
      ? Known<One> extends true ? Classes<Rest, All> : `"${One}" isn't in the strict style vocabulary`
      : Known<S> extends true ? All : `"${S}" isn't in the strict style vocabulary`

const known = new Set<string>(classes)
const prefixes = new Set<string>(variants)

/** Of `words`, the classes in the vocabulary, with their variants: what Tailwind draws of a page's source. */
export function named(words: Iterable<string>): string[] {
  return [...new Set(words)].filter((word) => {
    const parts = word.split(":")
    return known.has(parts.pop()!) && parts.every((part) => prefixes.has(part))
  })
}
