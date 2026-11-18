// What the UI library's build makes of its tokens (src/tokens.ts) for each target: the web's CSS variables
// (src/tokens.css, written here, never by hand) and a native host's token table (strict/renderer.ts's `Tokens`).
import { createHash } from "node:crypto"
import { converter, parse } from "culori"
import { parseAst, type Plugin } from "vite"
import { conditions, RUNTIME_VERSION, type Condition, type Rgba, type State, type Style as Entry2, type Styles as Table, type Token as Entry, type Tokens } from "./src/strict/renderer.ts"
import { negative } from "./src/styles.ts"
import { legacy, tokens, type Token } from "./src/tokens.ts"

type Varying = Record<"default" | Condition, unknown>
const all = [...new Set([...Object.values(tokens), ...Object.values(legacy)].flatMap((group) => Object.values(group) as Token[]))]

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
      if (condition !== "default") return []
      if (value.of) return [[token.id, `color-mix(in oklab, var(--${value.of.id}) ${+(value.alpha * 100).toFixed(2)}%, transparent)`]]
      const by = varying(value)
      return [[token.id, by.dark ? `light-dark(${by.default}, ${by.dark})` : String(by.default)]]
    }
    case "material": {
      const at = varying(value)[condition] as any
      return at ? [[token.id, `blur(${css(at.blur)}) saturate(${at.saturate})`]] : []
    }
    case "length":
    case "radius": {
      const length = at(value)
      return length ? [[token.id, css(length)]] : []
    }
    case "shadow": {
      const shadows = at(value) as any[] | undefined
      return shadows ? [[token.id, shadows.map((shadow) => `${shadow.inset ? "inset " : ""}${shadow.x}px ${shadow.y}px ${shadow.blur}px ${shadow.spread}px ${shadow.colour}`).join(", ")]] : []
    }
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
      case "colour": {
        if (!value.of) return { kind: "colour", value: each(value, rgba) as never }
        // A colour at a part of another's opacity, as it is in each condition the other's varies by.
        const of = varying(value.of.value)
        return { kind: "colour", value: each(of.dark ? of : of.default, (colour: string) => { const [r, g, b, a] = rgba(colour); return [r, g, b, Math.round(a * value.alpha * 10000) / 10000] }) as never }
      }
      case "shadow":
        return { kind: "shadow", value: each(value, (shadows: any[]) => shadows.map((shadow) => ({ x: { px: shadow.x }, y: { px: shadow.y }, blur: { px: shadow.blur }, spread: { px: shadow.spread }, colour: rgba(shadow.colour), ...(shadow.inset ? { inset: true } : {}) }))) as never }
      case "type":
        return { kind: "type", value: { font: value.font.id, size: value.size.value, lineHeight: value.lineHeight, weight: value.weight.id, tracking: value.tracking, tabular: value.tabular } }
      default:
        return { kind: token.kind, value } as Entry
    }
  }
  return { v: RUNTIME_VERSION, tokens: Object.fromEntries(all.map((token) => [token.id, entry(token)])) }
}

if (import.meta.main) process.stdout.write(tokensCss())

// Styles (src/styles.ts): each `styles({ … })` in a module, worked out as it's built, into what it is on each target.


const hash = (text: string) => createHash("sha256").update(text).digest("base64url").replace(/[-_]/g, "").slice(0, 8)
const states: State[] = ["hover", "pressed", "focused", "entering"]

/** Each shorthand, as the longhands it says. */
const longhands: Record<string, string[]> = {
  padding: ["paddingTop", "paddingBottom", "paddingStart", "paddingEnd"],
  paddingInline: ["paddingStart", "paddingEnd"],
  paddingBlock: ["paddingTop", "paddingBottom"],
  margin: ["marginTop", "marginBottom", "marginStart", "marginEnd"],
  marginInline: ["marginStart", "marginEnd"],
  marginBlock: ["marginTop", "marginBottom"],
  gap: ["rowGap", "columnGap"],
  inset: ["top", "bottom", "start", "end"],
  borderWidth: ["borderTopWidth", "borderBottomWidth", "borderStartWidth", "borderEndWidth"],
  borderRadius: ["borderTopStartRadius", "borderTopEndRadius", "borderBottomStartRadius", "borderBottomEndRadius"],
}

