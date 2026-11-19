// What the UI library's build makes of its tokens (src/tokens.ts) for each target: the web's CSS variables
// (src/tokens.css, written here, never by hand) and a native host's token table (drawing-primitives/renderer.ts's `Tokens`).
import { createHash } from "node:crypto"
import { existsSync, readFileSync, statSync } from "node:fs"
import { dirname, join } from "node:path"
import { converter, parse } from "culori"
import { parseAst, type Plugin } from "vite"
import { conditions, RUNTIME_VERSION, type Condition, type Rgba, type State, type Style as Entry2, type Styles as Table, type Token as Entry, type Tokens } from "./src/drawing-primitives/renderer.ts"
import { negative, sprung } from "./src/styles.ts"
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
      if (value.bounce !== undefined) {
        if (condition !== "default") return []
        const { settles, easing } = sprung(value.duration, value.bounce)
        return [[token.id, `${settles}ms`], [token.id.replace(/^move-/, "ease-"), easing]]
      }
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

/** Tokens as CSS variables, on `:root`, as they are by default and where each condition holds. */
export function variablesCss(list: Token[], header: string): string {
  const rules = (condition: "default" | Condition) => list.flatMap((token) => declared(token, condition)).map(([name, value]) => `  --${name}: ${value};\n`).join("")
  return [
    header,
    `:root {\n${rules("default")}}\n`,
    ...conditions.map((condition) => (rules(condition) ? where[condition](rules(condition)) : "")),
  ].join("")
}

/** The UI library's tokens as CSS variables. */
export const tokensCss = () => variablesCss(all, "/* The design tokens as CSS variables, made from tokens.ts by the UI library's build (tables.ts). Don't edit it: edit tokens.ts. */\n")

/** A token as a native host's table has it. */
function tokenEntry(token: Token): Entry {
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

/** The tokens as a native host's table. */
export function tokensTable(): Tokens {
  return { v: RUNTIME_VERSION, tokens: Object.fromEntries(all.map((token) => [token.id, tokenEntry(token)])) }
}

if (import.meta.main) process.stdout.write(tokensCss())

// Styles (src/styles.ts) and an app's own tokens (`defineVars`, `createTheme`): each module's, worked out as it's built,
// into what they are on each target.

const hash = (text: string) => createHash("sha256").update(text).digest("base64url").replace(/[-_]/g, "").slice(0, 8)
const states: State[] = ["hover", "pressed", "focused", "entering"]
const kebab = (name: string) => name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)

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

/** Properties whose numbers aren't pixels. */
const numbers = ["flexGrow", "flexShrink", "opacity", "zIndex", "aspectRatio", "scale", "strokeWidth", "lineClamp"]
/** Properties whose values are tokens, or made of several things, which a dynamic style can't take as a value. */
const fixed = ["font", "gridTemplateColumns", "gridColumn", "boxShadow", "backdropFilter", "transitionProperty", "transition", "animationName", "animation", "lineClamp", "willChange"]

/** A value as a token's reference, a token's negative's, or itself, with those in it as references too (a grid's
 * `{ fill: tokens.grid[20] }`). */
