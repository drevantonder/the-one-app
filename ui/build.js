// Builds the two files a server hands to clients: dist/ui.js (every element) and dist/ui.css (the theme a page starts
// with, before ui.js has run).
import { transform } from "lightningcss"

const here = (path) => new URL(path, import.meta.url).pathname
const minified = (css) => transform({ filename: "ui.css", code: Buffer.from(css), minify: true }).code.toString()
const text = (path) => Bun.file(here(path)).text()

await Bun.write(here("dist/ui.css"), minified((await text("src/tokens.css")) + (await text("src/page.css"))))

const built = await Bun.build({
  entrypoints: [here("src/index.js")],
  outdir: here("dist"),
  naming: "ui.js",
  target: "browser",
  minify: true,
  // What ui.js is built from picks its production build by this (Lit's, without its development warnings).
  define: { "process.env.NODE_ENV": '"production"' },
  loader: { ".svg": "text" },
  plugins: [{
    // An element's look is a CSS file beside it, imported as its text.
    name: "css",
    setup: (build) => build.onLoad({ filter: /\.css$/ }, async ({ path }) => ({
      loader: "js",
      contents: `export default ${JSON.stringify(minified(await Bun.file(path).text()))}`,
    })),
  }],
})
if (!built.success) throw new AggregateError(built.logs, "ui.js didn't build")
