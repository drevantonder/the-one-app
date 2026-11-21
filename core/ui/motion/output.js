// What a motion keeps: its curve, filmstrip and contact sheet, and how two of them differ.
import { readFileSync, writeFileSync } from "node:fs"
import { join, relative } from "node:path"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"
import { out } from "./options.js"
import { finger } from "./gesture.js"

/// The curve: what moved, and the finger, over time, each frame over its budget red.
export function curve(frames, way, values, stretches, budget, before, apple) {
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
export function filmstrip(frames, file) {
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
export function apart(frames, values, before) {
  const at = (t) => {
    const next = before.frames.findIndex((frame) => frame.at >= t)
    if (next <= 0) return before.values[next === 0 ? 0 : before.values.length - 1]
    const [a, b] = [before.frames[next - 1], before.frames[next]]
    return before.values[next - 1] + ((t - a.at) / (b.at - a.at)) * (before.values[next] - before.values[next - 1])
  }
  return Math.max(...frames.map((frame, n) => Math.abs(values[n] - at(frame.at))))
}


/// A rect as it's said: where, and how big.
export const rect = ({ x, y, width, height }) => `${Math.round(x)}, ${Math.round(y)} ${Math.round(width)} × ${Math.round(height)}`

/// Where a text's glyphs are drawn against where its `box` says: the pixels of a picture (`png`, of a window `points` wide) in
/// `box` (a point out) that aren't the colour just outside its top left, as the box they fill. → why they're not where
/// `box` is, or nothing where they are: none, or a box that's much smaller than it or off its middle.
export function strayInk(png, points, box) {
  const { data, width, height } = PNG.sync.read(Buffer.from(png, "base64"))
  const scale = width / points
  const [left, right, top, bottom] = [box.x - 1, box.x + box.width + 1, box.y - 1, box.y + box.height + 1].map((point) => Math.round(point * scale))
  const page = (Math.max(top, 0) * width + Math.max(left, 0)) * 4
  let [x0, x1, y0, y1] = [Infinity, -Infinity, Infinity, -Infinity]
  for (let y = Math.max(top, 0); y < Math.min(bottom, height); y++)
    for (let x = Math.max(left, 0); x < Math.min(right, width); x++) {
      const at = (y * width + x) * 4
      if (Math.abs(data[at] - data[page]) + Math.abs(data[at + 1] - data[page + 1]) + Math.abs(data[at + 2] - data[page + 2]) > 120) [x0, x1, y0, y1] = [Math.min(x0, x), Math.max(x1, x), Math.min(y0, y), Math.max(y1, y)]
    }
  if (x1 < x0) return `no glyphs are drawn in ${rect(box)}`
  const ink = { x: x0 / scale, y: y0 / scale, width: (x1 - x0 + 1) / scale, height: (y1 - y0 + 1) / scale }
  const [along, across] = box.width >= box.height ? [ink.width / box.width, ink.height / box.height] : [ink.height / box.height, ink.width / box.width]
  const off = Math.max(Math.abs(ink.x + ink.width / 2 - box.x - box.width / 2) - box.width * 0.15, Math.abs(ink.y + ink.height / 2 - box.y - box.height / 2) - box.height * 0.15)
  return along < 0.6 || across < 0.35 || off > 1 ? `it's drawn at ${rect(box)} but its glyphs fill ${rect(ink)}` : undefined
}

/// How much of two pictures differs, from 0 to 1, where they're the same size, with where in `diff`.
export function differs(one, other, diff) {
  const [a, b] = [one, other].map((file) => PNG.sync.read(readFileSync(file)))
  if (a.width !== b.width || a.height !== b.height) return undefined
  const marked = new PNG({ width: a.width, height: a.height })
  const changed = pixelmatch(a.data, b.data, marked.data, a.width, a.height, { threshold: 0.1 })
  writeFileSync(diff, PNG.sync.write(marked))
  return changed / (a.width * a.height)
}

/// The contact sheet: every motion's filmstrip and curve, before beside after where there's a before.
export function contactSheet(rows) {
  const cell = (folder) => `<figure><img src="${folder}/filmstrip.png"><img src="${folder}/curve.svg"></figure>`
  const body = rows
    .map((row) => `<section><h2>${row.scene} · ${row.motion} · ${row.hz} Hz</h2>${row.before ? `<div class="pair"><div><h3>Before</h3>${cell(relative(out, row.before))}</div><div><h3>After</h3>${cell(row.folder)}</div></div>` : cell(row.folder)}</section>`)
    .join("\n")
  writeFileSync(join(out, "index.html"), `<!doctype html><meta charset="utf-8"><title>Motion</title>
<style>body{font-family:system-ui;margin:16px;background:#f6f6f7}h2{font-size:15px;margin:24px 0 8px}h3{font-size:12px;margin:0 0 4px;color:#666}figure{margin:0;display:flex;flex-direction:column;gap:8px}img{max-width:100%;background:white;border-radius:6px}.pair{display:grid;grid-template-columns:1fr 1fr;gap:16px}</style>
<h1>Motion</h1>${body}`)
}
