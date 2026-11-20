// Measures what a frame's work costs a real device, to calibrate the device tiers motion.js judges by (devices.json):
// `bun calibrate.js --ssh <host> --harness <its build of the harness> [--name <device>] [--out <dir>]` in core/ui, after
// `bun build.js`, with a harness built for that host (`cargo xwin build --release -p renderer --example screenshot --features fake
// --target x86_64-pc-windows-msvc` for a Windows one, which draws on its own GPU over ssh with Direct3D 12).
//
// It draws every scene of the gallery there, the first frames of each as it opens and every frame of each of its
// motions at 60 Hz, each waited for on its GPU, as the harness times its passes there (`gpu_ms`), at two device pixels a point, as a phone's or a
// laptop's at 4K is. Each frame's work (`work`: the pixels of each kind of thing drawn, the pixels copied, the bytes
// uploaded, and its draw calls) is the same on any device; how long its GPU took is that device's. It fits the cost of
// each (non-negative least squares) on half the scenes and says how far off that fit is on the other half, the
// calibration error, then fits on them all, and keeps the device's costs in devices.json's `measured`, with its CPU's
// time for the same frames (`paint_ms`, a frame's styling, layout and scene).
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { basename, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { FACES, moves, pageFolder, scenesOf, selector } from "./motion.js"

const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const args = process.argv.slice(2)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const host = option("--ssh")
const harness = option("--harness") && resolve(option("--harness"))
const name = option("--name") ?? host
const out = resolve(option("--out") ?? join(process.env.TMPDIR ?? "/tmp", "calibrate"))
if (!host || !harness) throw new Error("Say where: --ssh <host> --harness <the harness built for it>")

/// The work a frame did, as the fit takes it: millions of pixels of each kind, its draw calls, megabytes uploaded.
export const FEATURES = ["quads", "shadows", "sprites", "backdrops", "rasters", "copied", "draws", "uploaded"]
export const features = (frame) => [...["quads", "shadows", "sprites", "backdrops", "rasters", "copied"].map((kind) => frame.work[kind] / 1e6), frame.draws, frame.work.uploaded / 1e6]

/// Non-negative least squares, by coordinate descent: the costs `c` ≥ 0 and the constant `k` ≥ 0 for which each
/// `x · c + k` comes nearest each `y`.
export function fit(xs, ys) {
  const n = xs[0].length + 1
  const rows = xs.map((x) => [...x, 1])
  const c = new Array(n).fill(0)
  for (let round = 0; round < 2000; round++) {
    for (let j = 0; j < n; j++) {
      const [num, den] = rows.reduce(([num, den], row, i) => {
        const others = row.reduce((sum, v, k) => (k === j ? sum : sum + v * c[k]), 0)
        return [num + row[j] * (ys[i] - others), den + row[j] * row[j]]
      }, [0, 0])
      c[j] = den > 0 ? Math.max(0, num / den) : 0
    }
  }
  return { costs: Object.fromEntries(FEATURES.map((feature, j) => [feature, c[j]])), constant: c[n - 1] }
}

export const predict = ({ costs, constant }, frame) => features(frame).reduce((sum, v, j) => sum + v * costs[FEATURES[j]], constant)

const median = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]

// Every gallery scene, and its motions, in a folder of its own with what the harness is asked.
const gallery = here("gallery")
const batch = join(out, "batch")
rmSync(out, { recursive: true, force: true })
mkdirSync(batch, { recursive: true })
const built = join(out, "build")
const make = Bun.spawnSync(["bun", here("dist/build/app.js"), gallery, built])
if (make.exitCode !== 0) throw new Error(`The gallery didn't build: ${make.stderr}`)
const scenes = await scenesOf(gallery, built, join(out, "listing"), true)
const lines = []
scenes.forEach((scene, n) => {
  const folder = join(batch, `${n}`)
  const page = pageFolder(built, scene.page, join(out, "pages", `${n}`))
  cpSync(page, join(folder, "page"), { recursive: true, dereference: true })
  writeFileSync(join(folder, "answers.json"), JSON.stringify({ "ui/initialize": { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }))
  const taps = scene.taps.flatMap((tap) => [tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...selector(tap) }, { op: "clock", step: 1000 }])
  const played = Object.values(scene.motions ?? {}).map((motion) => ({ op: "replay", ...selector(motion.from), ...moves(motion.gesture), hz: 60, after: 600 }))
  const requests = [{ op: "clock", step: 1000 }, ...taps, { op: "profile", at: [0, 17, 33, 50], snap: false }, ...played]
  writeFileSync(join(folder, "requests.jsonl"), requests.map((request, id) => JSON.stringify({ v: 1, id: id + 1, ...request })).join("\n"))
  lines.push(`screenshot.exe ${n}\\page ${n}\\end.png ${n}\\end.json --ui ui --answers ${n}\\answers.json --size ${scene.size[0]}x${scene.size[1]} --scale 2 ${scene.touch ? "--touch " : ""}--requests ${n}\\requests.jsonl > ${n}\\answers.jsonl 2> ${n}\\said.txt`)
})
cpSync(here("dist/client"), join(batch, "ui"), { recursive: true, dereference: true })
cpSync(harness, join(batch, "screenshot.exe"))
for (const face of FACES) cpSync(face, join(batch, basename(face)))
writeFileSync(join(batch, "run.cmd"), ["@echo off", "cd /d %~dp0", ...lines].join("\r\n"))

