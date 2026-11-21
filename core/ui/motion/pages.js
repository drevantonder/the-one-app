// Building a folder into what the harness loads: the UI library, an app's client and its page.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from "node:fs"
import { basename, dirname, join } from "node:path"
import { here, out, root } from "./options.js"

/// When a file under `paths` last changed, the latest.
const changed = (...paths) => Math.max(...paths.map((path) => (statSync(path).isDirectory() ? Math.max(0, ...readdirSync(path, { recursive: true }).map((file) => statSync(join(path, file)).mtimeMs)) : statSync(path).mtimeMs)))
/// What's built that's older than what it's built from, as after a pull, so what a run draws and checks would be of the
/// build, not the source: the UI library, or the harness where it's built. → what to build again, if anything.
export function stale(harness) {
  if (!existsSync(here("dist/build/app.js")) || changed(here("src"), here("tables.ts"), here("build.js")) > statSync(here("dist/build/app.js")).mtimeMs) return "Build the UI library: `bun build.js` in core/ui"
  if (existsSync(harness) && changed(join(root, "clients/app/renderer/src"), join(root, "clients/app/renderer/examples")) > statSync(harness).mtimeMs) return `Build the native harness again, which is older than the renderer: \`cargo build -p renderer --example screenshot --features fake${harness.includes("/release/") ? " --release" : ""}\``
}

/// The UI library's build a pass runs: its own, until the pass copies it (`copied`).
let dist = here("dist")
/// `path` in the UI library's build a pass runs.
export const library = (path) => join(dist, path)

/// What a pass runs, copied into `out` as it starts: the UI library's build, and the harness where it's this machine's,
/// so either built again while the pass goes on (by whoever else works in the same checkout) can't change what draws
/// the rest of it. → the harness to run.
export function copied(harness, out) {
  const kept = join(out, ".built")
  rmSync(kept, { recursive: true, force: true })
  cpSync(here("dist"), join(kept, "dist"), { recursive: true, dereference: true })
  dist = join(kept, "dist")
  if (!harness || !existsSync(harness)) return harness
  cpSync(harness, join(kept, basename(harness)))
  return join(kept, basename(harness))
}

