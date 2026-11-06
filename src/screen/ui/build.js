// After Vite's builds (package.json's build): the import map every page that opts in gets (src/screen.rs), the
// auth pages' small stylesheet, the gallery's page, the client's own Settings screens written in React (../settings/<name>.jsx into
// ../settings/dist/<name>.html, built as an app's screens are), and the sources' hash, which `cargo test` checks the
// built files were made from.
import { createHash } from "node:crypto"
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import path from "node:path"
import vm from "node:vm"
import tailwindcss from "@tailwindcss/vite"
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

// The auth Worker's pages (../../../cloudflare/auth/pages.js) are plain HTML, not React: this theme and the utilities
// their class names use, as one stylesheet (src/auth.css), which the Worker imports as text.
const scratch = path.join(here, `run-${process.pid}`)
mkdirSync(scratch, { recursive: true })
try {
  const entry = path.join(scratch, "auth.js")
  writeFileSync(entry, `import ${JSON.stringify(path.join(here, "src", "auth.css"))};\n`)
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

// The gallery, a page of its own beside @one/ui's files (the address's /screen/ui/gallery.html): every component and
// layout part, with a host that answers only for icons, from the address's set.
writeFileSync(dist("gallery.html"), `<!doctype html>
<html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1,viewport-fit=cover">
<title>@one/ui</title>
<link rel="stylesheet" href="ui.css"><link rel="stylesheet" href="gallery.css">
<script>window.__one = { config: {}, host: (m) => m.type === "icons" ? fetch("/icons?names=" + m.names.join(",")).then((r) => r.json()) : Promise.reject(new Error("The gallery only shows the kit.")) };</script>
<script src="../bridge.js"></script>
<script type="importmap">${map}</script>
<script src="ui.js"></script>
</head><body><div id="root"></div><script type="module" src="gallery.js"></script></body></html>
`)

// The client's own screens in React, opted in to @one/ui with src/screen.rs's tag (its UI): Settings' (../settings) and
// the pages' and Accounts' (../home, with what they share in its lib/).
const settings = path.join(here, "..", "settings")
const home = path.join(here, "..", "home")
await buildScreens(settings, path.join(settings, "dist"), settings, `<meta name="@one/ui">`, shared)
await buildScreens(home, path.join(home, "dist"), home, `<meta name="@one/ui">`, shared)

// What the built files were made from: every source file, by path and content, in order: @one/ui's, the Settings
// screens' and what they share, everything in the pages' folder but its dist/, and the auth pages.
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
// The auth pages, whose class names auth.css is made of.
sources.push("../../../cloudflare/auth/pages.js")
const hash = createHash("sha256")
for (const file of sources.sort()) hash.update(file).update("\0").update(readFileSync(path.join(here, file))).update("\0")
writeFileSync(dist("sources.txt"), hash.digest("hex") + "\n")