/** A value as a token's reference, a token's negative's, or itself. */
function referenced(value: any): unknown {
  if (value && typeof value === "object" && "kind" in value && "id" in value) return { token: value.id }
  if (value && typeof value === "object" && value.negative && value.token) return { token: value.token.id, negative: true }
  return value
}

/** A value by the node's state (`{ default, hover }`), or `undefined` for one that's the same in every state. */
const byState = (value: any): Record<string, unknown> | undefined =>
  value && typeof value === "object" && !Array.isArray(value) && Object.keys(value).length && Object.keys(value).every((key) => key === "default" || states.includes(key as State)) ? value : undefined

/** A style as it's written, with its shorthands written out and its tokens as references: the table's. */
function written(style: Record<string, any>): Entry2 {
  const out: Record<string, unknown> = {}
  for (const [property, value] of Object.entries(style)) {
    const by = byState(value)
    const made = by ? Object.fromEntries(Object.entries(by).map(([state, value]) => [state, referenced(value)])) : referenced(value)
    for (const one of longhands[property] ?? [property]) out[one] = made
  }
  return out as Entry2
}

/** A token reference's CSS. */
const variable = (ref: any) => (ref.negative ? `calc(-1 * var(--${ref.token}))` : `var(--${ref.token})`)
const kebab = (name: string) => name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
/** Each property's CSS name, where it isn't its own in kebab case. */
const named: Record<string, string> = {
  paddingStart: "padding-inline-start", paddingEnd: "padding-inline-end", marginStart: "margin-inline-start", marginEnd: "margin-inline-end",
  start: "inset-inline-start", end: "inset-inline-end", borderStartWidth: "border-inline-start-width", borderEndWidth: "border-inline-end-width",
  borderTopStartRadius: "border-start-start-radius", borderTopEndRadius: "border-start-end-radius", borderBottomStartRadius: "border-end-start-radius",
  borderBottomEndRadius: "border-end-end-radius",
}

/** A property's value as CSS declarations. */
function declarations(property: string, value: any): string {
  const ref = value && typeof value === "object" && "token" in value
  const plain = ref ? variable(value) : typeof value === "number" && !["flexGrow", "flexShrink", "opacity", "zIndex", "aspectRatio", "scale", "strokeWidth", "lineClamp"].includes(property) ? `${value}px` : String(value)
  switch (property) {
    case "font": {
      const type = all.find((token) => token.id === value.token)!.value as any
      return `font-family:var(--${type.font.id});font-size:var(--${type.size.id});font-weight:var(--${type.weight.id});line-height:${type.lineHeight};letter-spacing:${type.tracking ? `${type.tracking}em` : "normal"};font-variant-numeric:${type.tabular ? "tabular-nums" : "normal"}`
    }
    case "gridTemplateColumns":
      return `grid-template-columns:${"repeat" in value ? `repeat(${value.repeat},minmax(0,1fr))` : `repeat(auto-fill,minmax(${variable(value.fill)},1fr))`}`
    case "gridColumn":
      return `grid-column:span ${value.span}/span ${value.span}`
    case "translateX":
      return `--ui-x:${plain};translate:var(--ui-x,0) var(--ui-y,0)`
    case "translateY":
      return `--ui-y:${plain};translate:var(--ui-x,0) var(--ui-y,0)`
    case "rotate":
      return `rotate:${value}deg`
    case "backdropFilter":
      return `-webkit-backdrop-filter:${plain};backdrop-filter:${plain}`
    case "lineClamp":
      return `display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:${value};overflow:hidden`
    case "userSelect":
      return `-webkit-user-select:${value};user-select:${value}`
    case "transitionProperty":
      return `transition-property:${(value as string[]).map((one) => (one === "translateX" || one === "translateY" ? "translate" : named[one] ?? kebab(one))).join(",")}`
    case "transition":
      return `transition-duration:var(--${value.token});transition-timing-function:var(--${value.token.replace(/^move-/, "ease-")})`
    case "animationName":
      return `animation-name:${value === "none" ? "none" : `ui-${value}`};animation-iteration-count:infinite`
    case "animation":
      return `animation-duration:var(--${value.token});animation-timing-function:var(--${value.token.replace(/^move-/, "ease-")})`
    case "borderTopWidth":
    case "borderBottomWidth":
    case "borderStartWidth":
    case "borderEndWidth":
      return `${named[property] ?? kebab(property)}:${plain};border-${property.match(/border(\w+?)Width/)![1].replace(/^Start$/, "inline-start").replace(/^End$/, "inline-end").toLowerCase()}-style:var(--ui-border-style,solid)`
    case "borderStyle":
      return `--ui-border-style:${value};border-style:${value}`
  }
  return `${named[property] ?? kebab(property)}:${plain}`
}

