// Builds what a server hands to clients, into dist: ui.css (the theme a page starts with, before any script has
// run), ui.js (every element, and all a page needs to draw), and Zag's two parts, which ui.js loads by itself:
// one once the page has drawn, one only on a device with a mouse (src/zag.js).
import { rmSync } from "node:fs"
import { basename } from "node:path"
import { transform } from "lightningcss"

const here = (path) => new URL(path, import.meta.url).pathname
const minified = (css) => transform({ filename: "ui.css", code: Buffer.from(css), minify: true }).code.toString()
const text = (path) => Bun.file(here(path)).text()

async function build(options) {
  const built = await Bun.build({ outdir: here("dist"), target: "browser", minify: true, ...options })
  if (!built.success) throw new AggregateError(built.logs, "The kit didn't build")
  return built.outputs
}

rmSync(here("dist"), { recursive: true, force: true })
await Bun.write(here("dist/ui.css"), minified((await text("src/tokens.css")) + (await text("src/page.css"))))

// Zag's parts, and what the two share, under names that change when they do: a client keeps them for good.
const parts = await build({
  entrypoints: [here("src/zag-late.js"), here("src/zag-mouse.js")],
  naming: { entry: "ui-[name]-[hash].js", chunk: "ui-zag-[hash].js" },
  splitting: true,
})
const named = Object.fromEntries(parts.filter((part) => part.kind === "entry-point").map((part) => [basename(part.path).slice(3, -12), basename(part.path)]))

await build({
  entrypoints: [here("src/index.js")],
  naming: "ui.js",
  // What ui.js is built from picks its production build by this (Lit's, without its development warnings).
  define: { "process.env.NODE_ENV": '"production"' },
  loader: { ".svg": "text" },
  plugins: [{
    name: "kit",
    setup(build) {
      // An element's look is a CSS file beside it, imported as its text.
      build.onLoad({ filter: /\.css$/ }, async ({ path }) => ({
        loader: "js",
        contents: `export default ${JSON.stringify(minified(await Bun.file(path).text()))}`,
      }))
      // Zag's parts stay files of their own, loaded when they're wanted.
      build.onResolve({ filter: /\/zag-(late|mouse)\.js$/ }, ({ path }) => ({ path: `./${named[basename(path, ".js")]}`, external: true }))
    },
  }],
})
