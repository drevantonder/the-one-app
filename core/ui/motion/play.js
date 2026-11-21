// A motion played on a scene, and a round of them over folders.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { basename, join, relative } from "node:path"
import { against, only, out, present, rates, ssh } from "./options.js"
import { build, pageFolder } from "./pages.js"
import { device, devices, hostScore, judge } from "./tiers.js"
import { presentThere, run, scenesOf } from "./harness.js"
import { finger, ios, moves, selector, travel } from "./gesture.js"
import { apart, contactSheet, curve, differs, filmstrip, rect } from "./output.js"


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
    const lines = shown.said.map((round, n) => `  ${hz} Hz presented, round ${n + 1}: ${round.frames} frames, p95 painting ${ms(round.paint_ms)}, drawing ${ms(round.draw_ms)} (waiting for an image ${ms(round.acquire_ms)}, submitting ${ms(round.submit_ms)}, presenting ${ms(round.present_ms)}), GPU ${ms(round.gpu_ms)}, render passes p95 ${round.passes} (most ${round.most_passes})`)
    if (!shown.said.length) lines.push(`  ${hz} Hz presented: nothing said: ${shown.stderr.trim().split("\n").at(-1)}`)
    const memory = shown.stderr.split("\n").map((line) => line.trim()).filter((line) => line.startsWith('{"memory"')).map((line) => JSON.parse(line).memory.gpu).at(-1)
    if (memory) lines.push(`  ${hz} Hz presented, the GPU at its end: ${memory.encoders} command encoders, ${((memory.reserved ?? 0) / 1e6).toFixed(1)} MB reserved (${((memory.allocated ?? 0) / 1e6).toFixed(1)} MB allocated)`)
    if (shown.failed) {
      process.exitCode = 1
      lines.push(`  ${hz} Hz presented FAILS: ${device.name}'s frames took longer than its ${(1000 / hz).toFixed(1)} ms refresh`)
    }
    return { said: lines.join("\n") }
  }
  // Where what it opens should first be drawn: what `opensFrom` names, before the press.
  const origin = motion.opensFrom && { op: "query", ...selector(motion.opensFrom) }
  const replay = { op: "replay", ...(motion.from ? selector(motion.from) : {}), ...(events ? { events } : { moves: stretches, lift }), hz, snap: true, ...(motion.follow ? { track: selector(motion.follow) } : {}), ...(motion.once ? { count: { ...selector(motion.once), includeHidden: true } } : {}), ...(motion.shows ? { shows: { ...selector(motion.shows), includeHidden: true } } : {}), ...(motion.holds ? { holds: { ...selector(motion.holds), includeHidden: true } } : {}), ...(motion.frameMs ? { frame_ms: motion.frameMs } : {}) }
  const started = performance.now()
  const ran = await run(at, folder, { ...scene, reducedMotion: motion.reducedMotion }, [{ op: "clock", step: 1000 }, ...taps, ...(origin ? [origin] : []), replay])
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
  // A refresh that drew nothing new (a busy device's) shows the frame drawn before it.
  let blurs
  const plain = motion.blurred ? frames.findIndex((frame, n) => ((blurs = frame.drawn === false ? blurs : frame.work?.backdrops), n > 0 && !blurs)) : -1
  // What a swipe moves goes one way while it's under the fingers, a trace's till they lift: never back.
  const lifted = events?.findLast((event) => event.kind === "up" || ["ended", "cancelled"].includes(event.phase))
  const end = events ? (lifted ?? events.at(-1)).ms : stretches.reduce((sum, stretch) => sum + stretch.ms, 0)
  const toward = Math.sign(values.at(-1) - values[0])
  const back = motion.steady ? values.findIndex((value, n) => n > 0 && frames[n].at <= end && (value - values[n - 1]) * toward < -1) : -1
  // What `onward` follows never goes back against the way it's going, in any frame, as a frame that draws it where it began
  // for one refresh does.
  const onward = motion.onward ? values.findIndex((value, n) => n > 1 && (value - values[n - 1]) * Math.sign(values.at(-1) - values[0]) < -1) : -1
  if (onward >= 0) failed.push(`it went back ${Math.abs(values[onward] - values[onward - 1]).toFixed(0)} pt at frame ${onward + 1} (${frames[onward].at} ms)`)
  if (back >= 0) failed.push(`it went back ${Math.abs(values[back] - values[back - 1]).toFixed(0)} pt at frame ${back + 1} (${frames[back].at} ms)`)
  // A press let go moves on from where the finger last showed it, as UIScrollView's deceleration and SwiftUI's spring
  // do: no frame as it's let go jumps further than the finger moved in a refresh just before, or than the frame after
  // goes on (a fling's first eased step), within a point. Two frames drawn in the refresh it lifts in show only the
  // second, a jump that skips where the finger was.
  if (!events && lift && way !== "still" && !motion.reducedMotion) {
    const refresh = 1000 / hz
    const at = frames.findIndex((frame) => frame.at >= end)
    const step = (n) => (n > 0 && n < values.length ? Math.abs(values[n] - values[n - 1]) : 0)
    const fingered = Math.max(...[0, 1, 2].map((n) => Math.abs(finger(stretches, end - n * refresh, way) - finger(stretches, end - (n + 1) * refresh, way))))
    const jump = at > 0 ? [at, at + 1].reduce((most, n) => (step(n) > step(most) ? n : most)) : -1
    if (jump > 0 && step(jump) > Math.max(fingered, step(jump + 1)) + 1)
      failed.push(`it jumped ${step(jump).toFixed(0)} pt at frame ${jump + 1} (${frames[jump].at} ms) as it was let go, where the finger moved at most ${fingered.toFixed(0)} pt a refresh and the frame after ${step(jump + 1).toFixed(0)} pt`)
  }
  // What it opens is first drawn from where what it opens from is, its top and its middle there, as an iPhone's folder
  // grows out of its square and its apps out of the square's: within a point, or a step of its own motion, which may
  // have taken one before the frame it's first drawn on.
  if (origin) {
    const from = ran.answers.at(-2)?.result?.nodes?.[0]?.rect
    const drawn = frames.flatMap((frame) => (frame.rect ? [frame.rect] : []))
    const apart = (a, b) => Math.max(Math.abs(a.y - b.y), Math.abs(a.x + a.width / 2 - b.x - b.width / 2))
    const [first] = drawn
    const next = drawn.find((one) => apart(one, first) > 0)
    if (!from || !first) failed.push(`${from ? "what it follows" : "what it opens from"} isn't there to compare`)
    else if (apart(first, from) > Math.max(1, next ? apart(next, first) : 0)) failed.push(`it's first drawn at ${rect(first)}, not where it opens from, ${rect(from)}`)
  }
  // What each wheel's turn moves goes over several frames, as Windows' apps and Chrome smooth-scroll a notch: no frame
  // moves it as far as one turn does by itself, and from the frame it first moves to the last turn, none holds still
  // more than 2 pt short of where it comes to rest (its end, where it gets there first, which it nears by less than a
  // point a frame, on whole points), as a spin of the wheel glides on.
  if (motion.eases) {
    const wheels = (events ?? []).filter((event) => event.kind === "wheel")
    const turn = Math.max(0, ...wheels.map((event) => Math.max(Math.abs(event.dx ?? 0), Math.abs(event.dy ?? 0))))
    const jump = values.findIndex((value, n) => n > 0 && Math.abs(value - values[n - 1]) >= turn - 0.5)
    const moved = values.findIndex((value) => value !== values[0])
    const held = moved < 0 ? -1 : values.findIndex((value, n) => n > moved && frames[n].at <= wheels.at(-1).ms && value === values[n - 1] && Math.abs(value - values.at(-1)) > 2)
    if (!turn) failed.push("it has no wheel's turns to ease")
    else if (jump >= 0) failed.push(`it moved ${Math.abs(values[jump] - values[jump - 1]).toFixed(0)} pt at frame ${jump + 1} (${frames[jump].at} ms), as far as a turn of the wheel moves it, all at once`)
    else if (held >= 0) failed.push(`it held still at frame ${held + 1} (${frames[held].at} ms) as the wheel turned`)
  }
  // What `once` names is never in sight twice in a frame, read or not, as a large title and its bar's are cross-faded.
  const twice = motion.once ? frames.findIndex((frame) => frame.count > 1) : -1
  if (twice >= 0) failed.push(`${frames[twice].count} of ${JSON.stringify(motion.once)} drawn at once at frame ${twice + 1} (${frames[twice].at} ms)`)
  // What `shows` names is drawn from the frame what `follow` names has landed (it moves less than half a point from there
  // on), as a folder's apps are handed back to its square as they get there: a hand-back that waits leaves them drawn
  // in the layer they flew in, and the square's undrawn, for as long as it waits.
  if (motion.shows) {
    const where = frames.map((frame) => frame.rect).filter(Boolean)
    const last = where.at(-1)
    const lands = frames.findIndex((frame, n) => frame.rect && frames.slice(n).every((later) => !later.rect || Math.abs(later.rect.x - last.x) < 0.5 && Math.abs(later.rect.y - last.y) < 0.5))
    const missing = lands < 0 ? -1 : frames.findIndex((frame, n) => n >= lands && frame.shows === 0)
    if (!last) failed.push("what it follows isn't there to land")
    else if (missing >= 0) failed.push(`${JSON.stringify(motion.shows)} isn't drawn at frame ${missing + 1} (${frames[missing].at} ms), where what moved landed at frame ${lands + 1} (${frames[lands].at} ms)`)
  }
  // What `holds` names is drawn in every frame from the first where what `follow` moves to the one where it lands: apps
  // flying out of a folder, or back into it, are drawn on their way, not handed over before they've gone.
  if (motion.holds) {
    const where = frames.map((frame) => frame.rect)
    const first = where.findIndex((rect, n) => rect && where[n + 1] && (Math.abs(where[n + 1].x - rect.x) > 0.5 || Math.abs(where[n + 1].y - rect.y) > 0.5))
    const last = where.findLastIndex((rect, n) => rect && where[n - 1] && (Math.abs(where[n - 1].x - rect.x) > 0.5 || Math.abs(where[n - 1].y - rect.y) > 0.5))
    const hidden = first < 0 ? -1 : frames.findIndex((frame, n) => n >= first && n <= last && frame.holds === 0)
    if (first < 0) failed.push("what it follows doesn't move")
    else if (hidden >= 0) failed.push(`${JSON.stringify(motion.holds)} isn't drawn at frame ${hidden + 1} (${frames[hidden].at} ms), while what moves is on its way (frames ${first + 1} to ${last + 1})`)
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


/// Every motion played, by its folder, for the contact sheet: a round that plays some keeps the rest.
const rows = new Map()

/// Plays every motion of `targets`. → each line it says.
export async function round(targets) {
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