/** A class's rule, where it's in `state`. */
function rule(name: string, state: string, body: string): string {
  if (state === "hover") return `@media (hover:hover) and (pointer:fine){.${name}:hover{${body}}}`
  if (state === "pressed") return `.${name}:active{${body}}`
  if (state === "focused") return `.${name}:focus-visible{${body}}`
  if (state === "entering") return `@starting-style{.${name}{${body}}}`
  return `.${name}{${body}}`
}

/** What a module's styles made: each style's classes by property, and the CSS and table entries they need. */
export interface Made {
  css: Map<string, string>
  table: Table["styles"]
}

/** A style as the web and a host have it: its id, and its classes by property; its CSS and entry in `made`. */
function compile(style: Record<string, any>, made: Made): Record<string, string> {
  const entry = written(style)
  const id = `s${hash(JSON.stringify(entry))}`
  made.table[id] = entry
  const out: Record<string, string> = { $id: id }
  for (const [property, value] of Object.entries(entry)) {
    const by = byState(value) ?? { default: value }
    out[property] = Object.entries(by)
      .map(([state, value]) => {
        const body = declarations(property, value)
        const name = `x${hash(`${state}|${body}`)}`
        // A state's rules after those it wins over, and a type style's before what changes a part of it.
        made.css.set(`${["default", ...states].indexOf(state)}${property === "font" ? 0 : 1}${name}`, rule(name, state, body))
        return name
      })
      .join(" ")
  }
  return out
}

/** Every `styles({ … })` in `code`, worked out: the code with each as what it made. `styles` is the UI library's (from
 * "ui", or ./styles.ts in the UI library), and what it's given is literals, `tokens.…` and `negative(…)`. */
export function styled(code: string, file: string, made: Made): string | undefined {
  if (!/\bstyles\(\s*\{/.test(code)) return
  const calls: [number, number, number, number][] = []
  const visit = (node: any) => {
    if (!node || typeof node !== "object") return
    if (node.type === "CallExpression" && node.callee?.type === "Identifier" && node.callee.name === "styles" && node.arguments.length === 1) calls.push([node.start, node.end, node.arguments[0].start, node.arguments[0].end])
    for (const value of Object.values(node)) if (value && typeof value === "object") Array.isArray(value) ? value.forEach(visit) : visit(value)
  }
  visit(parseAst(code, { lang: file.endsWith(".tsx") ? "tsx" : file.endsWith(".ts") ? "ts" : "jsx" }))
  let out = code
  for (const [start, end, from, to] of calls.reverse()) {
    const written = code.slice(from, to)
    let sheet: Record<string, Record<string, unknown>>
    try {
      sheet = new Function("tokens", "negative", `"use strict"; return (${written})`)(tokens, negative)
    } catch (error) {
      throw new Error(`${file}: styles() takes what's written out, literals, tokens.… and negative(…), and this isn't: ${(error as Error).message}`)
    }
    const compiled = Object.fromEntries(Object.entries(sheet).map(([key, style]) => [key, compile(style, made)]))
    out = `${out.slice(0, start)}${JSON.stringify(compiled)}${out.slice(end)}`
  }
  return out
}

/** The CSS of what `made`, its rules in the order states win. */
export const stylesCss = (made: Made) => [...made.css.entries()].sort(([a], [b]) => a.slice(0, 2).localeCompare(b.slice(0, 2))).map(([, rule]) => rule).join("\n")

/** Vite's plugin: each module's styles worked out as it's built, into `made`. */
export const stylesPlugin = (made: Made): Plugin => ({
  name: "styles",
  enforce: "pre",
  transform(code, id) {
    if (!/\.[jt]sx?$/.test(id) || id.includes("node_modules")) return
    const out = styled(code, id, made)
    return out === undefined ? undefined : { code: out, map: null }
  },
})
