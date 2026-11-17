// Builds an app: `bun app.js <source> <out>`. A server runs it, as the UI library it serves holds it (build.js), when a
// plugin with an app is deployed, on the app's source as it was deployed, and serves what it writes to <out>. Each
// `.html` in <source> is a page, and each module its package.json `exports` (`"./profile": "./profile.tsx"`) is built as
// `<name>.js`, with its exports, for another plugin's page to import: a component it draws. What every app shares (Solid,
// Zag's Solid adapter and the UI library) is left out of it, and imported from the plugin `ui`, which the network serves
// once, so every page and every module it imports, whichever plugin built it, shares one Solid; the import map a server
// serves a page with names the same (`OneCore.MCP`), for what's imported by name as it runs. A page the edge serves
// itself is built here too. A build that fails says why on stderr, a problem to a line with its file and line, and
// exits with 1.
import { existsSync, readdirSync, readFileSync } from "node:fs"
import { extname, join, relative } from "node:path"
import solid from "@solidjs/vite-plugin"
import { build, parseAst } from "vite"

const [source, out] = process.argv.slice(2)

// What an app imports that the plugin `ui` serves, by where it's served (ui/build.js).
const shared = { "solid-js": "/ui/client/solid.js", "@solidjs/web": "/ui/client/web.js", "@zag-js/solid": "/ui/client/zag.js", ui: "/ui/client/ui.js" }

const pages = readdirSync(source, { recursive: true }).filter((path) => path.endsWith(".html") && !path.split("/").includes("node_modules"))
// The modules it exports, by their names: `./profile` is `profile`.
const manifest = join(source, "package.json")
const exported = Object.entries((existsSync(manifest) && JSON.parse(readFileSync(manifest, "utf8")).exports) || {}).map(([name, file]) => [name.replace(/^\.\//, ""), join(source, file)])

try {
  await build({
    configFile: false,
    root: source,
    // Its files find each other wherever the client serves the app.
    base: "./",
    logLevel: "silent",
    plugins: [
      solid(),
      // Every page has the UI library's look before any of it has run.
      { name: "ui", transformIndexHtml: () => [{ tag: "link", attrs: { rel: "stylesheet", href: "/ui/client/ui.css" }, injectTo: "head-prepend" }] },
    ],
    build: {
      outDir: out,
      emptyOutDir: true,
      sourcemap: true,
      rolldownOptions: {
        input: { ...Object.fromEntries(pages.map((page) => [page.replace(/\.html$/, ""), join(source, page)])), ...Object.fromEntries(exported) },
        external: Object.keys(shared),
        // A module another page imports keeps its name and what it exports.
        preserveEntrySignatures: "exports-only",
        output: { paths: shared, entryFileNames: (entry) => (exported.some(([name]) => name === entry.name) ? "[name].js" : "assets/[name]-[hash].js") },
      },
    },
  })
} catch (error) {
  for (const problem of error.errors ?? [error]) console.error(describe(problem))
  process.exit(1)
}

/// A problem as the compiler says it, with where: the file as the app's source names it, its line and column, and
/// the lines around it. Solid's compiler says only what's wrong, so a file it couldn't read is read again by Vite's
/// own parser, which says where.
function describe(problem) {
  let { message, loc: at = {}, frame } = problem
  const file = at.file ?? problem.id
  if (file && !at.line) {
    try {
      parseAst(readFileSync(file, "utf8"), { lang: extname(file).slice(1) }, file)
    } catch (parsed) {
      if (parsed.loc) {
        at = parsed.loc
        frame = parsed.frame
        message = parsed.message.split("\n")[1] ?? message
      }
    }
  }
  const where = file ? `${relative(source, file)}${at.line ? `:${at.line}:${at.column}` : ""}: ` : ""
  return `${where}${message.replace(`${file}: `, "").replaceAll(source + "/", "")}${frame ? `\n${frame}` : ""}`
}
