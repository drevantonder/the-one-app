// An app's React screens, built for `deploy` (src/deploy.rs) by the deploying machine's own Bun or Node, from a
// folder holding @one/ui's build tools (src/screen/ui/package.json): each screens/<name>.jsx or .tsx of the app becomes
// dist/<name>.html, one screen's HTML with its own styles and module in it, which app.js imports as text. React and
// @one/ui (`shared`) are left to the page's import map, so a screen never brings its own copy.
//
// node screens.mjs <app folder> <the tag a screen opts in with> <shared name>...
import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { build } from "vite"

const [app, tag, ...shared] = process.argv.slice(2)
const here = import.meta.dirname
const screens = readdirSync(path.join(app, "screens")).filter((f) => /^[a-z0-9-]+\.[jt]sx$/.test(f))

// The screen's own Tailwind utilities, from the theme's names (src/screen/ui/src/theme.css), and none of the base the
// page's ui.css already gave it.
const scratch = path.join(here, `run-${process.pid}`)
mkdirSync(scratch, { recursive: true })
const css = path.join(scratch, "screen.css")
const tailwind = path.join(here, "node_modules", "tailwindcss")
writeFileSync(css, `@layer theme, base, components, utilities;
@import ${JSON.stringify(path.join(tailwind, "theme.css"))} layer(theme) theme(reference);
@import ${JSON.stringify(path.join(tailwind, "utilities.css"))} layer(utilities) source(${JSON.stringify(path.resolve(app))});
@import ${JSON.stringify(path.join(here, "theme.css"))};
`)

try {
  mkdirSync(path.join(app, "dist"), { recursive: true })
  for (const file of screens) {
    const name = file.replace(/\.[jt]sx$/, "")
    const entry = path.join(scratch, `${name}.js`)
    writeFileSync(entry, `import ${JSON.stringify(css)};\nimport ${JSON.stringify(path.resolve(app, "screens", file))};\n`)
    const out = await build({
      configFile: false,
      root: path.resolve(app),
      publicDir: false,
      logLevel: "warn",
      plugins: [react({ compiler: true }), tailwindcss()],
      define: { "process.env.NODE_ENV": JSON.stringify("production") },
      build: {
        write: false,
        reportCompressedSize: false,
        minify: true,
        assetsInlineLimit: () => true,
        lib: { entry, formats: ["es"], fileName: () => `${name}.js`, cssFileName: name },
        rolldownOptions: { external: shared, output: { codeSplitting: false } },
      },
    })
    const files = (Array.isArray(out) ? out : [out]).flatMap((o) => o.output)
    const module = files.filter((f) => f.type === "chunk").map((f) => f.code).join("\n")
    const styles = files.filter((f) => f.type === "asset" && f.fileName.endsWith(".css")).map((f) => String(f.source)).join("\n")
    // `</script` and `</style` can't appear inside them, so neither ends early.
    const html = `${tag}\n<style>${styles.replace(/<\/style/gi, "<\\/style")}</style>\n<div id="root"></div>\n<script type="module">${module.replace(/<\/script/gi, "<\\/script")}</script>\n`
    writeFileSync(path.join(app, "dist", `${name}.html`), html)
  }
} finally {
  rmSync(scratch, { recursive: true, force: true })
}
