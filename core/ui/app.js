// Builds an app: `bun app.js <source> <out> [<where>]`, where `<where>` is the folder `<source>` is in the app's repo
// (`apps/home-screen/client`), which its sourcemaps name each file by, so an error says which app's file it threw in.
// A server runs it, as the UI library it serves holds it (build.js), when a
// app with an app is deployed, on the app's source as it was deployed, and serves what it writes to <out>. Each
// `.html` in <source> is a page, and each module its package.json `exports` (`"./profile": "./profile.tsx"`) is built as
// `<name>.js`, with its exports, for another app's page to import: a component it draws. What every app shares (Solid,
// the UI library) is left out of it, and imported from the app `ui`, which the network serves
// once, so every page and every module it imports, whichever app built it, shares one Solid; the import map a server
// serves a page with names the same (`OneCore.Mcp`), for what's imported by name as it runs. A page the edge serves
// itself is built here too. A build that fails says why on stderr, a problem to a line with its file and line, and
// exits with 1.
import { existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { dirname, extname, join, relative, resolve, sep } from "node:path"
import { fileURLToPath } from "node:url"
import solid from "@solidjs/vite-plugin"
import { build, normalizePath, parseAst } from "vite"
import { newMade, stylesPlugin, stylesTable } from "./tables.ts"

const [source, out, where = ""] = process.argv.slice(2)

// What an app imports that the app `ui` serves, by where it's served (ui/build.js).
const shared = { "solid-js": "/ui/client/solid.js", "solid-js/internal": "/ui/client/internal.js", "@solidjs/web": "/ui/client/web.js", ui: "/ui/client/ui.js" }

/** The app's styles (src/styles.ts), made as each module is built. */
const made = newMade()

const pages = readdirSync(source, { recursive: true }).filter((path) => path.endsWith(".html") && !path.split(sep).includes("node_modules"))
// The modules it exports, by their names: `./profile` is `profile`.
const manifest = join(source, "package.json")
const exported = Object.entries((existsSync(manifest) && JSON.parse(readFileSync(manifest, "utf8")).exports) || {}).map(([name, file]) => [name.replace(/^\.\//, ""), join(source, file)])

try {
  await build({
    configFile: false,
    root: source,
    // Its TypeScript is read as the UI library's is, whatever the app's own tsconfig.json says for its editor: an
    // app's extends one of the UI library's (tsconfig.app.json), which isn't beside it where a server builds it.
    tsconfig: fileURLToPath(new URL("tsconfig.json", import.meta.url)),
    // Its files find each other wherever the client serves the app.
    base: "./",
    logLevel: "silent",
    plugins: [stylesPlugin(made), solid()],
    build: {
      outDir: out,
      emptyOutDir: true,
      sourcemap: true,
      // Every browser and web view the app runs in preloads modules itself, so a page doesn't load and run a polyfill first.
      modulePreload: { polyfill: false },
      rolldownOptions: {
        input: { ...Object.fromEntries(pages.map((page) => [page.replace(/\.html$/, ""), join(source, page)])), ...Object.fromEntries(exported) },
        external: Object.keys(shared),
        // A module another page imports keeps its name and what it exports.
        preserveEntrySignatures: "exports-only",
        output: {
          paths: shared,
          sourcemapPathTransform: (path, map) => join(where, relative(source, resolve(dirname(map), path))),
          entryFileNames: (entry) => (exported.some(([name]) => name === entry.name) ? "[name].js" : "assets/[name]-[hash].js"),
        },
      },
    },
  })
} catch (error) {
  for (const problem of error.errors ?? [error]) console.error(describe(problem))
  process.exit(1)
}

// Its styles, beside each page and beside its modules: the style table the renderer draws with, which a host loads as
// it loads a module of the app for another's page (`Modules` in the renderer's js.rs).
for (const folder of new Set([out, ...pages.map((page) => join(out, dirname(page)))])) writeFileSync(join(folder, "styles.json"), JSON.stringify(stylesTable(made)))

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
  return `${where}${message.replace(`${file}: `, "").replaceAll(`${normalizePath(resolve(source))}/`, "")}${frame ? `\n${frame}` : ""}`
}
