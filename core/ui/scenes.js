// Draws every scene on the DOM and on a native host, and compares the two: `bun scenes.js [<folder>…] [--out <dir>]
// [--against <dir>] [--dark]` in core/ui, after `bun build.js` (and, for the native half, `cargo build -p strict
// --example screenshot`). `--against` compares each scene's DOM drawing with an earlier run's too, kept in that
// `--out`: what a change did to how things look, drawn before it (on main) and after.
//
// A folder is the gallery (core/ui/gallery, its scenes the components': `*.scenes.tsx`) or an app's (apps/<app>,
// its scenes its screens', in `client/scenes.json`); with none, the gallery and every app that has scenes. An app's
// scenes are what each screen shows, by name, and what it's given to show it, as the tools' answers a client would
// give (record.js's answers): `{ "Inbox": { "answers": { "notifications_inbox": { … } } } }`, with any of `page` (its
// page, `client/index.html` unless said, and a link after `#`), `taps` (what's pressed first, by what each is named or
// says), `size` (`[390, 844]`, a phone's, unless said), `touch` (a finger points at it, as on a phone), and `like` (another scene's name, as Storybook's args are a
// component's: this is that scene, with its own taps, and its answers in place of that one's, tool by tool). An answer `{ "$error": "Couldn't reach it" }` is a tool's
// refusal. A method of the client's (`ui/initialize`, `one/client`) is answered as it's named too. It's 1 January 2026,
// 00:00 UTC, as a native host's harness has it, so a fixture's times are said from then.
//
// Each scene is drawn on the DOM in headless Chromium, in a frame a stand-in client answers, and on a native host by
// its headless harness (clients/app/strict's `screenshot` example), each as a picture and its semantic tree as text
// (src/strict/outline.ts). It says, scene by scene, whether the trees are the same and how much of the pictures
// differs, and keeps them all in `--out` to look at. A native host that can't draw a page yet (an older protocol)
// gives nothing, which it says.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { chromium } from "playwright-core"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"
import { outline } from "./src/strict/outline.ts"

const here = (path) => new URL(path, import.meta.url).pathname
const root = here("../../")
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const out = resolve(option("--out") ?? join(process.env.TMPDIR ?? "/tmp", "scenes"))
const dark = flag("--dark")
const against = option("--against") && resolve(option("--against"))
const folders = args.filter((arg, at) => !arg.startsWith("--") && !["--out", "--against"].includes(args[at - 1])).map((folder) => resolve(folder))
const targets = folders.length ? folders : [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")))]
const harness = join(root, "target/debug/examples/screenshot")
// The strict protocol the UI library speaks, and the native host: a host that speaks an older one draws nothing of a
// page written for this one, so there's nothing yet to compare.
const speaks = (file, pattern) => Number(readFileSync(join(root, file), "utf8").match(pattern)?.[1])
const protocol = speaks("core/ui/src/strict/protocol.ts", /export const VERSION = (\d+)/)
const hosted = speaks("clients/app/strict/src/lib.rs", /pub const VERSION: u32 = (\d+)/)

if (!existsSync(here("dist/build/app.js"))) throw new Error("Build the UI library first: `bun build.js` in core/ui")

// What a page runs to have its semantic tree as text, as `bun record.js --dom` makes it.
const outlined = Bun.spawnSync(["bun", here("record.js"), "--dom"]).stdout.toString()

/// A target's scenes, each its name, page, link, answers, taps and size.
async function scenesOf(target, built, serve) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const all = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(all[scene.like]), taps: undefined, ...scene, answers: { ...whole(all[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(all).map(([name, scene]) => {
      const { page: at = "client/index.html", answers = {}, taps = [], size = [390, 844], touch = false } = whole(scene)
      const [page, link] = at.split("#")
      return { name, page, link, answers, taps, size, touch }
    })
  }
  // The gallery lists its own, drawn at `#scenes`.
  const listed = await serve.text("index.html", "scenes")
  return JSON.parse(listed).map(({ name, taps, size = [390, 844], touch = false }) => ({ name, page: "index.html", link: name, answers: {}, taps, size, touch }))
}

/// A stand-in client for a page in a frame: it answers what the page asks from `answers`, as a client would.
const host = (page, link, answers, [width, height], touch) => `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:transparent}iframe{border:0;width:${width}px;height:${height}px;display:block}</style>
<iframe id="page" src="/${page}${link ? `#${encodeURIComponent(link)}` : ""}"></iframe>
<script>
const answers = ${JSON.stringify(answers)}
window.pending = 0
addEventListener("message", ({ data, source }) => {
  if (!data || data.id === undefined) return
  const reply = (body) => source.postMessage({ jsonrpc: "2.0", id: data.id, ...body }, "*")
  const own = (method) => (method in answers ? answers[method] : undefined)
  if (data.method === "ui/initialize") return reply({ result: own("ui/initialize") ?? { hostContext: { containerDimensions: { width: ${width}, height: ${height} }, deviceCapabilities: { touch: ${touch}, hover: ${!touch} }, theme: ${JSON.stringify(dark ? "dark" : "light")} } } })
  if (data.method === "tools/call") {
    const answer = own(data.params.name)
    if (answer === undefined || answer?.$error) return reply({ result: { isError: true, content: [{ type: "text", text: answer?.$error ?? "No answer for " + data.params.name }] } })
    return reply({ result: { content: [{ type: "text", text: JSON.stringify(answer) }] } })
  }
  // A method no answer names isn't answered, as record.js's and the native harness's aren't.
  const answer = own(data.method)
  reply(answer === undefined ? { error: { code: -32601, message: data.method + " isn't answered here" } } : { result: answer })
})
</script>`

/// Serves a built page, what its build leaves as it is (a picture) as a server serves an app's files, the UI library, and the stand-in client around it.
function server(built, target) {
  const pages = new Map()
  const listener = Bun.serve({
    port: 0,
    fetch(request) {
      const url = new URL(request.url)
      if (url.pathname === "/host.html") return new Response(pages.get(url.searchParams.get("scene")), { headers: { "content-type": "text/html" } })
      const path = decodeURIComponent(url.pathname)
      const file = path.startsWith("/ui/client/") ? Bun.file(here("dist/client") + path.slice("/ui/client".length)) : [join(built, path), join(target, path)].map((one) => Bun.file(one)).find((one) => one.size)
      return file?.size ? new Response(file) : new Response("Not found", { status: 404 })
    },
  })
  return {
    url: (key) => `http://localhost:${listener.port}/host.html?scene=${encodeURIComponent(key)}`,
    host: (key, scene) => pages.set(key, host(scene.page, scene.link, scene.answers, scene.size, !!scene.touch)),
    stop: () => listener.stop(true),
  }
}

