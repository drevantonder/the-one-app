// Plays every scene's gestures (its `motions`) on a native host's headless harness, frame by frame, on its paused
// clock: `bun motion.js [<folder>…] [--scene <words>] [--hz 60,120] [--out <dir>] [--against <dir>] [--dark]
// [--release] [--watch] [--ssh <host> --harness <its build of the harness>]` in core/ui, after `bun build.js` and `cargo
// build -p renderer --example screenshot --features fake` (`--release` too for `--release`, whose costs are a shipped
// build's). `--ssh` plays them on a real device instead, as calibrate.js draws there (a Windows one's harness built with
// `cargo xwin build --release … --target x86_64-pc-windows-msvc`, which draws with Direct3D 12 over ssh), at its tier's
// device pixels a point, and judges its tier by each frame's own times there: its GPU's (its timestamps), its thread's
// painting and drawing, and its JS's. With `--present` too, and the `window` example built for it as `--harness`, each
// motion that plays a trace of a wheel's notches is played in a real window in the device's person's session instead
// (a scheduled task's), drawn and presented on its display as the app's windows are, which the headless harness never
// does: it fails where its frames' painting and drawing, or its GPU, took longer than a refresh at their 95th
// percentile, and says each part (waiting for an image, submitting, presenting).
//
// A folder is the gallery (core/ui/gallery, its scenes in `*.scenes.tsx`) or an app's (apps/<app>, in
// `client/scenes.json`), as `scenes.js` takes them; with none, the gallery and every app with a motion. A motion is
// `{ from, gesture, follow }` (the `Motion` type in src/types/ui.d.ts): the press goes down on what `from` names, moves
// as `gesture` says, and what `follow` names is watched move, with what scrolls under the press. Each is played at
// each rate of `--hz` (a display's frames a second: 60 and 120 unless said) by the inspector's `replay`, through the
// gesture and spring code a window runs, and for each it keeps, in `--out`:
//
// - `filmstrip.png`: eight of its frames, from the press to where it comes to rest, and `frames/` every one, each a
//   picture numbered as it was drawn (`000.png` the first), to look at frame by frame;
// - `curve.svg`: how far what moved had gone at each frame, beside where the finger was and where iOS would have it
//   from the lift (a node let go springing back by SwiftUI's `.spring`, a list flung slowing as UIScrollView does),
//   each frame that took longer than a frame's time to paint marked red;
// - `frames.json`: every frame's time, position and cost, as `replay` gives it;
//
// and it says, motion by motion, how many frames it took to come to rest and how many went over their budget, and how
// many each kind of device would drop (devices.json's tiers: a low-end Android phone, an older and a current iPhone, a
// laptop's Intel GPU at 4K, a browser's WebGPU), from each frame's work, the same on any device, as calibrate.js
// measured a real one's GPU take for it and as the tier's CPU and GPU score beside this machine's and that one's. With
// `--against` (an earlier run's `--out`), each is compared with that run's: how far apart the curves come, at most, and
// how much of the filmstrips differs (`changed.png`). `index.html` in `--out` is a contact sheet of them all, before
// and after side by side. `--watch` plays them again as their app, the gallery or the UI library changes, a folder's
// own motions alone where only it changed, each round said with how long it took.
//
// `--checks` plays each scene's checks instead (`checks`, the `CheckStep` type): what people do with it, as the UI
// SDK's own tests do (keyboard, pointer, focus, dismissing, typeahead, placement), and what must hold as they do, on
// the fake platform's window, once as it is and again with `--seeds` (3 unless said) seeds that make it adversarial
// (the harness's `--seed`: refreshes late, input as things move, presses held as long as a person might), and with
// each seed a check once failed on (`seeds`). Every check also comes to rest, leaves nothing drawn going, draws every
// picture, each within a few hundred ms of its page asking for it as the slowest device of its kind decodes it
// (devices.json's CPU scores, the harness's `--slower`), any menu, list or dialog open at its end is drawn whole in
// the window, and its last frame, drawn on what the window kept from frames before, is the one drawn afresh. Every
// scene is drawn the same too, the frame it keeps against one afresh, at rest, back from the background, with the
// system's appearance switched and on a computer its window made smaller; and every scene on a computer (not `touch`)
// is walked round by Tab, and what the keyboard comes to must draw otherwise than it does without it. It says, check
// by check, which passed, and for
// one that failed, its seed, which step and why; it exits 1 where one failed.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, watch, writeFileSync } from "node:fs"
import { basename, dirname, join, relative, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"
import { messages } from "./src/words.ts"

const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const root = here("../../")
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const out = resolve(option("--out") ?? join(process.env.TMPDIR ?? "/tmp", "motion"))
const against = option("--against") && resolve(option("--against"))
const rates = (option("--hz") ?? "60,120").split(",").map(Number)
const only = option("--scene")?.toLowerCase()
const dark = flag("--dark")
const checking = flag("--checks")
const seeds = Number(option("--seeds") ?? 3)
const folders = args.filter((arg, at) => !arg.startsWith("--") && !["--out", "--against", "--hz", "--scene", "--seeds", "--ssh", "--harness"].includes(args[at - 1])).map((folder) => resolve(folder))
// On a device of devices.json's tiers over ssh (the one whose `measured` it is), with a harness built for it, its own
// GPU and CPU draw each frame, as calibrate.js has them, and its tier is judged by the times it took there.
const ssh = option("--ssh")
const present = flag("--present")
let harness = ssh ? option("--harness") && resolve(option("--harness")) : join(root, `target/${flag("--release") ? "release" : "debug"}/examples/screenshot`)
/// How every harness runs, here and in scenes.js, on a machine others share: Vulkan on lavapipe, which draws on the
/// CPU with 2 threads rather than one a core (`LP_NUM_THREADS`), as a few run at once.
export const env = { ...process.env, VK_ICD_FILENAMES: process.env.VK_ICD_FILENAMES ?? "/usr/share/vulkan/icd.d/lvp_icd.json", LP_NUM_THREADS: process.env.LP_NUM_THREADS ?? "2" }
/// The most a harness may use, and for how long: a page that grows or never ends is stopped, not the machine.
const MEMORY = "3G"
const MINUTES = 10
/// `command` run within those, in a systemd scope of its own on Linux, where systemd is; elsewhere as it is.
export const capped = (command) => (Bun.which("systemd-run") ? ["systemd-run", "--user", "--scope", "--quiet", "-p", `MemoryMax=${MEMORY}`, "-p", "MemorySwapMax=0", "timeout", `${MINUTES}m`, ...command] : command)
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

/// The faces the harness falls back to where its own font hasn't a character, this machine's, which a device it's sent
/// to is sent beside it (renderer's `fake::fallback_fonts`).
export const FACES = ["/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", "/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf"]

/// Why a capped harness was stopped, by its exit status, if it was.
export const stopped = (status) => ({ 137: `it used more than ${MEMORY}B of memory, and was stopped`, 124: `it ran for ${MINUTES} minutes, and was stopped` })[status]


/// Every folder with motions, or with `--checks` every folder with scenes: the gallery, and each app's.
const everything = () => [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")) && (checking || readFileSync(join(app, "client/scenes.json"), "utf8").includes('"motions"')))]

/// How a gesture moves, as the inspector's `replay` takes it: each stretch `{dx, dy, ms}`, and whether it lifts.
export function moves(gesture) {
  const ms = ([dx, dy], speed) => (Math.hypot(dx, dy) / speed) * 1000
  if (gesture.moves) return { moves: gesture.moves, lift: gesture.lift !== false }
  // Input as a device delivered it: its events, which move nothing by a finger's moves.
  if (gesture.trace) return { moves: [], lift: false, trace: gesture.trace }
  if (gesture.swipe) return { moves: [{ dx: gesture.swipe[0], dy: gesture.swipe[1], ms: ms(gesture.swipe, gesture.speed ?? 800) }], lift: true }
  // A flick: a short way, quickly, as a thumb flicks a list.
  if (gesture.fling) return { moves: [{ dx: gesture.fling[0], dy: gesture.fling[1], ms: 80 }], lift: true }
  if (gesture.drag) {
    const parts = (gesture.pauses ?? 0) + 1
    const [dx, dy] = gesture.drag.map((by) => by / parts)
    const each = ms([dx, dy], gesture.speed ?? 400)
    const stretches = Array.from({ length: parts }, (_, at) => [...(at ? [{ ms: 200 }] : []), { dx, dy, ms: each }]).flat()
    // Held still before it's let go, so it's let go still.
    return { moves: [...stretches, { ms: 150 }], lift: true }
  }
  // A sheet pulled down, slowing, then flicked on as it's let go.
  if (gesture.pull) return { moves: [{ dy: gesture.pull * 0.75, ms: 400 }, { dy: gesture.pull * 0.25, ms: 70 }], lift: true }
  throw new Error(`A gesture is a swipe, a fling, a drag, a pull or its moves: ${JSON.stringify(gesture)}`)
}

/// What the inspector selects for a tap's name, as `scenes.js` presses it.
export const selector = (tap) => (typeof tap === "string" ? { name: tap } : tap)

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
<script src="/ui/client/colour-mode.js"></script>
<link rel="stylesheet" href="/ui/client/ui.css">
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

/// The folder on `--ssh`'s device that each run there is in, with the UI library and the harness beside them, made
/// as the first run needs it and taken away as this ends.
let remote
function onDevice() {
  if (remote) return remote
  const said = (command) => {
    const ran = Bun.spawnSync(["ssh", ssh, command], { stdout: "pipe", stderr: "pipe" })
    if (ran.exitCode !== 0) throw new Error(`${ssh}: ${command}: ${ran.stderr.toString().trim()}`)
    return ran.stdout.toString().trim()
  }
  remote = `${said("echo %TEMP%")}\\one-motion-${process.pid}`
  const staged = join(out, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(library("client"), join(staged, "ui"), { recursive: true, dereference: true })
  cpSync(harness, join(staged, "screenshot.exe"))
  for (const face of FACES) cpSync(face, join(staged, basename(face)))
  const sent = Bun.spawnSync(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} 'mkdir "${remote}" && tar -xf - -C "${remote}"'`], { stdout: "pipe", stderr: "pipe" })
  if (sent.exitCode !== 0) throw new Error(`${ssh}: the harness didn't go there: ${sent.stderr.toString().trim()}`)
  process.on("exit", () => Bun.spawnSync(["ssh", ssh, `rmdir /s /q "${remote}"`]))
  return remote
}

/// Runs the harness on `--ssh`'s device, in a folder of its own there with what it's given: its page, its answers and
/// its requests. → what it said on stdout and stderr, and whether it failed; its `end.json` back in `folder`.
export async function runThere(at, folder, args, lines) {
  const base = onDevice()
  const run = `${base}\\${basename(folder)}-${Math.random().toString(36).slice(2, 8)}`
  const staged = join(folder, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(at, join(staged, "page"), { recursive: true, dereference: true })
  cpSync(join(folder, "answers.json"), join(staged, "answers.json"))
  writeFileSync(join(staged, "requests.jsonl"), lines)
  const there = args.map((arg) => (arg === at ? `${run}\\page` : arg === join(folder, "answers.json") ? `${run}\\answers.json` : arg === "-" ? `${run}\\requests.jsonl` : arg.startsWith(folder) ? `${run}\\${basename(arg)}` : arg === library("client") ? `${base}\\ui` : arg))
  // Each argument quoted as Windows' C runtime reads one, a quote in it escaped (an `--errors` list's), and the whole
  // quoted for the shell here.
  const command = `mkdir "${run}" && tar -xf - -C "${run}" && "${base}\\screenshot.exe" ${there.map((arg) => `"${arg.replaceAll('"', '\\"')}"`).join(" ")} & type "${run}\\end.json" 1>&2 & rmdir /s /q "${run}"`
  const ran = Bun.spawn(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} '${command.replaceAll("'", "'\\''")}'`], { stdout: "pipe", stderr: "pipe" })
  const [stdout, stderr] = await Promise.all([new Response(ran.stdout).text(), new Response(ran.stderr).text(), ran.exited])
  rmSync(staged, { recursive: true, force: true })
  // Its tree, which it writes at its end, comes back after what it said.
  const tree = stderr.lastIndexOf("\n{")
  if (tree >= 0) writeFileSync(join(folder, "end.json"), stderr.slice(tree + 1))
  return { stdout, stderr: tree >= 0 ? stderr.slice(0, tree) : stderr, failed: ran.exitCode !== 0 || !stdout.trim() }
}

/// A motion's trace played in a real window on `--ssh`'s device, in its person's session, by the `window` example
/// (`--present`): a scheduled task runs it there, and says it's done on a named pipe, which is every session's, that
/// this waits on there, five minutes at most. → what it said of each round's
/// frames, as JSON lines, and whether it failed.
async function presentThere(at, folder, scene, events) {
  const base = onDevice()
  const name = `${basename(folder)}-${Math.random().toString(36).slice(2, 8)}`.replace(/[^\w-]/g, "")
  const run = `${base}\\${name}`
  const staged = join(folder, ".device")
  rmSync(staged, { recursive: true, force: true })
  cpSync(at, join(staged, "page"), { recursive: true, dereference: true })
  cpSync(join(folder, "answers.json"), join(staged, "answers.json"))
  writeFileSync(join(staged, "trace.json"), JSON.stringify(events))
  const client = scene.answers?.["one/client"]
  const panel = client?.launcher ? ["--panel", "16"] : []
  const args = ["page", "--trace", "trace.json", "--ui", `${base}\\ui`, "--answers", "answers.json", "--size", `${scene.size[0]}x${scene.size[1]}`, "--rounds", "3", ...panel]
  writeFileSync(join(staged, "run.cmd"), ["@echo off", `cd /d "${run}"`, `"${base}\\screenshot.exe" ${args.map((arg) => `"${arg}"`).join(" ")} > said.txt 2> err.txt`, "echo %errorlevel% > exit.txt", `echo done> \\\\.\\pipe\\${name}`].join("\r\n"))
  writeFileSync(
    join(staged, "wait.ps1"),
    [
      // Its person may write to it from their session, which a pipe's own permissions don't let them.
      "$security = New-Object System.IO.Pipes.PipeSecurity",
      "$security.AddAccessRule((New-Object System.IO.Pipes.PipeAccessRule([System.Security.Principal.WindowsIdentity]::GetCurrent().User, 'ReadWrite', 'Allow')))",
      `$pipe = New-Object System.IO.Pipes.NamedPipeServerStream('${name}', 'In', 1, 'Byte', 'None', 0, 0, $security)`,
      `schtasks /Create /F /SC ONCE /ST 23:59 /TN ${name} /TR '"${run}\\run.cmd"' /IT | Out-Null`,
      `schtasks /Run /TN ${name} | Out-Null`,
      "$null = $pipe.WaitForConnectionAsync().Wait(300000)",
      "$pipe.Dispose()",
      `schtasks /Delete /F /TN ${name} | Out-Null`,
    ].join("\r\n"),
  )
  const sent = Bun.spawnSync(["sh", "-c", `tar -C "${staged}" -cf - . | ssh ${ssh} 'mkdir "${run}" && tar -xf - -C "${run}"'`], { stdout: "pipe", stderr: "pipe" })
  rmSync(staged, { recursive: true, force: true })
  if (sent.exitCode !== 0) return { stdout: "", stderr: sent.stderr.toString(), failed: true }
  const task = `powershell -NoProfile -ExecutionPolicy Bypass -File "${run}\\wait.ps1" & type "${run}\\exit.txt" & type "${run}\\said.txt" & type "${run}\\err.txt" 1>&2 & rmdir /s /q "${run}"`
  const ran = Bun.spawn(["ssh", ssh, task], { stdout: "pipe", stderr: "pipe" })
  const [stdout, stderr] = await Promise.all([new Response(ran.stdout).text(), new Response(ran.stderr).text(), ran.exited])
  const [exit, ...said] = stdout.split("\n")
  return { said: said.filter((line) => line.trim().startsWith("{")).map((line) => JSON.parse(line)), stderr, failed: exit.trim() !== "0" }
}

/// Whether `scene`'s window is a panel: a computer's launcher is drawn as one (a Mac's, its glass the system's), as its
/// client says it is.
const panelled = (scene) => {
  const client = scene.answers?.["one/client"]
  return Boolean(client?.launcher && client?.mac)
}

/// The harness run on a scene, with `requests`: each answer, and its stderr. → `{ answers, said }`.
async function run(at, folder, scene, requests, seed = 0) {
  const answers = { ...scene.answers, "ui/initialize": scene.answers?.["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  const lines = requests.map((request, id) => JSON.stringify({ v: 1, id: id + 1, ...request })).join("\n")
  const panel = panelled(scene) ? ["--panel", "0"] : []
  // Its pictures decode as slowly as on the slowest device of its kind: as this machine's CPU is to that one's, at
  // that one's device pixels a point, where it's drawn at one here; on a device, as that decodes them.
  const slowest = devices.tiers.find((tier) => tier.name === (scene.touch ? "low-end Android" : "Intel UHD laptop at 4K"))
  const slower = hostScore && !ssh ? ["--slower", String((hostScore / slowest.cpu) * slowest.scale ** 2)] : []
  const args = [at, join(folder, "end.png"), join(folder, "end.json"), "--ui", library("client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", String(device?.scale ?? 1), "--requests", "-", ...(dark ? ["--dark"] : []), ...(scene.touch ? ["--touch"] : []), ...(scene.errors ? ["--errors", JSON.stringify(scene.errors)] : []), ...(scene.safeArea ? ["--safe-area", scene.safeArea.join(",")] : []), ...(scene.latency ? ["--latency", String(scene.latency)] : []), "--time-zone", scene.timeZone ?? "UTC", ...(seed ? ["--seed", String(seed)] : []), ...panel, ...slower]
  if (ssh) {
    const { stdout, stderr, failed } = await runThere(at, folder, args, lines)
    return { answers: stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line)), said: stderr.trim(), failed }
  }
  const harnessed = Bun.spawn(capped([harness, ...args]), { stdin: new TextEncoder().encode(lines), stdout: "pipe", stderr: "pipe", env })
  const [stdout, stderr] = await Promise.all([new Response(harnessed.stdout).text(), new Response(harnessed.stderr).text(), harnessed.exited])
  return { answers: stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line)), said: [stderr.trim(), stopped(harnessed.exitCode)].filter(Boolean).join("\n"), failed: harnessed.exitCode !== 0 }
}

/// A folder's scenes that have motions, or with `all`, every one an app's page draws, each with its page, link,
/// answers, taps, size and touch.
export async function scenesOf(target, built, folder, all = false) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const scenes = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(scenes[scene.like]), taps: undefined, ...scene, answers: { ...whole(scenes[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(scenes)
      .map(([name, scene]) => ({ name, ...whole(scene) }))
      .filter((scene) => all || (checking ? scene.checks : scene.motions))
      .map(({ name, page, component, uses = [], answers = {}, taps = [], size = [390, 844], touch = false, timeZone = "UTC", safeArea, errors, latency, motions, checks, seeds }) => {
        const [at, link] = (page ?? (component ? `${COMPONENTS}/${component}.html` : "client/index.html")).split("#")
        return { name, page: at, link, uses, answers, taps, size, touch, timeZone, safeArea, errors, latency, motions, checks, seeds }
      })
  }
  // The gallery lists its own, drawn at `#scenes`.
  const listing = { link: "scenes", size: [390, 844], answers: {} }
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, "index.html", folder)
  const listed = await run(at, folder, listing, [])
  const text = JSON.stringify(JSON.parse(readFileSync(join(folder, "end.json"), "utf8"))).match(/"name":"(\[.*?\])"/)?.[1]
  if (!text) throw new Error(`The gallery listed no scenes: ${listed.said}`)
  return JSON.parse(JSON.parse(`"${text}"`))
    .filter((scene) => all || (checking ? scene.checks : scene.motions))
    .map(({ name, taps = [], size = [390, 844], touch = false, safeArea, errors, motions, checks, seeds }) => ({ name, page: "index.html", link: name, answers: {}, taps, size, touch, safeArea, errors, motions, checks, seeds }))
}

/// How far what moved had gone at each frame: what `follow` names where it's said and moved, else what scrolls.
function travel(frames) {
  const series = (pick) => frames.map((frame) => pick(frame)).map((at) => (at === undefined ? undefined : at - pick(frames[0])))
  const candidates = [
    ["along", series((frame) => frame.x)],
    ["down", series((frame) => frame.y)],
    ["scrolled along", series((frame) => frame.scroll?.x)],
    ["scrolled down", series((frame) => frame.scroll?.y)],
  ].filter(([, values]) => values.every((value) => value !== undefined))
  const range = (values) => Math.max(...values) - Math.min(...values)
  return candidates.sort(([, a], [, b]) => range(b) - range(a))[0] ?? ["still", frames.map(() => 0)]
}

/// Where the finger was `t` ms in, along its moves, the way `travel` measures: on what it moves, or against what scrolls.
function finger(stretches, t, way) {
  let [x, y, since] = [0, 0, 0]
  for (const { dx = 0, dy = 0, ms } of stretches) {
    const part = ms <= 0 ? 1 : Math.min(Math.max((t - since) / ms, 0), 1)
    ;[x, y] = [x + dx * part, y + dy * part]
    if (t < since + ms) break
    since += ms
  }
  return { along: x, down: y, "scrolled along": -x, "scrolled down": -y }[way] ?? 0
}

/// iOS's own motion, as Apple gives it: SwiftUI's springs (`.spring(duration:bounce:)`: a mass of 1, stiffness
/// (2π/duration)², damping 4π(1 − bounce)/duration), its default `.spring` 0.5 s with no bounce, which a node let go
/// springs back by, and UIScrollView's normal deceleration, 0.998 of its speed kept each millisecond.
export const swiftui = { spring: [500, 0], smooth: [500, 0], snappy: [500, 0.15], bouncy: [500, 0.3] }
const DECELERATION = 0.998

/// Where a spring of `duration` ms and `bounce`, `from` its rest and moving `velocity` a second, is `t` ms later.
export function spring([duration, bounce], from, velocity, t) {
  const response = Math.max(duration / 1000, 0.01)
  const fraction = bounce >= 0 ? 1 - bounce : 1 / (1 + bounce)
  const [k, c, s] = [(2 * Math.PI / response) ** 2, (4 * Math.PI * fraction) / response, t / 1000]
  const natural = Math.sqrt(k)
  const ratio = c / (2 * natural)
  if (Math.abs(ratio - 1) < 1e-4) return (from + (velocity + natural * from) * s) * Math.exp(-natural * s)
  if (ratio < 1) {
    const damped = natural * Math.sqrt(1 - ratio * ratio)
    return Math.exp(-ratio * natural * s) * (from * Math.cos(damped * s) + ((velocity + ratio * natural * from) / damped) * Math.sin(damped * s))
  }
  const root = natural * Math.sqrt(ratio * ratio - 1)
  const [r1, r2] = [-ratio * natural + root, -ratio * natural - root]
  const a = (velocity - r2 * from) / (r1 - r2)
  return a * Math.exp(r1 * s) + (from - a) * Math.exp(r2 * s)
}

/// What iOS would draw after the press lifts, frame by frame from the first after it, where it says: a node let go
/// springs back by SwiftUI's `.spring` from where it was drawn at the speed the finger had; what scrolls, flung at
/// the finger's speed, slows as UIScrollView does. A pause before the lift leaves no speed. → each frame's value,
/// or none before the lift, or none where iOS gives nothing to compare (a stretch past an end springing back, or a
/// tap, which moves nothing).
function ios(frames, way, values, stretches) {
  const lift = stretches.reduce((sum, { ms }) => sum + ms, 0)
  const last = stretches.at(-1)
  if (!last) return undefined
  const speed = last.dx || last.dy ? ((way.endsWith("along") ? last.dx ?? 0 : last.dy ?? 0) / last.ms) * 1000 : 0
  const at = frames.findIndex((frame) => frame.at >= lift)
  if (at < 0) return undefined
  const [start, rest] = [values[at], values.at(-1)]
  if (way.startsWith("scrolled")) {
    // A scroll still stretched past its end, or that comes to one, springs back as iOS's does, which Apple doesn't give.
    if (frames.slice(at).some((frame) => (frame.scroll?.y ?? 0) < 0 || (frame.scroll?.x ?? 0) < 0) || values.slice(at).some((value) => Math.abs(value - start) > Math.abs(rest - start) + 0.5)) return undefined
    const velocity = -speed
    return frames.map((frame, n) => (n < at ? undefined : start + (velocity * (DECELERATION ** (frame.at - frames[at].at) - 1)) / Math.log(DECELERATION) / 1000))
  }
  return frames.map((frame, n) => (n < at ? undefined : rest + spring(swiftui.spring, start - rest, speed, frame.at - frames[at].at)))
}

/// The curve: what moved, and the finger, over time, each frame over its budget red.
function curve(frames, way, values, stretches, budget, before, apple) {
  const [width, height, pad] = [640, 320, 40]
  const end = Math.max(...frames.map((frame) => frame.at), ...(before?.frames.map((frame) => frame.at) ?? []))
  const fingers = frames.map((frame) => finger(stretches, frame.at, way))
  const all = [...values, ...fingers, ...(before?.values ?? [])]
  const [low, high] = [Math.min(0, ...all), Math.max(1, ...all)]
  const x = (t) => pad + (t / end) * (width - 2 * pad)
  // Its legend above it.
  const y = (v) => height - pad - ((v - low) / (high - low)) * (height - 2 * pad - 20)
  const line = (points, colour, dash = "") => `<polyline fill="none" stroke="${colour}" stroke-width="2" ${dash && `stroke-dasharray="${dash}"`} points="${points.map(([t, v]) => `${x(t).toFixed(1)},${y(v).toFixed(1)}`).join(" ")}"/>`
  // A frame waits on its JS, its painting and its GPU before the refresh shows it (the harness's `inspect::over`).
  const over = frames.filter((frame) => (frame.script_ms ?? 0) + frame.paint_ms + (frame.gpu_ms ?? 0) > budget)
  return `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" font-family="system-ui" font-size="11">
<rect width="100%" height="100%" fill="white"/>
<line x1="${pad}" y1="${y(0)}" x2="${width - pad}" y2="${y(0)}" stroke="#ddd"/>
${line(frames.map((frame, at) => [frame.at, fingers[at]]), "#999", "4 4")}
${before ? line(before.frames.map((frame, at) => [frame.at, before.values[at]]), "#e8a23a") : ""}
${apple ? line(frames.flatMap((frame, at) => (apple[at] === undefined ? [] : [[frame.at, apple[at]]])), "#3a9a4a", "2 3") : ""}
${line(frames.map((frame, at) => [frame.at, values[at]]), "#2f6fde")}
${over.map((frame) => `<circle cx="${x(frame.at).toFixed(1)}" cy="${y(values[frames.indexOf(frame)]).toFixed(1)}" r="3" fill="#d33"/>`).join("")}
<text x="${pad}" y="16">${way}, in points, over ${Math.round(end)} ms</text>
<text x="${width - pad}" y="16" text-anchor="end" fill="#2f6fde">drawn</text><text x="${width - pad}" y="30" text-anchor="end" fill="#999">finger</text>${before ? `<text x="${width - pad}" y="44" text-anchor="end" fill="#e8a23a">before</text>` : ""}${apple ? `<text x="${width - pad}" y="${before ? 58 : 44}" text-anchor="end" fill="#3a9a4a">iOS</text>` : ""}${over.length ? `<text x="${pad}" y="30" fill="#d33">${over.length} frames over ${budget.toFixed(1)} ms</text>` : ""}
<text x="${pad}" y="${height - 12}">0</text><text x="${width - pad}" y="${height - 12}" text-anchor="end">${Math.round(end)} ms</text>
<text x="4" y="${y(high) + 4}">${Math.round(high)}</text><text x="4" y="${y(low) + 4}">${Math.round(low)}</text>
</svg>`
}

/// Eight frames, evenly over its time, side by side at half their size, cut to the band of rows anything moved in.
function filmstrip(frames, file) {
  const picked = Array.from({ length: 8 }, (_, at) => frames[Math.round((at / 7) * (frames.length - 1))])
  const pictures = picked.map((frame) => PNG.sync.read(Buffer.from(frame.png, "base64")))
  const { width, height } = pictures[0]
  const moved = (y) => pictures.some((picture) => picture.data.compare(pictures[0].data, y * width * 4, (y + 1) * width * 4, y * width * 4, (y + 1) * width * 4) !== 0)
  const rows = Array.from({ length: height }, (_, y) => y).filter(moved)
  const [top, bottom] = rows.length ? [Math.max(0, rows[0] - 24), Math.min(height, rows.at(-1) + 25)] : [0, height]
  const [w, h] = [Math.floor(width / 2), Math.floor((bottom - top) / 2)]
  const strip = new PNG({ width: w * pictures.length, height: h })
  pictures.forEach((picture, n) => {
    for (let py = 0; py < h; py++)
      for (let px = 0; px < w; px++)
        for (let c = 0; c < 4; c++) {
          const at = (sx, sy) => picture.data[((top + sy) * width + sx) * 4 + c]
          const sum = at(2 * px, 2 * py) + at(2 * px + 1, 2 * py) + at(2 * px, 2 * py + 1) + at(2 * px + 1, 2 * py + 1)
          strip.data[(py * strip.width + n * w + px) * 4 + c] = sum >> 2
        }
  })
  writeFileSync(file, PNG.sync.write(strip))
}

/// How far apart two curves come, at most, the earlier's read at each of the later's times.
function apart(frames, values, before) {
  const at = (t) => {
    const next = before.frames.findIndex((frame) => frame.at >= t)
    if (next <= 0) return before.values[next === 0 ? 0 : before.values.length - 1]
    const [a, b] = [before.frames[next - 1], before.frames[next]]
    return before.values[next - 1] + ((t - a.at) / (b.at - a.at)) * (before.values[next] - before.values[next - 1])
  }
  return Math.max(...frames.map((frame, n) => Math.abs(values[n] - at(frame.at))))
}

/// The devices frames are judged against, and what this machine's CPU scores, as devices.json says.
const devices = JSON.parse(readFileSync(here("devices.json"), "utf8"))
const cpuName = process.platform === "darwin" ? Bun.spawnSync(["sysctl", "-n", "machdep.cpu.brand_string"]).stdout.toString() : (readFileSync("/proc/cpuinfo", "utf8").match(/model name\s*:\s*(.*)/)?.[1] ?? "")
const hostScore = Object.entries(devices.hosts).find(([name]) => cpuName.includes(name))?.[1]
const device = ssh && devices.tiers.find((tier) => tier.measured === ssh)
if (ssh && !device) throw new Error(`No tier in devices.json is measured on ${ssh}`)

/// What each tier with `hz` frames a second would take for each of `frames`, played at one device pixel a point: its
/// CPU's time for the frame (painting it and drawing it for the GPU) and its JS's since the frame before (this machine's, as its Geekbench score is to the
/// tier's) and its GPU's (what the device
/// measured takes for the frame's work at the tier's pixels, as its GPU's score is to the tier's), the frame dropped
/// where either is longer than the tier's frame. Its JS counts only where the frame `waits` for it: what a wheel scrolls
/// is drawn whatever its JS is doing. → each tier's name, the frames dropped of those drawn, the worst and why.
export function judge(frames, hz, waits = true) {
  // Drawn on the device itself, its own frames' times: the GPU's by its timestamps, its thread's painting and drawing.
  if (device) {
    if (device.hz !== hz) return []
    const times = frames.filter((frame) => frame.draws > 0).map((frame) => {
      const [gpuMs, cpuMs, jsMs] = [frame.gpu_ms ?? 0, frame.paint_ms + frame.draw_ms, waits ? frame.script_ms ?? 0 : 0]
      const ms = Math.max(gpuMs, cpuMs, jsMs)
      return { ms, by: ms === gpuMs ? "GPU" : ms === jsMs ? "JS" : "CPU" }
    })
    const worst = times.reduce((worst, time) => (time.ms > worst.ms ? time : worst), { ms: 0, by: "" })
    return [{ tier: device.name, dropped: times.filter((time) => time.ms > 1000 / hz).length, drawn: times.length, worst }]
  }
  return devices.tiers
    .filter((tier) => tier.hz === hz)
    .map((tier) => {
      const anchor = devices.tiers.find((one) => one.measured && devices.measured[one.measured])
      const gpu = anchor && devices.measured[anchor.measured].gpu
      const pixels = tier.scale * tier.scale
      const times = frames
        .filter((frame) => frame.draws > 0)
        .map((frame) => {
          const work = frame.work
          const shaded = gpu ? ["quads", "shadows", "sprites", "backdrops", "rasters", "copied"].reduce((sum, kind) => sum + (work[kind] / 1e6) * pixels * gpu.costs[kind], 0) : 0
          const gpuMs = gpu ? (shaded + (work.uploaded / 1e6) * pixels * gpu.costs.uploaded + gpu.constant) * (anchor.gpu / tier.gpu) + frame.draws * gpu.costs.draws * (tier.draws ?? 1) * (anchor.cpu / tier.cpu) : 0
          // The thread that paints a frame then draws it for the GPU; its JS runs on a thread of its own beside, and a
          // frame waits for whichever's longer.
          const [cpuMs, jsMs] = hostScore ? [frame.paint_ms + (frame.draw_ms ?? 0), waits ? frame.script_ms ?? 0 : 0].map((ms) => (ms * hostScore) / tier.cpu) : [0, 0]
          const ms = Math.max(gpuMs, cpuMs, jsMs)
          return { ms, by: ms === gpuMs ? "GPU" : ms === jsMs ? "JS" : "CPU" }
        })
      const budget = 1000 / tier.hz
      const worst = times.reduce((worst, time) => (time.ms > worst.ms ? time : worst), { ms: 0, by: "" })
      return { tier: tier.name, dropped: times.filter((time) => time.ms > budget).length, drawn: times.length, worst }
    })
}

/// A motion played at `hz` on a scene: its frames, filmstrip and curve kept in `folder`. → what it says, and its row in
/// the contact sheet.
async function play(built, scene, name, motion, hz, folder) {
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, scene.page, folder, scene.uses)
  const { moves: stretches, lift, trace } = moves(motion.gesture)
  const events = trace && JSON.parse(readFileSync(join(scene.from, trace), "utf8"))
  const taps = scene.taps.flatMap((tap) => [tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...selector(tap) }, { op: "clock", step: 1000 }])
  if (present) {
    if (!events?.some((event) => event.kind === "wheel")) return { said: `  ${hz} Hz: not a wheel's, which only the headless harness plays` }
    if (hz !== device.hz) return { said: `  ${hz} Hz: the device's display is ${device.hz} Hz` }
    const answers = { ...scene.answers, "ui/initialize": scene.answers?.["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
    writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
    const shown = await presentThere(at, folder, scene, events)
    const ms = (value) => `${value} ms`
    const lines = shown.said.map((round, n) => `  ${hz} Hz presented, round ${n + 1}: ${round.frames} frames, p95 painting ${ms(round.paint_ms)}, drawing ${ms(round.draw_ms)} (waiting for an image ${ms(round.acquire_ms)}, submitting ${ms(round.submit_ms)}, presenting ${ms(round.present_ms)}), GPU ${ms(round.gpu_ms)}`)
    if (!shown.said.length) lines.push(`  ${hz} Hz presented: nothing said: ${shown.stderr.trim().split("\n").at(-1)}`)
    if (shown.failed) {
      process.exitCode = 1
      lines.push(`  ${hz} Hz presented FAILS: ${device.name}'s frames took longer than its ${(1000 / hz).toFixed(1)} ms refresh`)
    }
    return { said: lines.join("\n") }
  }
  // Where what it opens should first be drawn: what `opensFrom` names, before the press.
  const origin = motion.opensFrom && { op: "query", ...selector(motion.opensFrom) }
  const replay = { op: "replay", ...(motion.from ? selector(motion.from) : {}), ...(events ? { events } : { moves: stretches, lift }), hz, snap: true, ...(motion.follow ? { track: selector(motion.follow) } : {}), ...(motion.frameMs ? { frame_ms: motion.frameMs } : {}) }
  const started = performance.now()
  const ran = await run(at, folder, scene, [{ op: "clock", step: 1000 }, ...taps, ...(origin ? [origin] : []), replay])
  const answer = ran.answers.at(-1)
  if (ran.failed || !answer?.result) return { said: `  ${hz} Hz: ${answer?.error ?? ran.said.split("\n").at(-1)}` }
  const { frames, budget_ms: budget, over, leaving } = answer.result
  // Whether its frames wait for its page's JS: a wheel's notches and the pointer moving don't, as a window scrolls them
  // at once, as a browser's compositor does (`Windows::waits`); a press, a key or a finger does.
  const waits = !events?.every((event) => ["wheel", "move"].includes(event.kind))
  const [way, values] = travel(frames)
  const kept = frames.map(({ png, width, height, ...frame }) => frame)
  const beforeAt = against && join(against, relative(out, folder))
  const before = beforeAt && existsSync(join(beforeAt, "frames.json")) ? JSON.parse(readFileSync(join(beforeAt, "frames.json"), "utf8")) : undefined
  const beforeTravel = before && { frames: before.frames, values: travel(before.frames)[1] }
  writeFileSync(join(folder, "frames.json"), JSON.stringify({ hz, budget, way, frames: kept }))
  const apple = ios(frames, way, values, stretches)
  writeFileSync(join(folder, "curve.svg"), curve(frames, way, values, stretches, budget, beforeTravel, apple))
  filmstrip(frames, join(folder, "filmstrip.png"))
  rmSync(join(folder, "frames"), { recursive: true, force: true })
  mkdirSync(join(folder, "frames"))
  frames.forEach((frame, n) => writeFileSync(join(folder, "frames", `${String(n).padStart(3, "0")}.png`), Buffer.from(frame.png, "base64")))
  const worst = Math.max(...frames.map((frame) => frame.paint_ms))
  const script = Math.max(...frames.map((frame) => frame.script_ms ?? 0))
  const rest = frames.at(-1).at
  const said = [`  ${hz} Hz: ${way} ${Math.round(Math.max(...values.map(Math.abs)))} pt, at rest after ${Math.round(rest)} ms (${frames.length} frames${motion.frameMs ? `, one each ${motion.frameMs} ms` : ""}), ${over} over its ${budget} ms budget, the worst ${worst} ms painting and ${script} ms of JS, in ${Math.round(performance.now() - started)} ms`]
  // What fails it: anything it took out still drawn once it's settled, and glass it should keep blurred drawn plain.
  const failed = []
  if (leaving) failed.push(`${leaving} taken out still drawn going once it settled`)
  const plain = motion.blurred ? frames.findIndex((frame, n) => n > 0 && !frame.work?.backdrops) : -1
  // What a swipe moves goes one way while it's under the fingers, a trace's till they lift: never back.
  const lifted = events?.findLast((event) => event.kind === "up" || ["ended", "cancelled"].includes(event.phase))
  const end = events ? (lifted ?? events.at(-1)).ms : stretches.reduce((sum, stretch) => sum + stretch.ms, 0)
  const toward = Math.sign(values.at(-1) - values[0])
  const back = motion.steady ? values.findIndex((value, n) => n > 0 && frames[n].at <= end && (value - values[n - 1]) * toward < -1) : -1
  if (back >= 0) failed.push(`it went back ${Math.abs(values[back] - values[back - 1]).toFixed(0)} pt at frame ${back + 1} (${frames[back].at} ms)`)
  // What it opens is first drawn where what it opens from is, as an iPhone's folder grows out of its square: within a
  // point, or a step of its own motion, which may have taken one before the frame it's first drawn on.
  if (origin) {
    const from = ran.answers.at(-2)?.result?.nodes?.[0]?.rect
    const drawn = frames.flatMap((frame) => (frame.rect ? [frame.rect] : []))
    const apart = (a, b) => Math.max(...["x", "y", "width", "height"].map((side) => Math.abs(a[side] - b[side])))
    const [first] = drawn
    const next = drawn.find((one) => apart(one, first) > 0)
    if (!from || !first) failed.push(`${from ? "what it follows" : "what it opens from"} isn't there to compare`)
    else if (apart(first, from) > Math.max(1, next ? apart(next, first) : 0)) failed.push(`it's first drawn at ${rect(first)}, not where it opens from, ${rect(from)}`)
  }
  if (plain >= 0) failed.push(`glass not blurred from frame ${plain + 1} of ${frames.length} (${frames[plain].at} ms, pressure ${frames[plain].pressure})`)
  // Every frame within the budget of the device it's to keep (`keeps`), at that device's rate, its JS too, the frames a
  // press lands and lifts on as much as any: what a press sets off starts on the next frame, as on iOS, not a hitch on.
  const tier = motion.keeps && devices.tiers.find((tier) => tier.name === motion.keeps)
  if (motion.keeps && (!tier || (!hostScore && !device))) failed.push(tier ? `this CPU isn't one of devices.json's hosts, to judge "${motion.keeps}" by` : `no tier named "${motion.keeps}" in devices.json`)
  const keeping = tier?.hz === hz && (hostScore || device) && judge(frames, hz, waits).find(({ tier }) => tier === motion.keeps)
  if (keeping?.dropped) failed.push(`${keeping.tier} drops ${keeping.dropped} of ${keeping.drawn} frames (the worst ${keeping.worst.ms.toFixed(1)} ms, its ${keeping.worst.by})`)
  if (failed.length) {
    process.exitCode = 1
    said.push(`  ${hz} Hz FAILS: ${failed.join("; ")}`)
  }
  if (apple) {
    const compared = frames.flatMap((frame, n) => (apple[n] === undefined ? [] : [Math.abs(values[n] - apple[n])]))
    said.push(`  ${hz} Hz against iOS (${way.startsWith("scrolled") ? "UIScrollView's deceleration" : "SwiftUI's .spring"}, from the lift): at most ${Math.max(...compared).toFixed(1)} pt apart`)
  }
  const judged = judge(frames, hz, waits)
  if (judged.length)
    said.push(`  ${hz} Hz on devices: ${judged.map(({ tier, dropped, drawn, worst }) => `${tier} drops ${dropped} of ${drawn} frames (the worst ${worst.ms.toFixed(1)} ms, its ${worst.by})`).join("; ")}`)
  if (before) {
    const diff = existsSync(join(beforeAt, "filmstrip.png")) && differs(join(beforeAt, "filmstrip.png"), join(folder, "filmstrip.png"), join(folder, "changed.png"))
    said.push(`  ${hz} Hz against before: curves at most ${apart(frames, values, beforeTravel).toFixed(1)} pt apart, at rest after ${Math.round(before.frames.at(-1).at)} ms then, filmstrips ${diff === undefined ? "not the same size" : `${(diff * 100).toFixed(1)}% different`}`)
  }
  return { said: said.join("\n"), row: { scene: scene.name, motion: name, hz, folder: relative(out, folder), before: before && beforeAt } }
}

/// A rect as it's said: where, and how big.
const rect = ({ x, y, width, height }) => `${Math.round(x)}, ${Math.round(y)} ${Math.round(width)} × ${Math.round(height)}`

/// A tap as the harness's request, as scenes.js presses it.
const pressed = (tap) => (tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...selector(tap) })

/// The roles a layer over the page has, which a check's end finds drawn whole in the window wherever it's open.
const LAYERS = ["menu", "listbox", "dialog", "alertdialog"]
/// The longest a picture may take to show from when its page asks for it, on the slowest device of its kind: a few
/// hundred ms, as the App Store's icons on iOS show with their page or fade in just after.
const PICTURE_MS = 300

/// Why a node `wait_for` waited on isn't as it should be: what a person sees of it, or its states.
const why = (answer, is) => {
  const node = answer?.result?.node
  if (!node) return "it isn't there"
  const off = Object.entries(is ?? {}).filter(([state, want]) => (node.states?.[state] ?? false) !== want)
  if (off.length) return off.map(([state]) => `${state} is ${node.states?.[state] ?? false}`).join(", ")
  if (node.opacity === undefined) return "it isn't drawn"
  if (node.opacity < 0.99) return `it's drawn at opacity ${node.opacity}`
  if (node.seen < 0.99) return `${Math.round(node.seen * 100)}% of it is in the window`
  return "it isn't"
}

/// A check's steps as the harness's requests, each with what judges its answer: why it failed, or nothing.
function requests(steps, folder) {
  const named = (tap) => (typeof tap === "string" ? tap : tap.name ?? tap.text ?? JSON.stringify(tap))
  const out = []
  // What the window looked like before anything's done, which `dimmed` compares what's over the page with.
  let before
  if (steps.some((step) => step.expect?.dimmed)) out.push({ request: { op: "snap" }, judge: (answer) => void (before = answer?.result?.png) })
  for (const [at, step] of steps.entries()) {
    const from = out.length
    if (step.wait !== undefined) out.push({ request: { op: "clock", step: step.wait } })
    else if (step.away !== undefined) out.push({ request: { op: "away", ms: step.away }, judge: (answer) => answer?.result?.drawn > 0 && `it drew ${answer.result.drawn} frames in the background, which an iPhone's GPU refuses, with the glyphs and pictures they uploaded` })
    else if (step.putAway) out.push({ request: { op: "put_away" } })
    else if (step.summon) out.push({ request: { op: "summon" } })
    else if (step.back) out.push({ request: { op: "back" } })
    else if (step.answers) out.push({ request: { op: "changed", answers: step.answers } })
    // An agent's press comes the inspector's way, as to a device's window, though nobody could press it (a panel put away).
    else if (step.press) out.push({ request: { op: "tap", ...selector(step.press), hold: step.hold, clicks: step.clicks, ...(step.agent ? { inspector: true } : {}) } })
    else if (step.hover) out.push({ request: { op: "hover", ...selector(step.hover) } })
    else if (step.down) out.push({ request: { op: "down", ...selector(step.down) } })
    else if (step.up) out.push({ request: { op: "up" } })
    else if (step.trace) out.push({ request: { op: "trace", events: JSON.parse(readFileSync(join(folder, step.trace), "utf8")) } })
    else if (!step.expect) out.push({ request: pressed(step) })
    else {
      const { shown, gone, focused, text, states, drawn, blur, keyboard, margins, tall, whole, unclipped, centred, widest, contrast, textMenu, handles, magnifier, dimmed, js } = step.expect
      // Whether `settle` says it's settled, waiting a moment, as a check after it reads the page: never while an answer
      // it waits on is on its way (Playwright's network idle).
      if (step.expect.settled !== undefined) out.push({ request: { op: "settle", deadline: 300 }, judge: (answer) => answer?.result?.settled !== step.expect.settled && (step.expect.settled ? `it hadn't settled: ${JSON.stringify(answer?.result?.pending)}` : "it said it had settled, with what it waits for on its way") })
      // Where something's drawn, so what comes in above it isn't seen to push it.
      if (step.expect.at) out.push({ request: { op: "query", ...selector(step.expect.at.of) }, judge: (answer) => { const y = answer?.result?.nodes?.[0]?.rect?.y; return (y === undefined || Math.abs(y - step.expect.at.y) > 1) && `${named(step.expect.at.of)} is drawn at y ${y}, not ${step.expect.at.y}` } })
      if (js !== undefined) out.push({ request: { op: "window" }, judge: (answer) => answer?.result?.js_bytes > js * 1e6 && `its JS holds ${(answer.result.js_bytes / 1e6).toFixed(1)} MB, over ${js} MB` })
      // Each waited for, as Playwright's assertions are, a second at most, or `within` ms.
      const until = (tap, state, said, is) => ({ request: { op: "wait_for", ...selector(tap), state, ...(is ? { is } : {}), ...(step.within !== undefined ? { timeout: step.within } : {}) }, judge: (answer) => !answer?.result?.met && said(answer) })
      if (shown) out.push(until(shown, "shown", (answer) => `${named(shown)} isn't shown: ${why(answer)}`))
      if (gone) out.push(until(gone, "gone", () => `${named(gone)} is still there`))
      if (text) out.push(until({ text }, "there", () => `nothing says "${text}"`))
      if (focused) out.push(until(focused, "focused", () => `${named(focused)} hasn't the keyboard`))
      if (drawn)
        out.push({
          ...until(drawn.of, "drawn", (answer) => `${named(drawn.of)} is drawn at opacity ${answer?.result?.node?.opacity}, not ${drawn.opacity}`),
          request: { op: "wait_for", ...selector(drawn.of), state: "drawn", opacity: drawn.opacity, ...(step.within !== undefined ? { timeout: step.within } : {}) },
        })
      if (states) out.push(until(states.of, "there", (answer) => `${named(states.of)}: ${why(answer, states.is)}`, states.is))
      if (keyboard)
        out.push({
          request: { op: "keyboard" },
          judge: (answer) => {
            const asked = answer?.result?.keyboard
            if (!asked) return "nothing has the keyboard"
            const off = Object.entries(keyboard).filter(([key, want]) => JSON.stringify(asked[key]) !== JSON.stringify(want))
            return off.length ? `the keyboard is asked for ${off.map(([key]) => `${key} ${JSON.stringify(asked[key])}`).join(", ")}` : undefined
          },
        })
      if (margins !== undefined) {
        let width
        out.push({ request: { op: "window" }, judge: (answer) => void (width = answer?.result?.width) })
        out.push({
          request: { op: "query", text: "" },
          judge: (answer) => {
            const near = (answer?.result?.nodes ?? []).filter((node) => ["text", "raw-text"].includes(node.kind) && node.opacity > 0 && node.seen > 0 && node.rect && (node.rect.x < margins || node.rect.x + node.rect.width > width - margins))
            return near.length ? near.map((node) => `"${node.name}" is drawn ${Math.round(Math.min(node.rect.x, width - node.rect.x - node.rect.width))} pt from the window's edge`).join(", ") : undefined
          },
        })
      }
      if (tall)
        out.push({
          request: { op: "query", ...selector(tall.of) },
          judge: (answer) => {
            const heights = (answer?.result?.nodes ?? []).map((node) => node.rect?.height).filter((height) => height !== undefined)
            return !heights.some((height) => Math.abs(height - tall.height) <= 1) && `${named(tall.of)} is ${heights.length ? heights.map((height) => `${height} pt`).join(" and ") : "not drawn"} tall, not ${tall.height} pt`
          },
        })
      // Every text in sight drawn whole, or the one named: none cut short with "…".
      if (whole)
        out.push({
          request: { op: "query", ...(whole === true ? { text: "" } : selector(whole)) },
          judge: (answer) => {
            const cut = (answer?.result?.nodes ?? []).filter((node) => node.cut && node.opacity > 0 && node.seen > 0)
            return cut.length ? cut.map((node) => `"${node.name}" is cut short`).join(", ") : undefined
          },
        })
      // Nothing of what's named or inside it clipped away, as a badge that overhangs its icon isn't cut by what's around.
      if (unclipped)
        out.push({
          request: { op: "query", ...selector(unclipped), descendants: true },
          judge: (answer) => {
            const nodes = answer?.result?.nodes ?? []
            if (!nodes.length) return `${named(unclipped)} isn't drawn`
            const clipped = nodes.filter((node) => node.opacity > 0 && node.seen < 0.99)
            return clipped.length ? clipped.map((node) => `${node.name ?? node.kind} (${node.rect ? rect(node.rect) : "nowhere"}) is clipped, ${Math.round(node.seen * 100)}% of it is in the window`).join(", ") : undefined
          },
        })
      // What's named is in the middle of the window across, within a point, as a lone column or an empty page's words are.
      if (centred) {
        let width
        out.push({ request: { op: "window" }, judge: (answer) => void (width = answer?.result?.width) })
        out.push({
          request: { op: "query", ...selector(centred) },
          judge: (answer) => {
            const rect = answer?.result?.nodes?.[0]?.rect
            if (!rect) return `${named(centred)} isn't drawn`
            const off = rect.x + rect.width / 2 - width / 2
            return Math.abs(off) > 1 && `${named(centred)}'s middle is ${Math.round(off)} pt from the window's`
          },
        })
      }
      // Nothing named so is wider than `most` points, as a form's fields and a page's column stop at a readable width.
      if (widest)
        out.push({
          request: { op: "query", ...selector(widest.of) },
          judge: (answer) => {
            const wide = (answer?.result?.nodes ?? []).filter((node) => node.rect?.width > widest.most + 0.5)
            return wide.length ? `${named(widest.of)} is ${wide.map((node) => Math.round(node.rect.width)).join(" and ")} pt wide, over ${widest.most}` : undefined
          },
        })
      // What's named stands off what's around it: the most contrast (WCAG's ratio) between the colour 3 pt outside its
      // start, at its middle, and any along its first 4 pt in, where a control's ring or a card's edge and fill are.
      if (contrast) {
        let found, points
        out.push({ request: { op: "window" }, judge: (answer) => void (points = answer?.result?.width) })
        out.push({ request: { op: "query", ...selector(contrast.of) }, judge: (answer) => void (found = answer?.result?.nodes?.[0]?.rect) })
        out.push({
          request: { op: "snap" },
          judge: (answer) => {
            if (!found) return `${named(contrast.of)} isn't drawn`
            const { data, width } = PNG.sync.read(Buffer.from(answer.result.png, "base64"))
            const scale = width / points
            const luminance = (x, y) => {
              const at = (Math.round(y * scale) * width + Math.round(x * scale)) * 4
              const linear = (c) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4)
              return 0.2126 * linear(data[at]) + 0.7152 * linear(data[at + 1]) + 0.0722 * linear(data[at + 2])
            }
            const y = found.y + found.height / 2
            const outside = luminance(Math.max(0, found.x - 3), y)
            let most = 1
            for (let x = found.x; x < found.x + 4; x += 1 / scale) {
              const inside = luminance(x, y)
              most = Math.max(most, (Math.max(inside, outside) + 0.05) / (Math.min(inside, outside) + 0.05))
            }
            return most < contrast.least && `${named(contrast.of)} stands off what's around it at ${most.toFixed(2)}:1, under ${contrast.least}:1`
          },
        })
      }
      // What's over the page dims it: the window's corner, which no layer over it covers, is darker than it was.
      if (dimmed)
        out.push({
          request: { op: "snap" },
          judge: (answer) => {
            const corner = (png) => {
              const { data, width } = PNG.sync.read(Buffer.from(png, "base64"))
              let sum = 0
              for (let y = 0; y < 8; y++) for (let x = 0; x < 8; x++) for (let c = 0; c < 3; c++) sum += data[(y * width + x) * 4 + c]
              return sum / (8 * 8 * 3)
            }
            return answer?.result?.png && before && corner(answer.result.png) > corner(before) * 0.9 && `the page behind it isn't dimmed: its corner is as light as before (${Math.round(corner(answer.result.png))} of ${Math.round(corner(before))})`
          },
        })
      if (blur !== undefined) out.push({ request: { op: "profile", at: [0], snap: false }, judge: (answer) => (answer?.result?.frames?.[0]?.work?.backdrops > 0) !== blur && (blur ? "no blur is drawn" : "a blur is drawn") })
      // What a phone's system was asked to show of a finger's selection: its menu for text with these items (or none,
      // `textMenu: false`), drawn inside the safe area; so many handles; its magnifier or not.
      if (textMenu !== undefined || handles !== undefined || magnifier !== undefined)
        out.push({
          request: { op: "text_selection" },
          judge: (answer) => {
            const { menu, handles: drawn = [], magnifier: shown, safeArea } = answer?.result ?? {}
            const wrong = []
            if (textMenu === false && menu) wrong.push(`the menu for text shows ${menu.items.join(", ")}`)
            if (textMenu && !menu) wrong.push("no menu for text shows")
            if (textMenu?.items && menu && menu.items.join() !== textMenu.items.join()) wrong.push(`the menu for text has ${menu.items.join(", ")}`)
            if (textMenu && menu && menu.rect.y < safeArea.top) wrong.push(`the menu for text is drawn at y ${menu.rect.y}, over the safe area's top ${safeArea.top}`)
            const over = (a, b) => a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height
            if (menu && drawn.some((handle) => over(menu.rect, handle.rect))) wrong.push("the menu for text covers a handle")
            if (handles !== undefined && drawn.length !== handles) wrong.push(`${drawn.length} handles are drawn`)
            if (magnifier !== undefined && Boolean(shown) !== magnifier) wrong.push(shown ? "the magnifier shows" : "no magnifier shows")
            return wrong.join("; ") || undefined
          },
        })
    }
    for (const one of out.slice(from)) one.step = `step ${at + 1}`
  }
  // Its end: at rest, nothing drawn going, every picture drawn, none later than PICTURE_MS after it was asked for,
  // and each layer open drawn whole in the window.
  const end = out.length
  out.push({ request: { op: "clock", step: 2000 } })
  out.push({ request: { op: "settle", deadline: 1000 }, judge: (answer) => !answer?.result?.settled && `it hadn't settled: ${JSON.stringify(answer?.result?.pending)}` })
  const atRest = ({ leaving, moving, blank, picture_ms }) =>
    leaving ? `${leaving} nodes taken out are still drawn` : moving ? `${moving} nodes are still moving` : blank ? `${blank} pictures are drawn empty` : picture_ms > PICTURE_MS ? `a picture took ${picture_ms} ms to show, where ${PICTURE_MS} is the most` : undefined
  out.push({ request: { op: "window" }, judge: (answer) => answer?.result && atRest(answer.result) })
  out.push({ request: { op: "fresh" }, judge: (answer) => unlike(answer) })
  for (const role of LAYERS)
    out.push({ request: { op: "wait_for", role, state: "shown", timeout: 0 }, judge: (answer) => answer?.result?.node && !answer.result.met && `the ${role} open isn't shown: ${why(answer)}` })
  for (const one of out.slice(end)) one.step = "at its end"
  return out
}

/// A page that threw while drawing shows `render`'s Problem, "Can't show this page", with the error under it, and nothing
/// to press but Try Again, so what a check or the Tab walk finds there says nothing of the page. The request that finds it
/// at a run's end, and from its answer, why the run failed, if it did.
const crash = { op: "query" }
const crashed = (answer) => {
  const nodes = answer?.result?.nodes ?? []
  const heading = nodes.find((node) => node.name === messages.cantShowPage)
  if (!heading) return
  const said = (node) => node?.name ?? (node?.children ?? []).map((id) => said(nodes.find((other) => other.node === id))).join("")
  const siblings = nodes.find((node) => node.children?.includes(heading.node))?.children ?? []
  const error = said(nodes.find((node) => node.node === siblings[siblings.indexOf(heading.node) + 1]))
  return `the page crashed, showing "${messages.cantShowPage}": ${error || "no error"}`
}

/// Plays each check of `scene`, once as it is and with each seed. → each check's name and how each run went.
async function check(built, scene, folder) {
  const at = pageFolder(built, scene.page, folder, scene.uses)
  const taps = scene.taps.flatMap((tap) => [pressed(tap), { op: "clock", step: 1000 }])
  const played = Object.entries(scene.checks).flatMap(([name, steps]) =>
    [0, ...Array.from({ length: seeds }, (_, n) => n + 1), ...(scene.seeds ?? [])].map((seed) => ({ name, steps, seed })),
  )
  const results = []
  for (let from = 0; from < played.length; from += 4)
    results.push(
      ...(await Promise.all(
        played.slice(from, from + 4).map(async ({ name, steps, seed }) => {
          const ran = requests(steps, scene.from)
          const own = join(folder, name.replace(/[^\w.-]+/g, "-"), String(seed))
          mkdirSync(own, { recursive: true })
          const done = await run(at, own, scene, [{ op: "clock", step: 1000 }, ...taps, ...ran.map(({ request }) => request), crash], seed)
          const answers = done.answers.slice(1 + taps.length)
          const failed = done.failed
            ? `the harness failed: ${done.said.split("\n").at(-1)}`
            : crashed(answers[ran.length]) ?? ran.map(({ judge, request, step }, n) => (answers[n]?.error ? `${step} (${request.op}): ${answers[n].error}` : judge && judge(answers[n]) ? `${step}: ${judge(answers[n])}` : undefined)).find(Boolean)
          return { name, seed, failed }
        }),
      )),
    )
  return results
}

/// How many times a scene's keyboard is moved on by Tab, at most, as `walk` goes round it.
const STOPS = 40

/// The keyboard walked round a scene by Tab, as a person at a computer moves it: what it comes to must draw otherwise
/// where it is than it did before it came or after it left, as the OS's focus ring and the web's `:focus-visible` show
/// where it is. What scrolls into sight as it comes is judged by the picture it isn't moved in. → why it failed, or
/// nothing.
async function walk(built, scene, folder) {
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, scene.page, folder, scene.uses)
  const taps = scene.taps.flatMap((tap) => [pressed(tap), { op: "clock", step: 1000 }])
  const look = [{ op: "query" }, { op: "snap" }]
  // Each picture two of the caret's blinks after the keyboard came, where a field's caret shows.
  const asked = [{ op: "clock", step: 1000 }, ...taps, ...look, ...Array.from({ length: STOPS }, () => [{ op: "key", key: "Tab" }, { op: "clock", step: 2400 }, ...look]).flat()]
  const done = await run(at, folder, scene, asked)
  if (done.failed) return `the harness failed: ${done.said.split("\n").at(-1)}`
  const answers = done.answers.slice(1 + taps.length)
  if (crashed(answers[0])) return crashed(answers[0])
  // Each stop: what had the keyboard, where it was drawn, and the picture.
  const stops = []
  for (let n = 0; n <= STOPS; n++) {
    const [query, snap] = [answers[n * 4], answers[n * 4 + 1]]
    const node = query?.result?.nodes?.find((node) => node.focused)
    if (!snap?.result?.png) return `step ${n}: ${snap?.error ?? query?.error ?? "no picture"}`
    stops.push({ node, picture: PNG.sync.read(Buffer.from(snap.result.png, "base64")) })
    // Round to where it started.
    if (n > 1 && node && node.node === stops[1].node?.node) break
  }
  // Where a node is, in points, is in the picture's device pixels (a Surface's two a point).
  const same = (one, other, rect) => {
    const scale = one.width / scene.size[0]
    const [x0, y0] = [Math.max(0, Math.floor((rect.x - 4) * scale)), Math.max(0, Math.floor((rect.y - 4) * scale))]
    const [x1, y1] = [Math.min(one.width, Math.ceil((rect.x + rect.width + 4) * scale)), Math.min(one.height, Math.ceil((rect.y + rect.height + 4) * scale))]
    for (let y = y0; y < y1; y++) if (one.data.compare(other.data, (y * one.width + x0) * 4, (y * one.width + x1) * 4, (y * one.width + x0) * 4, (y * one.width + x1) * 4) !== 0) return false
    return true
  }
  const unseen = []
  for (const [n, { node, picture }] of stops.entries()) {
    if (!n || !node?.rect || !(node.opacity > 0) || !(node.seen > 0)) continue
    const around = [stops[n - 1], stops[n + 1]].filter((other) => other && other.node?.node !== node.node)
    if (around.some((other) => same(picture, other.picture, node.rect))) unseen.push(`${node.role ?? node.kind} "${node.name ?? ""}"`)
  }
  return unseen.length ? `Tab's keyboard focus draws nothing on ${[...new Set(unseen)].join(", ")}` : undefined
}

