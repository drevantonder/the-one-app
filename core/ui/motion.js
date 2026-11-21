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
import { existsSync, readdirSync, readFileSync, watch } from "node:fs"
import { join } from "node:path"
import { checks } from "./motion/checks.js"
import { checking, flag, folders, harness, here, out, root, setHarness, ssh } from "./motion/options.js"
import { copied, stale } from "./motion/pages.js"
import { round } from "./motion/play.js"
import { cpuName, hostScore } from "./motion/tiers.js"

// What scenes.js and calibrate.js import from here.
export { capped, env, FACES, stopped } from "./motion/options.js"
export { moves, selector } from "./motion/gesture.js"
export { COMPACT, IPHONE, phone, runThere, scenesOf } from "./motion/harness.js"
export { build, COMPONENTS, copied, library, pageFolder, stale, usedApp } from "./motion/pages.js"

/// Every folder with motions, or with `--checks` every folder with scenes: the gallery, and each app's.
const everything = () => [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")) && (checking || readFileSync(join(app, "client/scenes.json"), "utf8").includes('"motions"')))]

// Run, rather than imported for what it shares (calibrate.js).
if (import.meta.main) {
  if (stale(ssh ? undefined : harness)) throw new Error(stale(ssh ? undefined : harness))
  if (!existsSync(harness)) throw new Error("Build the native harness first: `cargo build -p renderer --example screenshot --features fake`")
  // `--watch` plays what it builds again as it changes.
  if (!flag("--watch")) setHarness(copied(ssh ? undefined : harness, out) ?? harness)
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
