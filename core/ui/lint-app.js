// `bun run lint:app apps/<app>/client`, from the repo's root: type checks an app's page against the UI library
// alone, as its tsconfig.json says by extending tsconfig.app.json, with TypeScript's own checker. It fails on an element of
// the DOM's, `document`, `window` or any other Web API, a stylesheet, and a prop a native host can't draw. A server runs it
// too, as the UI library it serves holds it (build.js), on the app it deploys, where an entrypoint says its `runtime_version`
// (`OneCore.Entrypoint.lint/3`). Its errors are TypeScript's, a line each: file, line, column and message, as the folder names it.
// A raw colour or length in a static style is a warning, not an error: "make it a token" (a line each, as an error's, but
// `warning:`), which a deploy gives with its own.
import { readdirSync, readFileSync } from "node:fs"
import { extname, join, resolve } from "node:path"
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
function dom() {
  const found = []
  const root = resolve(folder)
  for (const path of readdirSync(root, { recursive: true })) {
    if (!/\.[mc]?[jt]sx?$/.test(path) || path.endsWith(".d.ts") || path.split("/").includes("node_modules")) continue
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
  if (!/\.[jt]sx?$/.test(path) || path.split("/").includes("node_modules")) continue
  try {
    styled(readFileSync(join(root, path), "utf8"), join(root, path), made)
  } catch (error) {
    console.error(error.message.replaceAll(`${root}/`, ""))
    refused = true
  }
}
for (const warning of made.warnings) console.log(warning.replaceAll(`${root}/`, ""))
process.exit(tsc.exitCode || refused || refusedDom.length ? 1 : 0)