// There, and back.
const started = performance.now()
const say = (command) => {
  const ran = Bun.spawnSync(command, { stdout: "pipe", stderr: "pipe" })
  if (ran.exitCode !== 0) throw new Error(`${command.join(" ")}: ${ran.stderr.toString().trim()}`)
  return ran.stdout.toString().trim()
}
const temp = say(["ssh", host, "echo %TEMP%"])
const remote = `${temp}\\one-calibrate`
say(["ssh", host, `if exist "${remote}" rmdir /s /q "${remote}"`])
say(["scp", "-rq", batch, `${host}:${remote.replaceAll("\\", "/")}`])
say(["ssh", host, `"${remote}\\run.cmd"`])
rmSync(join(out, "back"), { recursive: true, force: true })
say(["scp", "-rq", `${host}:${remote.replaceAll("\\", "/")}`, join(out, "back")])
say(["ssh", host, `rmdir /s /q "${remote}"`])
console.log(`${scenes.length} scenes drawn on ${name} in ${Math.round((performance.now() - started) / 1000)} s`)

// Each frame the GPU drew (one where nothing changed costs nothing, and isn't one), by its scene.
const frames = scenes.flatMap((scene, n) => {
  const answers = existsSync(join(out, "back", `${n}`, "answers.jsonl")) ? readFileSync(join(out, "back", `${n}`, "answers.jsonl"), "utf8").split("\n").filter(Boolean).map((line) => JSON.parse(line)) : []
  return answers.flatMap((answer) => answer.result?.frames ?? []).filter((frame) => frame.work && frame.gpu_ms !== undefined && frame.draws > 0).map((frame) => ({ ...frame, scene: n }))
})
if (frames.length < 20) throw new Error(`Only ${frames.length} frames came back: see ${join(out, "back")}`)
const xs = (some) => some.map(features)
const error = (model, some) => median(some.map((frame) => Math.abs(predict(model, frame) - frame.gpu_ms) / frame.gpu_ms))
const [even, odd] = [frames.filter((frame) => frame.scene % 2 === 0), frames.filter((frame) => frame.scene % 2 === 1)]
const held = (error(fit(xs(even), even.map((frame) => frame.gpu_ms)), odd) + error(fit(xs(odd), odd.map((frame) => frame.gpu_ms)), even)) / 2
const model = fit(xs(frames), frames.map((frame) => frame.gpu_ms))
const round = (n, places = 4) => Number(n.toPrecision(places))
const measured = {
  gpu: { costs: Object.fromEntries(Object.entries(model.costs).map(([feature, cost]) => [feature, round(cost)])), constant: round(model.constant), error: round(held, 2), frames: frames.length, scenes: scenes.length },
  paint_ms: round(median(frames.map((frame) => frame.paint_ms)), 3),
}
const devices = JSON.parse(readFileSync(here("devices.json"), "utf8"))
devices.measured[name] = { ...devices.measured[name], ...measured, on: new Date().toISOString().slice(0, 10) }
writeFileSync(here("devices.json"), `${JSON.stringify(devices, null, 2)}\n`)
console.log(`${frames.length} frames: ms each`, measured.gpu.costs, `+ ${measured.gpu.constant}; off by ${Math.round(held * 100)}% (median) on scenes it wasn't fitted on; its CPU's median frame ${measured.paint_ms} ms`)