/// A built page's folder as the harness loads it: its `index.html` with what it loads beside it, and each app it `uses`
/// (whose components it draws, as Settings draws its sections) built and beside it as `<app>/client/`, as scenes.js has it.
export function pageFolder(built, page, folder, uses = []) {
  const at = join(folder, "page")
  rmSync(at, { recursive: true, force: true })
  mkdirSync(at, { recursive: true })
  writeFileSync(join(at, "index.html"), readFileSync(join(built, page), "utf8").replace(/(src|href)="(\.\.\/)+/g, '$1="').replace(/(src|href)="\.\//g, '$1="'))
  // A component's page is beside the modules its client's build made, which it loads from beside it.
  if (page.startsWith(`${COMPONENTS}/`)) {
    for (const entry of readdirSync(join(built, COMPONENTS))) if (!entry.endsWith(".html")) symlinkSync(join(built, COMPONENTS, entry), join(at, entry))
  } else {
    symlinkSync(join(built, "assets"), join(at, "assets"))
    beside(join(built, dirname(page)), at)
  }
  for (const app of uses) {
    mkdirSync(join(at, app))
    symlinkSync(usedApp(app, out), join(at, app, "client"))
  }
  if (existsSync(join(built, dirname(page), "styles.json"))) cpSync(join(built, dirname(page), "styles.json"), join(at, "styles.json"))
  return at
}

/// What a server serves of an app's client as it was deployed, beside what it built (`OneCore.Entrypoint.build`): its
/// pictures, sounds, videos and fonts, which a page names by their paths (Settings' `updates.svg`), put in `built` as
/// the server has them.
const AS_IS = /\.(svg|png|jpe?g|gif|webp|avif|ico|mp3|wav|ogg|m4a|aac|mp4|webm|mov|woff2?|ttf|otf)$/i
export function servedAsIs(target, built) {
  const client = join(target, "client")
  if (!existsSync(client)) return
  for (const file of readdirSync(client, { recursive: true }).map(String).filter((file) => AS_IS.test(file) && !file.split("/").includes("node_modules")))
    cpSync(join(client, file), join(built, "client", file))
}

/// What's beside a page in `from` that it names by path, as `servedAsIs` put it, linked beside it in `at`.
export function beside(from, at) {
  if (!existsSync(from)) return
  for (const entry of readdirSync(from)) if (AS_IS.test(entry) && !existsSync(join(at, entry))) symlinkSync(join(from, entry), join(at, entry))
}

/// Where an app's exported components are built (its client, as a server builds it), for a scene that names one
/// (`component`) rather than a page: the component alone, drawn in grouped cards, as Settings draws its sections.
export const COMPONENTS = "components"

/// An app's own packages, which its client's package.json names, as a server installs them before it builds the app.
/// → why they didn't install, if they didn't.
function install(client) {
  const manifest = join(client, "package.json")
  if (!existsSync(manifest)) return
  const about = JSON.parse(readFileSync(manifest, "utf8"))
  if (!Object.keys({ ...about.dependencies, ...about.devDependencies }).length) return
  const made = Bun.spawnSync(["bun", "install", "--frozen-lockfile", "--ignore-scripts"], { cwd: client })
  if (made.exitCode !== 0) return made.stderr.toString().trim()
}

/// Builds `target` into `built` as a server builds an app: its packages, its page where one of its scenes draws a page
/// (an app whose scenes are all of components has none, and may have no page at all), what it keeps as it is, and its
/// client into COMPONENTS, with a page beside each module a scene names, that draws it. → why it didn't build, if it didn't.
export function build(target, built) {
  const packages = install(join(target, "client"))
  if (packages) return `its packages didn't install\n${packages}`
  const scenes = existsSync(join(target, "client/scenes.json")) ? JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8")) : {}
  const ofComponent = (scene) => !!scene && (scene.component || ofComponent(scenes[scene.like]))
  if (!Object.keys(scenes).length || Object.values(scenes).some((scene) => !ofComponent(scene))) {
    const made = Bun.spawnSync(["bun", library("build/app.js"), target, built])
    servedAsIs(target, built)
    if (made.exitCode !== 0) return `didn't build\n${made.stderr.toString().trim()}`
  }
  const named = [...new Set(Object.values(scenes).map((scene) => scene.component).filter(Boolean))]
  if (!named.length) return
  const at = join(built, COMPONENTS)
  const made = Bun.spawnSync(["bun", library("build/app.js"), join(target, "client"), at])
  if (made.exitCode !== 0) return `its components didn't build\n${made.stderr.toString().trim()}`
  for (const component of named) {
    writeFileSync(join(at, `${component}.scene.js`), `import { ListStyle, render } from "/ui/client/ui.js"
import Shown from "./${component}.js"
render(() => ListStyle({ value: "insetGrouped", get children() { return Shown({}) } }))
`)
    writeFileSync(join(at, `${component}.html`), `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">
<script type="module" src="./${component}.scene.js"></script>
`)
  }
}

/// An app another's page uses, its client built once a run, in `out`'s `.uses`. → where.
const usedApps = new Map()
export function usedApp(app, out) {
  if (!usedApps.has(app)) {
    const folder = join(out, ".uses", app)
    rmSync(folder, { recursive: true, force: true })
    const client = join(root, "apps", app, "client")
    const packages = install(client)
    if (packages) throw new Error(`${app}'s packages didn't install: ${packages}`)
    const made = Bun.spawnSync(["bun", library("build/app.js"), client, folder])
    if (made.exitCode !== 0) throw new Error(`${app}'s components didn't build: ${made.stderr}`)
    usedApps.set(app, folder)
  }
  return usedApps.get(app)
}