/// Why a frame drawn on what the window kept from frames before isn't the one drawn afresh (`fresh`), if it isn't: the
/// two pictures kept beside `folder`, as `kept.png` and `afresh.png`, where given one.
const unlike = (answer, folder) => {
  if (answer?.error) return answer.error
  const { differ, of, box, kept, afresh } = answer?.result ?? {}
  if (!differ) return
  if (folder && kept) for (const [name, png] of [["kept", kept], ["afresh", afresh]]) writeFileSync(join(folder, `${name}.png`), Buffer.from(png, "base64"))
  return `the frame drawn on what it kept differs from one drawn afresh in ${differ} of ${of} pixels${box ? `, from ${box.x},${box.y} ${box.width}×${box.height} px` : ""}`
}

/// A scene drawn the same however it came to be, as a frame drawn on what was kept from frames before must be, by what
/// changes under a page that isn't its own: opened and at rest, then after the app's been in the background and back
/// (`away`, which on an iPhone draws it in the other appearance and back for the app switcher, and puts a Mac's launcher
/// away as it takes the keyboard, so it's summoned again, as its person does), the system's appearance switched, and on
/// a computer its window made smaller. → why it failed, or nothing.
async function agrees(built, scene, folder) {
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, scene.page, folder, scene.uses)
  const taps = scene.taps.flatMap((tap) => [pressed(tap), { op: "clock", step: 1000 }])
  const rest = { op: "clock", step: 2000 }
  const changes = [
    ["at rest", []],
    ["back from the background", [{ op: "away", ms: 1000 }, ...(panelled(scene) ? [{ op: "summon" }] : [])]],
    [`switched to ${dark ? "Light" : "Dark"}`, [{ op: "appearance", dark: !dark }]],
    ...(scene.touch ? [] : [["made smaller", [{ op: "resize", width: scene.size[0] - 120, height: scene.size[1] - 80 }]]]),
  ]
  const asked = [{ op: "clock", step: 1000 }, ...taps, ...changes.flatMap(([, ops]) => [...ops, rest, { op: "fresh" }]), crash]
  const done = await run(at, folder, scene, asked)
  if (done.failed) return `the harness failed: ${done.said.split("\n").at(-1)}`
  const answers = done.answers.slice(1 + taps.length)
  if (crashed(answers.at(-1))) return crashed(answers.at(-1))
  let n = 0
  for (const [what, ops] of changes) {
    n += ops.length + 1
    const own = join(folder, what.replace(/[^\w.-]+/g, "-"))
    mkdirSync(own, { recursive: true })
    const why = unlike(answers[n++], own)
    if (why) return `${what}: ${why} (${own})`
  }
}

