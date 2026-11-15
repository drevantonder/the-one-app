// Builds an app: `bun app.js <source> <out>`. A server runs it, as the kit it serves holds it (build.js), when a
// plugin with an app is deployed, on the app's source as it was deployed, and serves what it writes to <out>. Each
// `.html` in <source> is a page. What every app shares (Solid, Zag's Solid adapter and the kit) is left out of it,
// and imported from the plugin `ui`, which the network serves once. A build that fails says why on stderr, a
// problem to a line with its file and line, and exits with 1.
import { readdirSync, readFileSync } from "node:fs"
import { extname, join, relative } from "node:path"
import solid from "@solidjs/vite-plugin"
import { build, parseAst } from "vite"

const [source, out] = process.argv.slice(2)

// What an app imports that the plugin `ui` serves, by where it's served (ui/build.js).
const shared = { "solid-js": "/ui/client/solid.js", "@solidjs/web": "/ui/client/web.js", "@zag-js/solid": "/ui/client/zag.js", ui: "/ui/client/ui.js" }

const pages = readdirSync(source, { recursive: true }).filter((path) => path.endsWith(".html") && !path.split("/").includes("node_modules"))

try {
  await build({
    configFile: false,
    root: source,
    // Its files find each other wherever the client serves the app.
    base: "./",
    logLevel: "silent",
    plugins: [
      solid(),
      // Every page has the kit's look before any of it has run.
      { name: "kit", transformIndexHtml: () => [{ tag: "link", attrs: { rel: "stylesheet", href: "/ui/client/ui.css" }, injectTo: "head-prepend" }] },
    ],
    build: {
      outDir: out,
      emptyOutDir: true,
      sourcemap: true,
      rolldownOptions: {
        input: pages.map((page) => join(source, page)),
        external: Object.keys(shared),
        output: { paths: shared },
      },
    },
  })
} catch (error) {
  for (const problem of error.errors ?? [error]) console.error(said(problem))
  process.exit(1)
}

/// A problem as the compiler says it, with where: the file as the app's source names it, its line and column, and
/// the lines around it. Solid's compiler says only what's wrong, so a file it couldn't read is read again by Vite's
/// own parser, which says where.
function said(problem) {
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
