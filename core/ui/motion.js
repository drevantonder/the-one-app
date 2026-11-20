// Plays every scene's gestures (its `motions`) on a native host's headless harness, frame by frame, on its paused
// clock: `bun motion.js [<folder>…] [--scene <words>] [--hz 60,120] [--out <dir>] [--against <dir>] [--dark]
// [--release] [--watch]` in core/ui, after `bun build.js` and `cargo build -p renderer --example screenshot`
// (`--release` too for `--release`, whose costs are a shipped build's).
//
// A folder is the gallery (core/ui/gallery, its scenes in `*.scenes.tsx`) or an app's (apps/<app>, in
// `client/scenes.json`), as `scenes.js` takes them; with none, the gallery and every app with a motion. A motion is
// `{ from, gesture, follow }` (the `Motion` type in src/types/ui.d.ts): the press goes down on what `from` names, moves
// as `gesture` says, and what `follow` names is watched move, with what scrolls under the press. Each is played at
// each rate of `--hz` (a display's frames a second: 60 and 120 unless said) by the inspector's `replay`, through the
// gesture and spring code a window runs, and for each it keeps, in `--out`:
//
// - `filmstrip.png`: eight of its frames, from the press to where it comes to rest;
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
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync, watch, writeFileSync } from "node:fs"
import { join, relative, resolve } from "node:path"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"

const here = (path) => new URL(path, import.meta.url).pathname
const root = here("../../")
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const out = resolve(option("--out") ?? join(process.env.TMPDIR ?? "/tmp", "motion"))
const against = option("--against") && resolve(option("--against"))
const rates = (option("--hz") ?? "60,120").split(",").map(Number)
const only = option("--scene")?.toLowerCase()
const dark = flag("--dark")
const folders = args.filter((arg, at) => !arg.startsWith("--") && !["--out", "--against", "--hz", "--scene"].includes(args[at - 1])).map((folder) => resolve(folder))
const harness = join(root, `target/${flag("--release") ? "release" : "debug"}/examples/screenshot`)
const env = { ...process.env, VK_ICD_FILENAMES: process.env.VK_ICD_FILENAMES ?? "/usr/share/vulkan/icd.d/lvp_icd.json" }