/// Plays every check of `targets`' scenes. → each line it says, and how many failed.
async function checks(targets) {
  const said = []
  let failed = 0
  let passed = 0
  for (const target of targets) {
    const name = target.endsWith("core/ui/gallery") ? "gallery" : target.split("/").at(-1)
    const built = join(out, name, ".build")
    const unbuilt = build(target, built)
    if (unbuilt) {
      said.push(`${name}: ${unbuilt}`)
      failed++
      continue
    }
    // Every scene is drawn the same however it came to be, each on a computer walked by Tab too, and those with checks
    // played.
    const scenes = (await scenesOf(target, built, join(out, name, ".scenes"), true)).filter((scene) => !only || scene.name.toLowerCase().includes(only))
    for (const scene of scenes) {
      // A trace a check plays is beside its scenes: the gallery's, or an app's client.
      scene.from = target.endsWith("core/ui/gallery") ? target : join(target, "client")
      const own = join(out, name, scene.name.replace(/[^\w.-]+/g, "-"))
      const results = scene.checks ? await check(built, scene, own) : []
      if (!scene.touch) results.push({ name: "Tab shows where the keyboard is", seed: 0, failed: await walk(built, scene, join(own, "tab")) })
      results.push({ name: "Drawn the same however it came to be", seed: 0, failed: await agrees(built, scene, join(own, "fresh")) })
      for (const [check, runs] of Object.entries(Object.groupBy(results, (result) => result.name))) {
        const bad = runs.filter((run) => run.failed)
        bad.length ? failed++ : passed++
        said.push(`${bad.length ? "✗" : "✓"} ${name} · ${scene.name} · ${check}${bad.length ? "" : ` (${runs.length} runs)`}`)
        for (const run of bad) said.push(`    ${run.seed ? `seed ${run.seed}` : "as it is"}: ${run.failed}`)
      }
    }
  }
  said.push(`${passed} checks passed, ${failed} failed`)
  return { said, failed }
}

