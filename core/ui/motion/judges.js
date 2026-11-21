// A check's steps as the harness's requests, each with what judges its answer.
import { readFileSync, writeFileSync } from "node:fs"
import { join } from "node:path"
import { PNG } from "pngjs"
import { pressed, selector } from "./gesture.js"
import { rect } from "./output.js"

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
export function requests(steps, folder) {
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
      const { shown, gone, focused, text, states, drawn, blur, keyboard, margins, tall, whole, unbroken, unclipped, bleeds, aligned, centred, widest, contrast, textMenu, handles, magnifier, dimmed, js } = step.expect
      // Whether `settle` says it's settled, waiting a moment, as a check after it reads the page: never while an answer
      // it waits on is on its way (Playwright's network idle).
      if (step.expect.settled !== undefined) out.push({ request: { op: "settle", deadline: 300 }, judge: (answer) => answer?.result?.settled !== step.expect.settled && (step.expect.settled ? `it hadn't settled: ${JSON.stringify(answer?.result?.pending)}` : "it said it had settled, with what it waits for on its way") })
      // Where something's drawn, so what comes in above it isn't seen to push it.
      if (step.expect.at) out.push({ request: { op: "query", ...selector(step.expect.at.of) }, judge: (answer) => { const rect = answer?.result?.nodes?.[0]?.rect; const off = ["x", "y", "width"].filter((side) => step.expect.at[side] !== undefined && !(Math.abs(rect?.[side] - step.expect.at[side]) <= 1)); return off.length > 0 && `${named(step.expect.at.of)} is drawn at ${off.map((side) => `${side} ${rect?.[side]}, not ${step.expect.at[side]}`).join(", ")}` } })
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
      // No line of a text ends inside a run of it a pattern (a regular expression's source) finds: a date or a short
      // phrase is drawn whole on one line, never broken inside, as a name is kept whole by the no-break spaces in it.
      if (unbroken)
        out.push({
          request: { op: "query", text: "" },
          judge: (answer) => {
            const pattern = new RegExp(unbroken.replace(/\u00a0/g, " ").replace(/ /g, "[ \u00a0]"), "g")
            const broken = []
            for (const node of answer?.result?.nodes ?? []) {
              if (!node.lines || node.lines.length < 2 || !(node.opacity > 0)) continue
              const text = node.lines.join(" ")
              // The spaces the lines were joined by, where a line ended.
              const joins = []
              let at = -1
              for (const line of node.lines.slice(0, -1)) joins.push((at += line.length + 1))
              for (const found of text.matchAll(pattern)) if (joins.some((join) => join >= found.index && join < found.index + found[0].length)) broken.push(`"${found[0]}" is broken across lines: ${node.lines.map((line) => `"${line}"`).join(", ")}`)
            }
            return broken.length ? [...new Set(broken)].join(", ") : undefined
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
      // What's named scrolls to the window's edges, both of them, as a shelf under the App Store's column on a Mac does: its box
      // isn't cut short at a column's margin with room beyond it.
      if (bleeds) {
        let width
        out.push({ request: { op: "window" }, judge: (answer) => void (width = answer?.result?.width) })
        out.push({
          request: { op: "query", ...selector(bleeds) },
          judge: (answer) => {
            const nodes = (answer?.result?.nodes ?? []).filter((node) => node.rect)
            if (!nodes.length) return `${named(bleeds)} isn't drawn`
            const short = nodes.filter((node) => node.rect.x > 0.5 || node.rect.x + node.rect.width < width - 0.5)
            return short.length ? short.map((node) => `${named(bleeds)} is drawn from ${Math.round(node.rect.x)} to ${Math.round(node.rect.x + node.rect.width)} pt, not to the window's edges, 0 to ${width}`).join(", ") : undefined
          },
        })
      }
      // Every node named is drawn at the same place along its row: the same distance from the window's start, or its end,
      // as controls at the end of rows are in a column.
      if (aligned)
        out.push({
          request: { op: "query", ...selector(aligned.of) },
          judge: (answer) => {
            const nodes = (answer?.result?.nodes ?? []).filter((node) => node.rect && node.opacity > 0)
            if (nodes.length < 2) return nodes.length ? undefined : `${named(aligned.of)} isn't drawn`
            const place = (node) => (aligned.at === "start" ? node.rect.x : node.rect.x + node.rect.width)
            const places = nodes.map(place)
            return Math.max(...places) - Math.min(...places) > 1 && `${named(aligned.of)} is drawn at ${places.map((at) => Math.round(at)).join(", ")} pt, not at one place along its row`
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

/// Why a frame drawn on what the window kept from frames before isn't the one drawn afresh (`fresh`), if it isn't: the
/// two pictures kept beside `folder`, as `kept.png` and `afresh.png`, where given one.
export const unlike = (answer, folder) => {
  if (answer?.error) return answer.error
  const { differ, of, box, kept, afresh } = answer?.result ?? {}
  if (!differ) return
  if (folder && kept) for (const [name, png] of [["kept", kept], ["afresh", afresh]]) writeFileSync(join(folder, `${name}.png`), Buffer.from(png, "base64"))
  return `the frame drawn on what it kept differs from one drawn afresh in ${differ} of ${of} pixels${box ? `, from ${box.x},${box.y} ${box.width}×${box.height} px` : ""}`
}
