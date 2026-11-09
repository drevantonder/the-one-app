// After Vite's builds (package.json's build): the import map every page that opts in gets (src/screen.rs), the
// auth pages, drawn, and their small stylesheet, the client's own Settings screens written in React (../settings/<name>.jsx into
// ../settings/dist/<name>.html, built as an app's screens are), and the sources' hash, which `cargo test` checks the
// built files were made from.
import { createHash } from "node:crypto"
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import vm from "node:vm"
import tailwindcss from "@tailwindcss/vite"
import react from "@vitejs/plugin-react"
import { build } from "vite"
import { buildScreens } from "./screens.mjs"
import { shared } from "./vite.config.ts"

const here = import.meta.dirname
const dist = (name) => path.join(here, "dist", name)

// Each name a screen imports is a small module giving what ui.js keeps on OneUI under that name. They're data: URLs,
// the same whether the page has the kit in it or links it, so a page needs no other file for them.
const box = { document: { addEventListener() {} } }
box.globalThis = box
vm.runInNewContext(readFileSync(dist("ui.js"), "utf8"), box)
const imports = {}
for (const name of shared) {
  const module = box.OneUI[name]
  if (!module) throw new Error(`ui.js keeps nothing as ${name}`)
  const names = Object.keys(module).filter((n) => n !== "default" && /^[A-Za-z_$][\w$]*$/.test(n))
  const code = `const m=globalThis.OneUI[${JSON.stringify(name)}];export default m.default??m;export const{${names.join(",")}}=m;`
  imports[name] = `data:text/javascript,${encodeURIComponent(code)}`
}
const map = JSON.stringify({ imports })
writeFileSync(dist("importmap.json"), map + "\n")

// The auth Worker's pages (../../onboarding/pages.tsx): each drawn from @one/ui's components to HTML here, as
// dist/auth.json, so no React reaches a browser before it's signed in; and this theme with the utilities their class
// names use, as one stylesheet (src/auth.css, written as dist/auth.css). The Worker imports both.
const scratch = path.join(here, `run-${process.pid}`)
mkdirSync(scratch, { recursive: true })
try {
  const draw = path.join(scratch, "draw.jsx")
  writeFileSync(draw, `import { renderToStaticMarkup } from "react-dom/server"
import { pages, parts } from ${JSON.stringify(path.join(here, "..", "..", "onboarding", "pages.tsx"))}
// React puts a preload of each image ahead of what it draws, for a page it streams; these are whole, the image in them.
const draw = (all) => Object.fromEntries(Object.entries(all).map(([name, page]) => [name, renderToStaticMarkup(page).replace(/<link rel="preload" as="image"[^>]*>/g, "")]))
export default { pages: draw(pages), parts: draw(parts) }
`)
  await build({
    configFile: false,
    root: here,
    publicDir: false,
    logLevel: "warn",
    plugins: [react()],
    // pages.tsx and Welcome (../../onboarding) are outside this folder: React is this folder's.
    resolve: { alias: { "@": path.join(here, "src") }, dedupe: ["react", "react-dom"] },
    define: { "process.env.NODE_ENV": JSON.stringify("production") },
    ssr: { noExternal: true },
    build: { ssr: draw, outDir: scratch, emptyOutDir: false, reportCompressedSize: false, rolldownOptions: { output: { entryFileNames: "drawn.mjs" } } },
  })
  const drawn = (await import(path.join(scratch, "drawn.mjs"))).default
  writeFileSync(dist("auth.json"), JSON.stringify(drawn, null, 1) + "\n")
  // Their class names, as the browser reads them, for Tailwind to find.
  const unescape = (s) => s.replace(/&(amp|quot|#x27|lt|gt);/g, (_, e) => ({ amp: "&", quot: '"', "#x27": "'", lt: "<", gt: ">" })[e])
  const html = Object.values(drawn).flatMap(Object.values).join("")
  writeFileSync(path.join(scratch, "classes.txt"), [...html.matchAll(/class="([^"]*)"/g)].map((m) => unescape(m[1])).join("\n"))
  writeFileSync(path.join(scratch, "auth.css"), `@import ${JSON.stringify(path.join(here, "src", "auth.css"))};\n@source "./classes.txt";\n`)
  const entry = path.join(scratch, "auth.js")
  writeFileSync(entry, `import ${JSON.stringify(path.join(scratch, "auth.css"))};\n`)
  const out = await build({
    configFile: false,
    root: here,
    publicDir: false,
    logLevel: "warn",
    plugins: [tailwindcss()],
    build: { write: false, reportCompressedSize: false, lib: { entry, formats: ["es"], fileName: () => "auth.js", cssFileName: "auth" } },
  })
  const css = (Array.isArray(out) ? out : [out]).flatMap((o) => o.output).filter((f) => f.type === "asset" && f.fileName.endsWith(".css")).map((f) => String(f.source)).join("\n")
  writeFileSync(dist("auth.css"), css)
} finally {
  rmSync(scratch, { recursive: true, force: true })
}

// The client's own screens in React, opted in to @one/ui with src/screen.rs's tag (its UI): Settings' (../settings) and
// the pages' and Accounts' (../home, with what they share in its lib/).
const settings = path.join(here, "..", "settings")
const home = path.join(here, "..", "home")
await buildScreens(settings, path.join(settings, "dist"), settings, `<meta name="@one/ui">`, shared)
await buildScreens(home, path.join(home, "dist"), home, `<meta name="@one/ui">`, shared)

// What the built files were made from: every source file, by path and content, in order: @one/ui's, the Settings
// screens' and what they share, everything in the pages' folder but its dist/, and onboarding's screens.
const sources = readdirSync(settings).filter((f) => /\.([jt]sx|js)$/.test(f)).map((f) => `../settings/${f}`)
const walk = (dir, root = here, prefix = "") => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".") || ["node_modules", "dist"].includes(entry.name)) continue
    const full = path.join(dir, entry.name)
    entry.isDirectory() ? walk(full, root, prefix) : sources.push(prefix + path.relative(root, full).split(path.sep).join("/"))
  }
}
walk(here)
walk(home, home, "../home/")
// Onboarding's screens, which auth.json is drawn from and Welcome is @one/ui's.
const onboarding = path.join(here, "..", "..", "onboarding")
sources.push(...readdirSync(onboarding).filter((f) => f.endsWith(".tsx")).map((f) => `../../onboarding/${f}`))
const hash = createHash("sha256")
for (const file of sources.sort()) hash.update(file).update("\0").update(readFileSync(path.join(here, file))).update("\0")
writeFileSync(dist("sources.txt"), hash.digest("hex") + "\n")
