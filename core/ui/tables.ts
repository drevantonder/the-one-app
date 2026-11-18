// What the UI library's build makes of its tokens (src/tokens.ts) for each target: the web's CSS variables
// (src/tokens.css, written here, never by hand) and a native host's token table (strict/protocol.ts's `Tokens`).
import { converter, formatRgb, parse } from "culori"
import { conditions, VERSION, type Condition, type Rgba, type Token as Entry, type Tokens } from "./src/strict/protocol.ts"
import { tokens, type Token } from "./src/tokens.ts"

type Varying = Record<"default" | Condition, unknown>
const all = Object.values(tokens).flatMap((group) => Object.values(group) as Token[])

/** A value by condition, as `{ default, dark, … }`, whether or not it varies. */
const varying = (value: unknown): Partial<Varying> => (value && typeof value === "object" && "default" in value ? (value as Varying) : { default: value })
/** Each value of a varying one, made into another. */
const each = (value: unknown, change: (value: any) => unknown) => {
  const by = varying(value)
  const made = Object.fromEntries(Object.entries(by).map(([condition, value]) => [condition, change(value)]))
  return Object.keys(made).length === 1 ? made.default : made
}

const rgb = converter("rgb")
function rgba(colour: string): Rgba {
  const { r, g, b, alpha = 1 } = rgb(parse(colour)!)
  const round = (value: number) => Math.round(Math.min(1, Math.max(0, value)) * 10000) / 10000
  return [round(r), round(g), round(b), round(alpha)]
}

const css = (length: any) => ("rem" in length ? `${+length.rem.toFixed(4)}rem` : "px" in length ? `${length.px}px` : `${length.percent}%`)

/** A token's CSS variables, each by name, as they are where `condition` holds (or by default). */
function declared(token: Token, condition: "default" | Condition): [string, string][] {
  const at = (value: unknown) => varying(value)[condition]
  const value = token.value as any
  switch (token.kind) {
    case "colour": {
      const by = varying(value)
      if (condition === "default") return [[token.id, by.dark ? `light-dark(${by.default}, ${by.dark})` : String(by.default)]]
      return []
    }
    case "length":
    case "radius": {
      const length = at(value)
      return length ? [[token.id, css(length)]] : []
    }
    case "shadow":
      return condition === "default" ? [[token.id, value.map((shadow: any) => `${shadow.x}px ${shadow.y}px ${shadow.blur}px ${shadow.spread}px ${shadow.colour}`).join(", ")]] : []
    case "font":
      return condition === "default" ? [[token.id, value.map((family: string) => (/^[\w-]+$/.test(family) ? family : `"${family}"`)).join(", ")]] : []
    case "weight": {
      const weight = at(value)
      return weight === undefined ? [] : [[token.id, String(weight)]]
    }
    case "type":
      return condition === "default" && value.tracking ? [[`${token.id.replace(/^type-/, "")}-tracking`, `${value.tracking}em`]] : []
    case "motion": {
      const duration = at(value.duration)
      const easing = at(value.easing) as number[] | undefined
      return [
        ...(duration === undefined ? [] : [[token.id, `${duration}ms`] as [string, string]]),
        ...(easing === undefined ? [] : [[token.id.replace(/^move-/, "ease-"), `cubic-bezier(${easing.join(", ")})`] as [string, string]]),
      ]
    }
  }
  throw new Error(`No CSS for a ${token.kind}`)
}

const indent = (rules: string) => rules.replace(/^(?=.)/gm, "  ")

/** Where each condition holds, as CSS says it, in the order they win. A colour's dark is its `light-dark()`, which
 * follows `color-scheme` wherever it's used: the page's, which a client sets from Colour Mode, or a part's own. */
const where: Record<Condition, (rules: string) => string> = {
  dark: (rules) => `:root.dark {\n${rules}}\n@media (prefers-color-scheme: dark) {\n  :root:not(.light) {\n${indent(rules)}  }\n}\n`,
  touch: (rules) => `@media (pointer: coarse) {\n  :root {\n${indent(rules)}  }\n}\n`,
  // An iPhone's or iPad's rem is its body text size, which its person chose (`-webkit-touch-callout` is known only to iOS).
  phone: (rules) => `@supports (-webkit-touch-callout: none) and (font: -apple-system-body) {\n  :root {\n    font: -apple-system-body;\n${indent(rules)}  }\n}\n`,
  reducedMotion: (rules) => `@media (prefers-reduced-motion: reduce) {\n  :root {\n${indent(rules)}  }\n}\n`,
}

/** The tokens as CSS variables. */
export function tokensCss(): string {
  const rules = (condition: "default" | Condition) => all.flatMap((token) => declared(token, condition)).map(([name, value]) => `  --${name}: ${value};\n`).join("")
  return [
    "/* The design tokens as CSS variables, made from tokens.ts by the UI library's build (tables.ts). Don't edit it: edit tokens.ts. */\n",
    `:root {\n${rules("default")}}\n`,
    ...conditions.map((condition) => (rules(condition) ? where[condition](rules(condition)) : "")),
  ].join("")
}

/** The tokens as a native host's table. */
export function tokensTable(): Tokens {
  const entry = (token: Token): Entry => {
    const value = token.value as any
    switch (token.kind) {
      case "colour":
        return { kind: "colour", value: each(value, rgba) as never }
      case "shadow":
        return { kind: "shadow", value: value.map((shadow: any) => ({ x: { px: shadow.x }, y: { px: shadow.y }, blur: { px: shadow.blur }, spread: { px: shadow.spread }, colour: rgba(shadow.colour) })) }
      case "type":
        return { kind: "type", value: { font: value.font.id, size: value.size.value, lineHeight: value.lineHeight, weight: value.weight.id, tracking: value.tracking, tabular: value.tabular } }
      default:
        return { kind: token.kind, value } as Entry
    }
  }
  return { v: VERSION, tokens: Object.fromEntries(all.map((token) => [token.id, entry(token)])) }
}

if (import.meta.main) process.stdout.write(tokensCss())
