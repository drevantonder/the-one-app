// The devices frames are judged against (devices.json), and how many frames each would drop.
import { readFileSync } from "node:fs"
import { flag, here, ssh } from "./options.js"

/// The devices frames are judged against, and what this machine's CPU scores, as devices.json says.
export const devices = JSON.parse(readFileSync(here("devices.json"), "utf8"))
export const cpuName = process.platform === "darwin" ? Bun.spawnSync(["sysctl", "-n", "machdep.cpu.brand_string"]).stdout.toString() : (readFileSync("/proc/cpuinfo", "utf8").match(/model name\s*:\s*(.*)/)?.[1] ?? "")
export const hostScore = Object.entries(devices.hosts).find(([name]) => cpuName.includes(name))?.[1]
export const device = ssh && devices.tiers.find((tier) => tier.measured === ssh)
if (ssh && !device) throw new Error(`No tier in devices.json is measured on ${ssh}`)
/// Whether its CPU times are a shipped build's, which a tier's CPU is judged by.
const shipped = flag("--release")


/// The kinds of pixels a frame's GPU shades (`work`), as calibrate.js fits their costs.
export const SHADED = ["quads", "shadows", "sprites", "backdrops", "rasters", "copied"]

/// What a device's `measured` costs (devices.json's, as calibrate.js fits them) say a frame takes there, in ms: its
/// GPU's shading of its work (`work`'s pixels, `pixels` times as many) and uploads, its draw calls on the GPU, and its
/// thread's drawing it for the GPU (making and submitting its passes' command lists and draw calls), where measured.
export function modelled(measured, frame, pixels = 1) {
  const { costs, constant } = measured.gpu
  const shaded = SHADED.reduce((sum, kind) => sum + (frame.work[kind] / 1e6) * pixels * costs[kind], 0) + (frame.work.uploaded / 1e6) * pixels * costs.uploaded + constant
  const draw = measured.draw && measured.draw.constant + (frame.passes ?? 0) * measured.draw.costs.passes + frame.draws * measured.draw.costs.draws
  return { shaded, calls: frame.draws * costs.draws, draw }
}

/// What each tier with `hz` frames a second would take for each of `frames`, played at one device pixel a point: its
/// GPU's (what the device measured takes for the frame's work at the tier's pixels, as its GPU's score is to the
/// tier's), and its CPU's for the frame: painting it (this machine's, as its Geekbench score is to the tier's) and
/// drawing it for the GPU (the device's, measured, as its CPU's score is to the tier's: what a driver costs a pass and a
/// draw call is the device's, not this machine's), and its JS's since the frame before, the frame dropped where any is
/// longer than the tier's frame. Its JS counts only where the frame `waits` for it: what a wheel scrolls is drawn
/// whatever its JS is doing. → each tier's name, the frames dropped of those drawn, the worst and why.
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
      const measured = anchor && devices.measured[anchor.measured]
      const times = frames
        .filter((frame) => frame.draws > 0)
        .map((frame) => {
          const { shaded, calls, draw } = measured ? modelled(measured, frame, tier.scale * tier.scale) : { shaded: 0, calls: 0 }
          const gpuMs = measured ? shaded * (anchor.gpu / tier.gpu) + calls * (tier.draws ?? 1) * (anchor.cpu / tier.cpu) : 0
          // A debug build's painting is several times a shipped one's (the Windows wheel's 4 ms here, 1 in a release
          // build), so it's judged only with --release; its drawing is the device's, measured, where it was.
          const painted = hostScore && shipped ? (frame.paint_ms * hostScore) / tier.cpu : 0
          const drawn = draw !== undefined ? draw * (tier.draws ?? 1) * (anchor.cpu / tier.cpu) : hostScore && shipped ? ((frame.draw_ms ?? 0) * hostScore) / tier.cpu : 0
          const [cpuMs, jsMs] = [painted + drawn, hostScore && shipped && waits ? ((frame.script_ms ?? 0) * hostScore) / tier.cpu : 0]
          const ms = Math.max(gpuMs, cpuMs, jsMs)
          return { ms, by: ms === gpuMs ? "GPU" : ms === jsMs ? "JS" : "CPU" }
        })
      const budget = 1000 / tier.hz
      const worst = times.reduce((worst, time) => (time.ms > worst.ms ? time : worst), { ms: 0, by: "" })
      return { tier: tier.name, dropped: times.filter((time) => time.ms > budget).length, drawn: times.length, worst }
    })
}
