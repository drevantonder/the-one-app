// Builds an app: `bun app.js <source> <out>`. A server runs it when a plugin with an app is deployed, on the app's
// source as it was deployed, and serves what it writes to <out>. Each `.html` in <source> is a page. What every app
// shares (Solid, and the kit) is left out of it, and imported from the plugin `ui`, which the network serves once.
// A build that fails says why on stderr, a problem to a line with its file and line, and exits with 1.
import { readdirSync } from "node:fs"
import { join, relative } from "node:path"
import solid from "@solidjs/vite-plugin"
import { build } from "vite"

const [source, out] = process.argv.slice(2)

// What an app imports that the plugin `ui` serves, by where it's served (ui/build.js).
const shared = { "solid-js": "/ui/solid.js", "@solidjs/web": "/ui/web.js", ui: "/ui/ui.js" }

const pages = readdirSync(source, { recursive: true }).filter((path) => path.endsWith(".html") && !path.split("/").includes("node_modules"))

try {
  if (!pages.length) throw new Error("An app has a page: an .html file, such as index.html, whose script is the app's.")
  await build({
    configFile: false,
    root: source,
    // Its files find each other wherever the client serves the app.
    base: "./",
    logLevel: "silent",
    plugins: [
      solid(),
      // Every page has the kit's look before any of it has run.
      { name: "kit", transformIndexHtml: () => [{ tag: "link", attrs: { rel: "stylesheet", href: "/ui/ui.css" }, injectTo: "head-prepend" }] },
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
  // The compiler's own words, with where: the file as the app's source names it, its line and column.
  const at = error.loc ?? {}
  const file = at.file ?? error.id
  const where = file ? `${relative(source, file)}${at.line ? `:${at.line}:${at.column}` : ""}: ` : ""
  console.error(`${where}${error.message.replace(`${file}: `, "").replaceAll(source + "/", "")}${error.frame ? `\n${error.frame}` : ""}`)
  process.exit(1)
}