/// Waits until a frame has drawn what it was asked and stopped moving: its fonts, two frames, and its animations done.
const settle = (frame) =>
  frame.evaluate(() =>
    document.fonts.ready
      .then(() => new Promise((drawn) => requestAnimationFrame(() => requestAnimationFrame(drawn))))
      .then(() => Promise.all(document.getAnimations().filter((move) => move.effect?.getComputedTiming().endTime !== Infinity).map((move) => move.finished.catch(() => {}))))
      .then(() => new Promise((drawn) => setTimeout(drawn, 400))),
  )

/// Presses what's named or says `name`, or what has `role` and is named `name`, as a person would.
async function tap(frame, name) {
  if (typeof name === "object") return frame.getByRole(name.role, { name: name.name, exact: true }).first().click()
  const found = await frame.evaluateHandle((name) => {
    const all = [...document.querySelectorAll("*")].filter((element) => element.checkVisibility?.() !== false)
    return all.find((element) => element.getAttribute("aria-label") === name) ?? all.findLast((element) => element.innerText?.trim() === name && element.children.length <= 2)
  }, name)
  const element = found.asElement()
  if (!element) throw new Error(`Nothing named or saying "${name}" to press`)
  await element.click()
}

/// A scene drawn on the DOM: its picture and its tree.
async function dom(browser, serve, key, scene, folder) {
  const page = await browser.newPage({ viewport: { width: scene.size[0], height: scene.size[1] }, deviceScaleFactor: 2, hasTouch: scene.touch, isMobile: scene.touch, colorScheme: dark ? "dark" : "light" })
  const problems = []
  page.on("pageerror", (error) => problems.push(String(error)))
  // The clock a native host's harness has (clients/app/strict/src/js.rs's `Clock::fixed`): 1 January 2026, UTC.
  await page.clock.setFixedTime(new Date(Date.UTC(2026, 0, 1)))
  serve.host(key, scene)
  await page.goto(serve.url(key))
  const frame = await (await page.waitForSelector("#page")).contentFrame()
  await frame.waitForLoadState()
  await settle(frame)
  for (const name of scene.taps) await tap(frame, name).then(() => settle(frame))
  await page.screenshot({ path: join(folder, "dom.png") })
  await frame.evaluate(outlined)
  const tree = await frame.evaluate(() => window.outlineOf(document.body))
  writeFileSync(join(folder, "dom.txt"), tree)
  await page.close()
  return { tree, problems }
}