/// How much of two pictures differs, from 0 to 1, where they're the same size, with where in `diff`.
function differs(one, other, diff) {
  const [a, b] = [one, other].map((file) => PNG.sync.read(readFileSync(file)))
  if (a.width !== b.width || a.height !== b.height) return undefined
  const marked = new PNG({ width: a.width, height: a.height })
  const changed = pixelmatch(a.data, b.data, marked.data, a.width, a.height, { threshold: 0.1 })
  writeFileSync(diff, PNG.sync.write(marked))
  return changed / (a.width * a.height)
}

/// The contact sheet: every motion's filmstrip and curve, before beside after where there's a before.
function contactSheet(rows) {
  const cell = (folder) => `<figure><img src="${folder}/filmstrip.png"><img src="${folder}/curve.svg"></figure>`
  const body = rows
    .map((row) => `<section><h2>${row.scene} · ${row.motion} · ${row.hz} Hz</h2>${row.before ? `<div class="pair"><div><h3>Before</h3>${cell(relative(out, row.before))}</div><div><h3>After</h3>${cell(row.folder)}</div></div>` : cell(row.folder)}</section>`)
    .join("\n")
  writeFileSync(join(out, "index.html"), `<!doctype html><meta charset="utf-8"><title>Motion</title>
<style>body{font-family:system-ui;margin:16px;background:#f6f6f7}h2{font-size:15px;margin:24px 0 8px}h3{font-size:12px;margin:0 0 4px;color:#666}figure{margin:0;display:flex;flex-direction:column;gap:8px}img{max-width:100%;background:white;border-radius:6px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}</style>
<h1>Motion</h1>${body}`)
}