/// Every folder with motions: the gallery, and each app whose scenes have one.
const everything = () => [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")) && readFileSync(join(app, "client/scenes.json"), "utf8").includes('"motions"'))]

/// How a gesture moves, as the inspector's `replay` takes it: each stretch `{dx, dy, ms}`, and whether it lifts.
export function moves(gesture) {
  const ms = ([dx, dy], speed) => (Math.hypot(dx, dy) / speed) * 1000
  if (gesture.moves) return { moves: gesture.moves, lift: gesture.lift !== false }
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

/// A built page's folder as the harness loads it: its `index.html` with what it loads beside it.
export function pageFolder(built, page, folder) {
  const at = join(folder, "page")
  rmSync(at, { recursive: true, force: true })
  mkdirSync(at, { recursive: true })
  writeFileSync(join(at, "index.html"), readFileSync(join(built, page), "utf8").replace(/(src|href)="(\.\.\/)+/g, '$1="').replace(/(src|href)="\.\//g, '$1="'))
  symlinkSync(join(built, "assets"), join(at, "assets"))
  if (existsSync(join(built, "styles.json"))) cpSync(join(built, "styles.json"), join(at, "styles.json"))
  return at
}

/// The harness run on a scene, with `requests`: each answer, and its stderr. → `{ answers, said }`.
async function run(at, folder, scene, requests) {
  const answers = { ...scene.answers, "ui/initialize": scene.answers?.["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  const lines = requests.map((request, id) => JSON.stringify({ v: 1, id: id + 1, ...request })).join("\n")
  const harnessed = Bun.spawn([harness, at, join(folder, "end.png"), join(folder, "end.json"), "--ui", here("dist/client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", "1", "--requests", "-", ...(dark ? ["--dark"] : []), ...(scene.touch ? ["--touch"] : [])], { stdin: new TextEncoder().encode(lines), stdout: "pipe", stderr: "pipe", env })
  const [stdout, stderr] = await Promise.all([new Response(harnessed.stdout).text(), new Response(harnessed.stderr).text(), harnessed.exited])
  return { answers: stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line)), said: stderr.trim(), failed: harnessed.exitCode !== 0 }
}

/// A folder's scenes that have motions, or with `all`, every one an app's page draws, each with its page, link,
/// answers, taps, size and touch.
export async function scenesOf(target, built, folder, all = false) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const all = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(all[scene.like]), taps: undefined, ...scene, answers: { ...whole(all[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(all)
      .map(([name, scene]) => ({ name, ...whole(scene) }))
      .filter((scene) => (all || scene.motions) && !scene.component && !scene.uses)
      .map(({ name, page = "client/index.html", answers = {}, taps = [], size = [390, 844], touch = false, motions }) => ({ name, page: page.split("#")[0], link: page.split("#")[1], answers, taps, size, touch, motions }))
  }
  // The gallery lists its own, drawn at `#scenes`.
  const listing = { link: "scenes", size: [390, 844], answers: {} }
  mkdirSync(folder, { recursive: true })
  const at = pageFolder(built, "index.html", folder)
  const listed = await run(at, folder, listing, [])
  const text = JSON.stringify(JSON.parse(readFileSync(join(folder, "end.json"), "utf8"))).match(/"name":"(\[.*?\])"/)?.[1]
  if (!text) throw new Error(`The gallery listed no scenes: ${listed.said}`)
  return JSON.parse(JSON.parse(`"${text}"`))
    .filter((scene) => all || scene.motions)
    .map(({ name, taps = [], size = [390, 844], touch = false, motions }) => ({ name, page: "index.html", link: name, answers: {}, taps, size, touch, motions }))
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
/// or none before the lift, or none where iOS gives nothing to compare (a stretch past an end springing back).
function ios(frames, way, values, stretches) {
  const lift = stretches.reduce((sum, { ms }) => sum + ms, 0)
  const last = stretches.at(-1)
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
  const over = frames.filter((frame) => frame.paint_ms > budget)
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

/// What each tier with `hz` frames a second would take for each of `frames`, played at one device pixel a point: its
/// CPU's time for the frame and its JS's since the frame before (this machine's, as its Geekbench score is to the
/// tier's) and its GPU's (what the device
/// measured takes for the frame's work at the tier's pixels, as its GPU's score is to the tier's), the frame dropped
/// where either is longer than the tier's frame. → each tier's name, the frames dropped of those drawn, the worst and why.
export function judge(frames, hz) {
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
          // Its JS runs on a thread of its own beside the one that paints, and a frame waits for whichever's longer.
          const [cpuMs, jsMs] = hostScore ? [frame.paint_ms, frame.script_ms ?? 0].map((ms) => (ms * hostScore) / tier.cpu) : [0, 0]
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
  const at = pageFolder(built, scene.page, folder)
  const { moves: stretches, lift } = moves(motion.gesture)
  const taps = scene.taps.flatMap((tap) => [tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...selector(tap) }, { op: "clock", step: 1000 }])
  const replay = { op: "replay", ...selector(motion.from), moves: stretches, lift, hz, snap: true, ...(motion.follow ? { track: selector(motion.follow) } : {}) }
  const started = performance.now()
  const ran = await run(at, folder, scene, [{ op: "clock", step: 1000 }, ...taps, replay])
  const answer = ran.answers.at(-1)
  if (ran.failed || !answer?.result) return { said: `  ${hz} Hz: ${answer?.error ?? ran.said.split("\n").at(-1)}` }
  const { frames, budget_ms: budget, over } = answer.result
  const [way, values] = travel(frames)
  const kept = frames.map(({ png, width, height, ...frame }) => frame)
  const beforeAt = against && join(against, relative(out, folder))
  const before = beforeAt && existsSync(join(beforeAt, "frames.json")) ? JSON.parse(readFileSync(join(beforeAt, "frames.json"), "utf8")) : undefined
  const beforeTravel = before && { frames: before.frames, values: travel(before.frames)[1] }
  writeFileSync(join(folder, "frames.json"), JSON.stringify({ hz, budget, way, frames: kept }))
  const apple = ios(frames, way, values, stretches)
  writeFileSync(join(folder, "curve.svg"), curve(frames, way, values, stretches, budget, beforeTravel, apple))
  filmstrip(frames, join(folder, "filmstrip.png"))
  const worst = Math.max(...frames.map((frame) => frame.paint_ms))
  const script = Math.max(...frames.map((frame) => frame.script_ms ?? 0))
  const rest = frames.at(-1).at
  const said = [`  ${hz} Hz: ${way} ${Math.round(Math.max(...values.map(Math.abs)))} pt, at rest after ${Math.round(rest)} ms (${frames.length} frames), ${over} over its ${budget} ms budget, the worst ${worst} ms painting and ${script} ms of JS, in ${Math.round(performance.now() - started)} ms`]
  if (apple) {
    const compared = frames.flatMap((frame, n) => (apple[n] === undefined ? [] : [Math.abs(values[n] - apple[n])]))
    said.push(`  ${hz} Hz against iOS (${way.startsWith("scrolled") ? "UIScrollView's deceleration" : "SwiftUI's .spring"}, from the lift): at most ${Math.max(...compared).toFixed(1)} pt apart`)
  }
  const judged = judge(frames, hz)
  if (judged.length)
    said.push(`  ${hz} Hz on devices: ${judged.map(({ tier, dropped, drawn, worst }) => `${tier} drops ${dropped} of ${drawn} frames (the worst ${worst.ms.toFixed(1)} ms, its ${worst.by})`).join("; ")}`)
  if (before) {
    const diff = existsSync(join(beforeAt, "filmstrip.png")) && differs(join(beforeAt, "filmstrip.png"), join(folder, "filmstrip.png"), join(folder, "changed.png"))
    said.push(`  ${hz} Hz against before: curves at most ${apart(frames, values, beforeTravel).toFixed(1)} pt apart, at rest after ${Math.round(before.frames.at(-1).at)} ms then, filmstrips ${diff === undefined ? "not the same size" : `${(diff * 100).toFixed(1)}% different`}`)
  }
  return { said: said.join("\n"), row: { scene: scene.name, motion: name, hz, folder: relative(out, folder), before: before && beforeAt } }
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
    const name = target.endsWith("core/ui/gallery") ? "gallery" : target.split("/").at(-1)
    const built = join(out, name, ".build")
    const make = Bun.spawnSync(["bun", here("dist/build/app.js"), target, built])
    if (make.exitCode !== 0) {
      said.push(`${name}: didn't build\n${make.stderr.toString().trim()}`)
      continue
    }
    const scenes = (await scenesOf(target, built, join(out, name, ".scenes"))).filter((scene) => !only || scene.name.toLowerCase().includes(only))
    // Each motion at each rate on a harness of its own, a few at once.
    const jobs = scenes.flatMap((scene) => Object.entries(scene.motions).flatMap(([motion, played]) => rates.map((hz) => ({ scene, motion, played, hz }))))
    const results = []
    for (let at = 0; at < jobs.length; at += 4)
      results.push(...(await Promise.all(jobs.slice(at, at + 4).map(({ scene, motion, played, hz }) => play(built, scene, motion, played, hz, join(out, name, scene.name.replace(/[^\w.-]+/g, "-"), motion.replace(/[^\w.-]+/g, "-"), `${hz}hz`)).then((result) => ({ scene, motion, ...result }))))))
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
  if (!existsSync(here("dist/build/app.js"))) throw new Error("Build the UI library first: `bun build.js` in core/ui")
  if (!existsSync(harness)) throw new Error("Build the native harness first: `cargo build -p renderer --example screenshot`")
  // A debug build's CPU is several times a shipped one's.
  if (!flag("--release")) console.log("Its CPU times are a debug build's: --release for a shipped build's")
  if (!hostScore) console.log(`This CPU (${cpuName.trim()}) has no Geekbench score in devices.json's hosts, so devices are judged by their GPUs alone`)
  const started = performance.now()
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
