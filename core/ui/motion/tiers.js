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
          // frame waits for whichever's longer. A debug build's are several times a shipped one's (the Windows wheel's
          // painting 4 ms here, 1 in a release build), so they're judged only with --release.
          const [cpuMs, jsMs] = hostScore && shipped ? [frame.paint_ms + (frame.draw_ms ?? 0), waits ? frame.script_ms ?? 0 : 0].map((ms) => (ms * hostScore) / tier.cpu) : [0, 0]
          const ms = Math.max(gpuMs, cpuMs, jsMs)
          return { ms, by: ms === gpuMs ? "GPU" : ms === jsMs ? "JS" : "CPU" }
        })
      const budget = 1000 / tier.hz
      const worst = times.reduce((worst, time) => (time.ms > worst.ms ? time : worst), { ms: 0, by: "" })
      return { tier: tier.name, dropped: times.filter((time) => time.ms > budget).length, drawn: times.length, worst }
    })
}