/// Every motion played, by its folder, for the contact sheet: a round that plays some keeps the rest.
const rows = new Map()

/// Plays every motion of `targets`. → each line it says.
async function round(targets) {
  const said = []
  for (const target of targets) {
    const name = basename(target)
    const built = join(out, name, ".build")
    const unbuilt = build(target, built)
    if (unbuilt) {
      said.push(`${name}: ${unbuilt}`)
      continue
    }
    const scenes = (await scenesOf(target, built, join(out, name, ".scenes"))).filter((scene) => !only || scene.name.toLowerCase().includes(only))
    // A trace a motion plays is beside its scenes.
    for (const scene of scenes) scene.from = target.endsWith("core/ui/gallery") ? target : join(target, "client")
    // Each motion at each rate on a harness of its own, a few at once.
    const jobs = scenes.flatMap((scene) => Object.entries(scene.motions).flatMap(([motion, played]) => rates.map((hz) => ({ scene, motion, played, hz }))))
    const results = []
    // One at a time on a device, whose GPU each frame's time is of.
    const together = ssh ? 1 : 4
    for (let at = 0; at < jobs.length; at += together)
      results.push(...(await Promise.all(jobs.slice(at, at + together).map(({ scene, motion, played, hz }) => play(built, scene, motion, played, hz, join(out, name, scene.name.replace(/[^\w.-]+/g, "-"), motion.replace(/[^\w.-]+/g, "-"), `${hz}hz`)).then((result) => ({ scene, motion, ...result }))))))
    let heading
    for (const result of results) {
      const title = `${name} · ${result.scene.name} · ${result.motion}`
      if (title !== heading) said.push((heading = title))
      said.push(result.said)
      if (result.row) rows.set(result.row.folder, result.row)
    }
  }
  contactSheet([...rows.values()])
  said.push(join(out, "index.html"))
  return said
}

