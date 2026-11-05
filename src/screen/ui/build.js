// After Vite's builds (package.json's build): the import map every page that opts in gets (src/screen.rs), the
// gallery's page, the client's own Settings screens written in React (../settings/<name>.jsx into
// ../settings/dist/<name>.html, built as an app's screens are), and the sources' hash, which `cargo test` checks the
// built files were made from.
import { createHash } from "node:crypto"
import { readdirSync, readFileSync, writeFileSync } from "node:fs"
import path from "node:path"
import vm from "node:vm"
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

// The client's own Settings screens in React, opted in to @one/ui with src/screen.rs's tag (its UI).
const settings = path.join(here, "..", "settings")
await buildScreens(settings, path.join(settings, "dist"), settings, `<meta name="@one/ui">`, shared)

// What the built files were made from: every source file, by path and content, in order, and the Settings screens'.
const sources = readdirSync(settings).filter((f) => /\.[jt]sx$/.test(f)).map((f) => `../settings/${f}`)
const walk = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".") || ["node_modules", "dist"].includes(entry.name)) continue
    const full = path.join(dir, entry.name)
    entry.isDirectory() ? walk(full) : sources.push(path.relative(here, full).split(path.sep).join("/"))
  }
}
walk(here)
const hash = createHash("sha256")
for (const file of sources.sort()) hash.update(file).update("\0").update(readFileSync(path.join(here, file))).update("\0")
writeFileSync(dist("sources.txt"), hash.digest("hex") + "\n")
