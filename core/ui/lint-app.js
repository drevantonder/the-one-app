// `bun run lint:app apps/<app>/client`, from the repo's root: type checks an app's page against the UI library
// alone, as its tsconfig.json says by extending tsconfig.app.json, with TypeScript's own checker. It fails on an element of
// the DOM's, `document`, `window` or any other Web API, a stylesheet, a prop a native host can't draw, and a `For` as a
// `List`'s rows, which makes them all at once (`List each` makes those near the window), and a `Row` straight in a `Section`,
// which has no card (the `List` it's in draws it), and a `Button` of `variant="destructive"`, which only an alert draws. A server runs it
// too, as the UI library it serves holds it (build.js), on the app it deploys, where an entrypoint says its `runtime_version`
// (`OneCore.Entrypoint.lint/3`). Its errors are TypeScript's, a line each: file, line, column and message, as the folder names it.
// A raw colour or length in a static style is a warning, not an error: "make it a token" (a line each, as an error's, but
// `warning:`), which a deploy gives with its own.
import { readdirSync, readFileSync } from "node:fs"
import { extname, join, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"
import { parseAst } from "vite"
import { newMade, styled } from "./tables.ts"

const folder = process.argv[2]
if (!folder) throw new Error("Which app's page? bun run lint:app apps/<app>/client")
const { extends: base } = Bun.JSONC.parse(readFileSync(join(folder, "tsconfig.json"), "utf8"))
if (!String(base).endsWith("tsconfig.app.json")) {
  console.error(`${folder} isn't checked against the UI library: its tsconfig.json extends ${base ?? "nothing"}, not core/ui/tsconfig.app.json`)
  process.exit(1)
}
// `window` and `document`, which a native host hasn't: TypeScript's checker names an unknown one, but a page can declare
// its own (`declare const window: any`), reach it by `globalThis` or `self`, or be a file the checker skips (`.js`). The UI
// library is how a page reaches its screen: `touch()`, `viewport()`, `link()` and `visible()` say what `window` would.
const DOM = new Set(["window", "document"])
/** What's said as a warning: a deploy gives it with its own. */
const noted = []
function dom() {
  const found = []
  const root = resolve(folder)
  for (const path of readdirSync(root, { recursive: true })) {
    if (!/\.[mc]?[jt]sx?$/.test(path) || path.endsWith(".d.ts") || path.split(sep).includes("node_modules")) continue
    const code = readFileSync(join(root, path), "utf8")
    let ast
    try {
      ast = parseAst(code, { lang: extname(path).slice(1).replace(/^[mc]/, "") }, path)
    } catch {
      continue // the checker and the build say why it doesn't parse
    }
    const at = (node) => {
      const before = code.slice(0, node.start).split("\n")
      return `${path}(${before.length},${before.at(-1).length + 1})`
    }
    const refuse = (node, name) => found.push(`${at(node)}: error: \`${name}\` isn't on a native host: the UI library is how a page reaches its screen (\`touch()\`, \`viewport()\`, \`link()\`, \`visible()\`)`)
    // What the file declares itself under those names (a variable, a parameter, a function), which its uses of them are.
    const own = new Set()
    const names = (pattern, into) => {
      if (!pattern) return
      if (pattern.type === "Identifier") into(pattern.name)
      else if (pattern.type === "ObjectPattern") pattern.properties.forEach((one) => names(one.value ?? one.argument, into))
      else if (pattern.type === "ArrayPattern") pattern.elements.forEach((one) => names(one, into))
      else if (pattern.type === "AssignmentPattern") names(pattern.left, into)
      else if (pattern.type === "RestElement") names(pattern.argument, into)
    }
    // `(globalThis as any)`, `globalThis!`
    const unwrapped = (node) => (node && ["TSAsExpression", "TSNonNullExpression", "TSTypeAssertion", "TSSatisfiesExpression", "ParenthesizedExpression"].includes(node.type) ? unwrapped(node.expression) : node)
    const walk = (node, visit, parent) => {
      if (!node || typeof node.type !== "string") return
      visit(node, parent)
      for (const [key, value] of Object.entries(node)) {
        if (key === "parent") continue
        if (Array.isArray(value)) value.forEach((one) => one && typeof one === "object" && walk(one, visit, node))
        else if (value && typeof value === "object") walk(value, visit, node)
      }
    }
    walk(ast, (node) => {
      if (node.type === "VariableDeclaration" && !node.declare) for (const one of node.declarations) names(one.id, (name) => own.add(name))
      if (/Function/.test(node.type)) {
        node.params?.forEach((param) => names(param, (name) => own.add(name)))
        if (node.id?.name) own.add(node.id.name)
      }
      if (node.type === "CatchClause") names(node.param, (name) => own.add(name))
      if (node.type === "ImportSpecifier" || node.type === "ImportDefaultSpecifier" || node.type === "ImportNamespaceSpecifier") own.add(node.local.name)
    })
    walk(ast, (node, parent) => {
      if (node.type === "Identifier" && DOM.has(node.name)) {
        const property = (parent?.type === "MemberExpression" && parent.property === node && !parent.computed) || (parent?.type === "Property" && parent.key === node && !parent.computed && !parent.shorthand) || (parent?.type === "MethodDefinition" && parent.key === node) || (parent?.type === "PropertyDefinition" && parent.key === node) || parent?.type?.startsWith("TS") || ["ImportSpecifier", "ExportSpecifier"].includes(parent?.type)
        if (!property && !own.has(node.name)) refuse(node, node.name)
      }
      // `declare const document: Document`, `declare global { var window: … }`
      if (node.type === "VariableDeclaration" && node.declare) for (const one of node.declarations) names(one.id, (name) => DOM.has(name) && refuse(one, name))
      // `<List><For each={…}>…</For></List>`: `For` makes every row at once, which a long list on a native host has no
      // memory for (its sandbox's), where `<List each={…}>` makes only those near the window (`VirtualizedList`). A `For`
      // beside a list's own rows, as a few fields and a row about them are, is said as a warning.
      const named = (element) => element.openingElement?.name?.name
      if (node.type === "JSXElement" && named(node) === "List") {
        const rows = node.children.filter((one) => !(one.type === "JSXText" && !one.value.trim()))
        for (const child of rows.filter((one) => one.type === "JSXElement" && named(one) === "For")) {
          if (rows.length === 1) found.push(`${at(child)}: error: a \`For\` as a \`List\`'s rows makes every row at once, which a long list can't start with on a native host: write \`<List each={…} keyed={…}>{(item) => <Row … />}</List>\`, which makes only the rows near the window`)
          else noted.push(`${at(child)}: warning: a \`For\` beside other rows in a \`List\` makes every row at once: fine for a few, but a long list's rows go in a \`List each={…}\` of their own`)
        }
      }
      // `<Section><Row /></Section>`: a `Row` is drawn in its card by the `List` it's in, where one outside a list has no
      // card, as its screen's background, and a line above it as the first row of a list has none.
      if (node.type === "JSXElement" && named(node) === "Section") {
        for (const child of node.children.filter((one) => one.type === "JSXElement" && named(one) === "Row")) {
          found.push(`${at(child)}: error: a \`Row\` in a \`Section\` has no card, which only the \`List\` it's in draws: write \`<List><Row … /></List>\``)
        }
      }
      // `<Button variant="destructive">`: a destructive action on a page is a red `Row` in a `List`, as iOS's Settings has
      // "Sign Out" and "Delete Contact"; a red button is an alert's, which `confirm({ destructive: true })` draws.
      if (node.type === "JSXElement" && named(node) === "Button") {
        const variant = node.openingElement.attributes.find((one) => one.type === "JSXAttribute" && one.name.name === "variant")
        if (variant?.value?.value === "destructive") found.push(`${at(node)}: error: a red button isn't a page's: a destructive action is a red \`Row\` in a \`List\` (\`<List><Row heading="Delete…" destructive onPress={…} /></List>\`), and \`confirm({ destructive: true })\` draws the alert's`)
      }
      // `<Row end={<Button>Update</Button>} />`: `end` is for an icon (a button of `size="icon"` too), an amount or a switch; a button with words goes in
      // `action`, which a phone puts under the row's words, where in `end` a longer heading would be squeezed by it.
      if (node.type === "JSXElement" && named(node) === "Row") {
        const end = node.openingElement.attributes.find((one) => one.type === "JSXAttribute" && one.name.name === "end")
        walk(end?.value, (inner) => {
          const icon = inner.openingElement?.attributes.some((one) => one.type === "JSXAttribute" && one.name.name === "size" && one.value?.value === "icon")
          if (inner.type === "JSXElement" && named(inner) === "Button" && !icon) found.push(`${at(inner)}: error: a \`Button\` in a \`Row\`'s \`end\` squeezes its words: a button goes in \`action\` (\`<Row heading="…" action={<Button>…</Button>} />\`), which a phone puts under them; \`end\` is for an icon, an amount or a switch`)
        })
      }
      // `globalThis.document`, `self["window"]`
      const object = unwrapped(node.object)
      if (node.type === "MemberExpression" && object?.type === "Identifier" && ["globalThis", "self"].includes(object.name)) {
        const name = node.computed ? node.property.value : node.property.name
        if (DOM.has(name)) refuse(node, name)
      }
    })
  }
  return found
}
const refusedDom = [...new Set(dom())]
for (const line of refusedDom) console.error(line)

const tsc = Bun.spawnSync([process.execPath, fileURLToPath(new URL("node_modules/typescript/bin/tsc", import.meta.url)), "-p", ".", "--pretty", "false"], { cwd: folder, stdout: "inherit", stderr: "inherit" })

// What its styles and tokens (`styles()`, `defineVars()`, `createTheme()`) can't be made of, as the build would refuse it,
// and what they warn of.
const root = resolve(folder)
const made = newMade()
let refused = false
for (const path of readdirSync(root, { recursive: true })) {
  if (!/\.[jt]sx?$/.test(path) || path.split(sep).includes("node_modules")) continue
  try {
    styled(readFileSync(join(root, path), "utf8"), join(root, path), made)
  } catch (error) {
    console.error(error.message.replaceAll(`${root}${sep}`, ""))
    refused = true
  }
}
for (const warning of made.warnings) console.log(warning.replaceAll(`${root}${sep}`, ""))
for (const warning of noted) console.log(warning)
process.exit(tsc.exitCode || refused || refusedDom.length ? 1 : 0)