// Run, rather than imported for what it shares (calibrate.js).
if (import.meta.main) {
  if (stale(ssh ? undefined : harness)) throw new Error(stale(ssh ? undefined : harness))
  if (!existsSync(harness)) throw new Error("Build the native harness first: `cargo build -p renderer --example screenshot --features fake`")
  // `--watch` plays what it builds again as it changes.
  if (!flag("--watch")) harness = copied(ssh ? undefined : harness, out) ?? harness
  // A debug build's CPU is several times a shipped one's.
  if (!flag("--release") && !ssh) console.log("Its CPU times are a debug build's: --release for a shipped build's")
  if (!hostScore && !ssh) console.log(`This CPU (${cpuName.trim()}) has no Geekbench score in devices.json's hosts, so devices are judged by their GPUs alone`)
  const started = performance.now()
  if (checking) {
    const { said, failed } = await checks(folders.length ? folders : everything())
    console.log(said.join("\n"), `\nin ${Math.round(performance.now() - started)} ms`)
    process.exit(failed ? 1 : 0)
  }
  console.log((await round(folders.length ? folders : everything())).join("\n"), `\nin ${Math.round(performance.now() - started)} ms`)

  if (flag("--watch")) {
    // What changed, played again: the UI library's own source rebuilds it first and plays everything.
    const watched = folders.length ? folders : everything()
    let pending
    const changed = new Set()
    const again = async () => {
      const touched = [...changed]
      changed.clear()
      const started = performance.now()
      const library = touched.some((file) => file.startsWith(here("src")))
      if (library) Bun.spawnSync(["bun", "build.js"], { cwd: here(".") })
      const targets = library ? watched : watched.filter((target) => touched.some((file) => file.startsWith(target)))
      console.log((await round(targets)).join("\n"), `\nin ${Math.round(performance.now() - started)} ms`)
    }
    for (const folder of [here("src"), ...watched.map((target) => (target.endsWith("core/ui/gallery") ? target : join(target, "client")))])
      watch(folder, { recursive: true }, (_, file) => {
        if (!file || file.includes("node_modules") || file.includes("dist/")) return
        changed.add(join(folder, file))
        clearTimeout(pending)
        pending = setTimeout(again, 100)
      })
    console.log("Watching for changes")
  }
}