function referenced(value: any): unknown {
  if (value && typeof value === "object" && "kind" in value && "id" in value) return { token: value.id }
  if (value && typeof value === "object" && value.negative && value.token) return { token: value.token.id, negative: true }
  if (value && typeof value === "object" && !Array.isArray(value)) return Object.fromEntries(Object.entries(value).map(([key, one]) => [key, referenced(one)]))
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
  const plain = ref ? variable(value) : typeof value === "number" && !numbers.includes(property) ? `${value}px` : String(value)
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
    case "appRegion":
      return `-webkit-app-region:${plain};app-region:${plain}`
    case "lineClamp":
      return `display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:${value};overflow:hidden`
    case "userSelect":
      return `-webkit-user-select:${value};user-select:${value}`
    case "transitionProperty":
    case "willChange":
      return `${kebab(property)}:${[...new Set((value as string[]).map((one) => (one === "translateX" || one === "translateY" ? "translate" : named[one] ?? kebab(one))))].join(",")}`
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

/** What a module's styles and tokens made: each style's classes by property, the CSS and entries they need, the app's own
 * tokens and the ones it names new values for (`defineVars`, `createTheme`) by id, and what it warns of. */
export interface Made {
  css: Map<string, string>
  table: Table["styles"]
  tokens: Map<string, Token>
  warnings: Set<string>
}

export const newMade = (): Made => ({ css: new Map(), table: {}, tokens: new Map(), warnings: new Set() })

/** An app's tokens as a native host's table has them, with the UI library's that are a part of one it names a new
 * value for (a label at 60%) as they are now. */
export function themeTokens(made: Made): Record<string, Entry> {
  const now = new Map(made.tokens)
  for (let more = true; more; ) {
    more = false
    for (const token of all) {
      const of = (token.value as { of?: Token } | undefined)?.of
      if (of && now.has(of.id) && !now.has(token.id)) {
        now.set(token.id, { ...token, value: { ...(token.value as object), of: now.get(of.id) } })
        more = true
      }
    }
  }
  return Object.fromEntries([...now].map(([id, token]) => [id, tokenEntry(token)]))
}

/** An app's style table as a native host has it: its styles, and its tokens where it has its own. */
export const stylesTable = (made: Made): Table => ({ v: RUNTIME_VERSION, styles: made.table, ...(made.tokens.size ? { tokens: themeTokens(made) } : {}) })

/** A style as the web and a host have it: its id, and its classes by property; its CSS and entry in `made`. `dynamic` is
 * the properties it takes as values, each by its parameter: a style table's entry is what's static, and the call gives the
 * rest. */
function compile(style: Record<string, any>, made: Made, dynamic: Record<string, string> = {}): Record<string, string> {
  const entry = written(style)
  const id = `s${hash(JSON.stringify([entry, Object.keys(dynamic).sort()]))}`
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
  // A spring's speed is its easing on the web, which `web()` makes from its spring as it's given.
  if (dynamic.transitionVelocity) {
    const spring = style.transition?.value
    if (spring?.bounce === undefined) throw new Error("a style's transitionVelocity is a spring's, so its transition is a spring token")
    ;(out as Record<string, unknown>).$spring = [spring.duration, spring.bounce]
  }
  // What a call gives is a CSS custom property of its own, which its class reads.
  for (const property of Object.keys(dynamic).filter((property) => property !== "transitionVelocity")) {
    const custom = `var(--${id}-${property})`
    const body = property === "rotate" ? `rotate:${custom}` : declarations(property, custom)
    const name = `x${hash(`default|${body}`)}`
    made.css.set(`01${name}`, rule(name, "default", body))
    out[property] = name
  }
  return out
}

/** What a dynamic style's function is called with as it's read, to see which of what it returns is a parameter as it is. */
class Parameter {
  constructor(readonly name: string) {}
}

/** The names a node uses, as an identifier. */
function names(node: any, into = new Set<string>()): Set<string> {
  if (!node || typeof node !== "object") return into
  if (node.type === "Identifier") into.add(node.name)
  for (const value of Object.values(node)) if (value && typeof value === "object") Array.isArray(value) ? value.forEach((one) => names(one, into)) : names(value, into)
  return into
}

const unwrapped = (node: any): any => (node?.type === "ParenthesizedExpression" ? unwrapped(node.expression) : node)
const keyOf = (property: any): string | undefined => (property.computed ? undefined : property.key?.type === "Identifier" ? property.key.name : property.key?.value)
const lang = (file: string) => (file.endsWith(".tsx") ? "tsx" : file.endsWith(".ts") ? "ts" : "jsx")
/** A place in `code`, as a compiler says it: line and column. */
function place(code: string, offset: number): string {
  const before = code.slice(0, offset).split("\n")
  return `${before.length}:${before.at(-1)!.length + 1}`
}

/** An app's token values as a token's: a colour is a hex colour, or its light and dark; a length is pixels, or by condition. */
function valued(kind: string, value: unknown, what: string): unknown {
  const check = (colour: unknown) => {
    if (typeof colour !== "string" || !parse(colour)) throw new Error(`${what} isn't a colour: ${JSON.stringify(colour)}`)
    return colour
  }
  if (kind === "colour") return each(value, check)
  return each(value, (px) => {
    if (typeof px !== "number") throw new Error(`${what} isn't a number of pixels: ${JSON.stringify(px)}`)
    return { px }
  })
}

const kindOf = (value: unknown): "colour" | "length" => {
  const first = value && typeof value === "object" && "default" in value ? value.default : value
  return typeof first === "number" ? "length" : "colour"
}

/** The tokens a module has by the names it gives them (`const brand = defineVars({ … })`), and those it imports from
 * the modules beside it, and what to write in place of its own `defineVars` and `createTheme` calls. */
function scopeOf(ast: any, code: string, file: string, made: Made, seen = new Set<string>()) {
  seen.add(file)
  const scope: Record<string, Record<string, Token>> = {}
  const edits: [number, number, string][] = []
  const evaluate = (text: string) => new Function("tokens", "negative", ...Object.keys(scope), `"use strict"; return (${text})`)(tokens, negative, ...Object.values(scope))
  for (const statement of ast.body) {
    if (statement.type === "ImportDeclaration" && /^\.\.?\//.test(statement.source.value)) {
      const base = join(dirname(file), statement.source.value)
      const target = [base, `${base}.ts`, `${base}.tsx`, `${base}.js`, join(base, "index.ts")].find((path) => existsSync(path) && statSync(path).isFile())
      if (!target || seen.has(target)) continue
      const theirs = scopeOf(parseAst(readFileSync(target, "utf8"), { lang: lang(target) }), readFileSync(target, "utf8"), target, made, seen).scope
      for (const specifier of statement.specifiers) if (specifier.type === "ImportSpecifier" && theirs[specifier.imported.name ?? specifier.imported.value]) scope[specifier.local.name] = theirs[specifier.imported.name ?? specifier.imported.value]
    }
    const declaration = statement.type === "ExportNamedDeclaration" ? statement.declaration : statement
    if (declaration?.type === "VariableDeclaration") {
      for (const one of declaration.declarations) {
        const call = unwrapped(one.init)
        if (call?.type !== "CallExpression" || call.callee?.name !== "defineVars" || one.id.type !== "Identifier") continue
        const here = `${file}:${place(code, call.start)}: defineVars()`
        try {
          const vars = evaluate(code.slice(call.arguments[0].start, call.arguments[0].end)) as Record<string, unknown>
          const made_: Record<string, Token> = {}
          for (const [key, value] of Object.entries(vars)) {
            const kind = kindOf(value)
            const token = { kind, id: `x${hash(`${file}|${one.id.name}`)}-${kebab(key)}`, value: valued(kind, value, `${here}: ${key}`) } as Token
            made.tokens.set(token.id, token)
            made_[key] = token
          }
          scope[one.id.name] = made_
          edits.push([call.start, call.end, JSON.stringify(Object.fromEntries(Object.entries(made_).map(([key, token]) => [key, { kind: token.kind, id: token.id }])))])
        } catch (error) {
          throw new Error(`${here} takes what's written out, hex colours, numbers and { default, dark }: ${(error as Error).message}`)
        }
      }
    } else if (statement.type === "ExpressionStatement" && unwrapped(statement.expression)?.type === "CallExpression" && unwrapped(statement.expression).callee?.name === "createTheme") {
      const call = unwrapped(statement.expression)
      const here = `${file}:${place(code, call.start)}: createTheme()`
      try {
        const [vars, values] = call.arguments.map((argument: any) => evaluate(code.slice(argument.start, argument.end))) as Record<string, any>[]
        for (const [key, value] of Object.entries(values)) {
          const target = vars[key]
          if (!target?.kind) throw new Error(`${key} isn't one of its tokens`)
          made.tokens.set(target.id, { ...target, value: valued(target.kind, value, `${here}: ${key}`) })
        }
      } catch (error) {
        throw new Error(`${here} takes tokens and what's written out for them: ${(error as Error).message}`)
      }
      edits.push([call.start, call.end, "undefined"])
    }
  }
  return { scope, edits }
}

/** Every `styles({ … })` in `code`, and every `defineVars({ … })` and `createTheme(…)`, worked out: the code with each as
 * what it made. They're the UI library's (from "ui", or ./styles.ts in the UI library), and what each is given is
 * literals, `tokens.…`, `negative(…)`, an app's own tokens, and for a style a function of its values. */
export function styled(code: string, file: string, made: Made): string | undefined {
  if (!/\b(styles|defineVars|createTheme)\(/.test(code)) return
  const ast = parseAst(code, { lang: lang(file) })
  const { scope, edits } = scopeOf(ast, code, file, made)
  const calls: any[] = []
  const visit = (node: any) => {
    if (!node || typeof node !== "object") return
    if (node.type === "CallExpression" && node.callee?.type === "Identifier" && node.callee.name === "styles" && node.arguments.length === 1) calls.push(node)
    for (const value of Object.values(node)) if (value && typeof value === "object") Array.isArray(value) ? value.forEach(visit) : visit(value)
  }
  visit(ast)
  const evaluate = (text: string, values: Record<string, unknown> = {}) =>
    new Function("tokens", "negative", ...Object.keys(scope), ...Object.keys(values), `"use strict"; return (${text})`)(tokens, negative, ...Object.values(scope), ...Object.values(values))
  for (const call of calls) {
    const sheet = unwrapped(call.arguments[0])
    if (sheet?.type !== "ObjectExpression") throw new Error(`${file}:${place(code, call.start)}: styles() takes an object written out`)
    const compiled: string[] = []
    for (const entry of sheet.properties) {
      const key = entry.type === "Property" ? keyOf(entry) : undefined
      const where = `${file}:${place(code, entry.start)}`
      if (key === undefined) throw new Error(`${where}: styles() takes each style by its name, written out`)
      const value = unwrapped(entry.value)
      if (value.type === "ArrowFunctionExpression") {
        const params = value.params.map((param: any) => {
          if (param.type !== "Identifier") throw new Error(`${where}: ${key}'s parameters are plain names`)
          return param.name as string
        })
        const body = unwrapped(value.body)
        if (body.type !== "ObjectExpression") throw new Error(`${where}: ${key} is a function that returns a style, written out: (height) => ({ height })`)
        for (const property of body.properties) {
          const used = names(property.value, new Set())
          const inner = unwrapped(property.value)
          const bare = inner?.type === "Identifier" && params.includes(inner.name)
          if (!bare && params.some((param) => used.has(param))) throw new Error(`${file}:${place(code, property.start)}: a dynamic style's value is its parameter, as it is (${keyOf(property)}: ${params.find((param) => used.has(param))}); work a value out where it's called`)
        }
        const shape = evaluate(code.slice(body.start, body.end), Object.fromEntries(params.map((param: string) => [param, new Parameter(param)]))) as Record<string, any>
        const statics: Record<string, any> = {}
        const dynamic: Record<string, string> = {}
        for (const [property, value] of Object.entries(shape)) {
          if (value instanceof Parameter) {
            if (fixed.includes(property)) throw new Error(`${where}: ${key}.${property} is a token's, so it isn't a value a style takes`)
            for (const one of longhands[property] ?? [property]) dynamic[one] = value.name
          } else statics[property] = value
        }
        warn(statics, body, code, file, made)
        const classes = compile(statics, made, dynamic)
        const pairs = Object.entries(dynamic).map(([property, param]) => `${JSON.stringify(property)}:${param}`)
        compiled.push(`${JSON.stringify(key)}:(${params.join(",")})=>(${JSON.stringify(classes).slice(0, -1)},"$dynamic":{${pairs.join(",")}}})`)
      } else {
        let style: Record<string, unknown>
        try {
          style = evaluate(code.slice(value.start, value.end))
        } catch (error) {
          throw new Error(`${where}: styles() takes what's written out, literals, tokens.…, negative(…), an app's own tokens and functions of values, and ${key} isn't: ${(error as Error).message}`)
        }
        warn(style, value, code, file, made)
        compiled.push(`${JSON.stringify(key)}:${JSON.stringify(compile(style, made))}`)
      }
    }
    edits.push([call.start, call.end, `{${compiled.join(",")}}`])
  }
  let out = code
  for (const [start, end, text] of edits.sort((a, b) => b[0] - a[0])) out = `${out.slice(0, start)}${text}${out.slice(end)}`
  return out
}

/** What a static style says as a raw literal, which is a warning: a token is what a style names, and what's data goes in a dynamic one. */
function warn(style: Record<string, any>, node: any, code: string, file: string, made: Made) {
  for (const [property, value] of Object.entries(style)) {
    for (const one of byState(value) ? Object.values(byState(value)!) : [value]) {
      const length = typeof one === "number" && one !== 0 && !numbers.includes(property) && property !== "rotate"
      const colour = typeof one === "string" && one.startsWith("#")
      if (!length && !colour) continue
      const at = node.properties?.find((one: any) => keyOf(one) === property)?.start ?? node.start
      made.warnings.add(`${file}(${place(code, at).replace(":", ",")}): warning: ${property}: ${JSON.stringify(one)} is a raw ${colour ? "colour" : "length"}; make it a token (defineVars for an app's own), or a dynamic style's value if it's data`)
    }
  }
}

/** The CSS of what `made`, its rules in the order states win, after its app's own tokens as CSS variables. */
export const stylesCss = (made: Made) =>
  [
    made.tokens.size ? variablesCss([...made.tokens.values()], "/* The app's own tokens (defineVars) and those it names new values for (createTheme). */\n") : "",
    ...[...made.css.entries()].sort(([a], [b]) => a.slice(0, 2).localeCompare(b.slice(0, 2))).map(([, rule]) => rule),
  ].join("\n")

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