/// A scene drawn by a native host's headless harness, or nothing where it hasn't been built or can't draw the page.
function native(built, scene, folder) {
  if (!existsSync(harness)) return { missing: "the native harness isn't built (`cargo build -p strict --example screenshot`)" }
  if (hosted < protocol) return { missing: `the native host speaks protocol ${hosted}, the UI library ${protocol}: nothing to compare until it speaks ${protocol}` }
  // The harness loads the module the folder's `index.html` names, from beside it: the page's, with what it loads.
  const at = join(folder, "page")
  rmSync(at, { recursive: true, force: true })
  mkdirSync(at)
  writeFileSync(join(at, "index.html"), readFileSync(join(built, scene.page), "utf8").replace(/(src|href)="(\.\.\/)+/g, '$1="').replace(/(src|href)="\.\//g, '$1="'))
  symlinkSync(join(built, "assets"), join(at, "assets"))
  if (existsSync(join(built, dirname(scene.page), "styles.json"))) cpSync(join(built, dirname(scene.page), "styles.json"), join(at, "styles.json"))
  const answers = { ...scene.answers, "ui/initialize": scene.answers["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  // The harness's clock stands still, so what moves after each tap is moved on to where it settles, as the DOM's is.
  const requests = scene.taps.flatMap((tap) => [{ op: "tap", ...(typeof tap === "object" ? tap : { name: tap }) }, { op: "clock", step: 1000 }]).map((request, at) => JSON.stringify({ v: 1, id: at + 1, ...request })).join("\n")
  const run = Bun.spawnSync(
    [harness, at, join(folder, "native.png"), join(folder, "native.json"), "--ui", here("dist/client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", "2", ...(dark ? ["--dark"] : []), ...(requests ? ["--requests", "-"] : [])],
    { stdin: requests ? new TextEncoder().encode(requests) : undefined, env: { ...process.env, VK_ICD_FILENAMES: process.env.VK_ICD_FILENAMES ?? "/usr/share/vulkan/icd.d/lvp_icd.json" } },
  )
  if (run.exitCode !== 0 || !existsSync(join(folder, "native.json"))) return { missing: run.stderr.toString().trim().split("\n").at(-1) || "the harness drew nothing" }
  const nodes = JSON.parse(readFileSync(join(folder, "native.json"), "utf8"))
  // Its tree: each node's role, name and what it is (its states, by ARIA's names, beside them), and its words.
  const semantic = ({ id, role, name, children = [], actions, states = {}, ...more }) =>
    role === "text" ? { text: name ?? "", states: {}, children: [] } : { role: role ?? undefined, name: name ?? undefined, states: Object.fromEntries(Object.entries({ ...more, ...states }).map(([state, value]) => [state, String(value)])), children: children.map(semantic) }
  const tree = outline({ states: {}, children: (Array.isArray(nodes) ? nodes : [nodes]).map(semantic) })
  writeFileSync(join(folder, "native.txt"), tree)
  return { tree }
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

/// Where two trees first differ, as a line of each.
function first(one = "", other = "") {
  const [a, b] = [one.split("\n"), other.split("\n")]
  const at = Array.from({ length: Math.max(a.length, b.length) }, (_, index) => index).find((index) => a[index] !== b[index])
  return at === undefined ? undefined : { at: at + 1, a: a[at]?.trim() ?? "(nothing)", b: b[at]?.trim() ?? "(nothing)" }
}

/// What a comparison says: the trees and the pictures, `them` named.
function compared(said, them, treeA, treeB, pictureA, pictureB, diff) {
  const off = first(treeA, treeB)
  said.push(`  ${them}: trees ${off ? `differ, first at line ${off.at}: "${off.a}" and "${off.b}"` : "the same"}`)
  const part = differs(pictureA, pictureB, diff)
  said.push(`  ${them}: pictures ${part === undefined ? "aren't the same size" : `${(part * 100).toFixed(1)}% different${part > 0.02 ? " (over 2%)" : ""}`}`)
}

const browser = await chromium.launch()
const report = []
for (const target of targets) {
  const name = target.endsWith("core/ui/gallery") ? "gallery" : target.split("/").at(-1)
  const built = join(out, name, "build")
  const make = Bun.spawnSync(["bun", here("dist/build/app.js"), target, built])
  if (make.exitCode !== 0) {
    report.push(`${name}: didn't build\n${make.stderr.toString().trim()}`)
    continue
  }
  const serve = server(built, target)
  serve.text = async (page, link) => {
    const tab = await browser.newPage()
    serve.host("list", { page, link, answers: {}, size: [390, 844] })
    await tab.goto(serve.url("list"))
    const frame = await (await tab.waitForSelector("#page")).contentFrame()
    await frame.waitForFunction(() => document.body.innerText.trim().startsWith("["))
    const text = await frame.evaluate(() => document.body.innerText.trim())
    await tab.close()
    return text
  }
  for (const scene of await scenesOf(target, built, serve)) {
    const folder = join(out, name, scene.name.replace(/[^\w.-]+/g, "-"))
    mkdirSync(folder, { recursive: true })
    const shown = await dom(browser, serve, `${name}/${scene.name}`, scene, folder).catch((error) => ({ problems: [String(error)] }))
    const drawn = native(built, scene, folder)
    const said = [`${name} · ${scene.name}`]
    if (shown.problems.length) said.push(`  DOM: ${shown.problems.join("; ")}`)
    if (drawn.missing) said.push(`  native: ${drawn.missing}`)
    else compared(said, "DOM and native", shown.tree, drawn.tree, join(folder, "dom.png"), join(folder, "native.png"), join(folder, "diff.png"))
    const before = against && join(against, name, folder.split("/").at(-1))
    if (before && existsSync(join(before, "dom.png"))) compared(said, "before and now", readFileSync(join(before, "dom.txt"), "utf8"), shown.tree, join(before, "dom.png"), join(folder, "dom.png"), join(folder, "changed.png"))
    else if (before) said.push("  before and now: no scene of this name before")
    said.push(`  ${folder}`)
    report.push(said.join("\n"))
  }
  serve.stop()
}
await browser.close()
console.log(report.join("\n"))
