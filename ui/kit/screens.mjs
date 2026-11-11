// React screens, each <name>.jsx or .tsx in a folder built into <name>.html: one screen's HTML with its own styles and
// module in it. React and @one/ui (`shared`) are left to the page's import map, so a screen never brings its own copy.
// The one way a screen is built: an app's (`one deploy`, src/app/deploy.rs, which runs this with the deploying machine's
// Bun or Node from a folder holding these tools), and the client's own Settings screens (build.js, here).
//
// node screens.mjs <screens folder> <out folder> <folder their classes are in> <the tag a screen opts in with> <shared name>...
import { mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import { fileURLToPath } from "node:url"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { build } from "vite"

const here = import.meta.dirname

export async function buildScreens(from, to, source, tag, shared, prefix = "") {
  const screens = readdirSync(from).filter((f) => /^[a-z0-9-]+\.[jt]sx$/.test(f))
  if (!screens.length) return
  // The screens' own Tailwind utilities, from the theme's names (src/theme.css) and the classes in `source` (an app's
  // folder, whose screens may import its other files), and none of the base the page's ui.css already gave them.
  const scratch = path.join(here, `run-${process.pid}`)
  mkdirSync(scratch, { recursive: true })
  const css = path.join(scratch, "screen.css")
  const tailwind = path.join(here, "node_modules", "tailwindcss")
  writeFileSync(css, `@layer theme, base, components, utilities;
@import ${JSON.stringify(path.join(tailwind, "theme.css"))} layer(theme) theme(reference);
@import ${JSON.stringify(path.join(tailwind, "utilities.css"))} layer(utilities) source(${JSON.stringify(path.resolve(source))});
@import ${JSON.stringify(path.join(here, "src", "theme.css"))};
`)
  try {
    mkdirSync(to, { recursive: true })
    for (const file of screens) {
      const name = file.replace(/\.[jt]sx$/, "")
      const root = path.resolve(from)
      const entry = path.join(scratch, `${name}.js`)
      writeFileSync(entry, `import ${JSON.stringify(css)};\nimport ${JSON.stringify(path.resolve(from, file))};\n`)
      const out = await build({
        configFile: false,
        root,
        publicDir: false,
        logLevel: "warn",
        plugins: [react({ compiler: true }), tailwindcss()],
        define: { "process.env.NODE_ENV": JSON.stringify("production") },
        build: {
          write: false,
          reportCompressedSize: false,
          minify: true,
          sourcemap: "hidden",
          assetsInlineLimit: () => true,
          lib: { entry, formats: ["es"], fileName: () => `${name}.js`, cssFileName: name },
          rolldownOptions: { external: shared, output: { codeSplitting: false } },
        },
      })
      const files = (Array.isArray(out) ? out : [out]).flatMap((o) => o.output)
      // Each module starts with a comment of its path from where the build runs, which differs by checkout and
      // machine; they're taken out, so the same sources build to the same file anywhere.
      const chunks = files.filter((f) => f.type === "chunk")
      const regions = new Set(chunks.flatMap((c) => c.moduleIds).map((id) => `// ${path.relative(process.cwd(), id).split(path.sep).join("/")}`))
      const [chunk] = chunks
      const lines = chunk.code.split("\n")
      const kept = lines.map((line) => !regions.has(line))
      const module = lines.filter((_, i) => kept[i]).join("\n")
      if (chunks.length !== 1) throw new Error(`${file} built into ${chunks.length} chunks: a screen is one module`)
      // The module says where it starts, as a line of its own: browsers number an inline script's lines differently, and the
      // bridge turns what's thrown into lines of the module (ui/bridge.js).
      const started = `globalThis.__oneScreen={name:${JSON.stringify(name)},at:new Error().stack};`
      // Beside the screen, what its built lines came from, which only the node reads (src/insights/source_map.rs): its files
      // from the screen's own folder, and its first line left for the one the screen starts with here.
      const mappings = chunk.map.mappings.split(";").filter((_, i) => kept[i]).join(";")
      const sources = chunk.map.sources.map((s) => {
        const whole = path.resolve(root, "dist", s)
        const pkg = whole.lastIndexOf("/node_modules/")
        return pkg >= 0 ? whole.slice(pkg + "/node_modules/".length) : path.posix.join(prefix, path.relative(path.resolve(source), whole).split(path.sep).join("/"))
      })
      writeFileSync(path.join(to, `${name}.html.map`), JSON.stringify({ sources, mappings: `;${mappings}` }) + "\n")
      const styles = files.filter((f) => f.type === "asset" && f.fileName.endsWith(".css")).map((f) => String(f.source)).join("\n")
      // `</script` and `</style` can't appear inside them, so neither ends early.
      const html = `${tag}\n<style>${styles.replace(/<\/style/gi, "<\\/style")}</style>\n<div id="root"></div>\n<script type="module">${started}\n${module.replace(/<\/script/gi, "<\\/script")}</script>\n`
      writeFileSync(path.join(to, `${name}.html`), html)
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true })
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [from, to, source, tag, ...shared] = process.argv.slice(2)
  await buildScreens(from, to, source, tag, shared)
}
