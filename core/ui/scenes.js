// Draws every scene on the DOM and on a native host, and compares the two: `bun scenes.js [<folder>…] [--out <dir>]
// [--against <dir>] [--dark] [--size <width>x<height>] [--release] [--web]` in core/ui, after `bun build.js` (and, for the native half, `cargo build -p renderer
// --example screenshot --features fake`, `--release` too for `--release`, whose timings are a shipped build's). `--against` compares each scene with an earlier run's too, our renderer's drawing where both have one (the DOM's where not), kept in that
// `--out`: what a change did to how things look, drawn before it (on main) and after.
//
// A folder is the gallery (core/ui/gallery, its scenes the components': `*.scenes.tsx`) or an app's (apps/<app>,
// its scenes its screens', in `client/scenes.json`); with none, the gallery and every app that has scenes. An app's
// scenes are what each screen shows, by name, and what it's given to show it, as the tools' answers a client would
// give (record.js's answers): `{ "Inbox": { "answers": { "notifications_inbox": { … } } } }`, with any of `page` (its
// page, `client/index.html` unless said, and a link after `#`), `component` (a component its client exports, drawn alone, in
// place of a page, as another app's page draws it), `uses` (other apps whose components the page draws, as Settings draws its sections: each is built, and served beside the page as `/<app>/client/`), `taps` (what's done first: pressed by what each is named or
// says, or `{ role, name }`; a point pressed, `{ x, y }`; a key, `{ key }` as the DOM names it, with `ctrl`, `shift`, `alt`
// or `meta` held; or text typed where the keyboard is, `{ type }`), `size` (`[390, 844]`, a phone's, unless said), `touch` (a finger points at it, as on a phone), and `like` (another scene's name, as Storybook's args are a
// component's: this is that scene, with its own taps, and its answers in place of that one's, tool by tool). An answer `{ "$error": "Couldn't reach it" }` is a tool's
// refusal. A method of the client's (`ui/initialize`, `one/client`) is answered as it's named too. It's 1 January 2026,
// 00:00 UTC, as a native host's harness has it, so a fixture's times are said from then.
//
// Each scene is drawn on the DOM in headless Chromium, in a frame a stand-in client answers, with `--web` by our
// renderer there too (its web frame, clients/web/renderer, on Chromium's WebGPU: on a machine with no GPU,
// SwiftShader's, which draws every frame on the CPU, so it's for a change to the web platform; elsewhere native and
// web draw with the same renderer), and on a native host by
// its headless harness (clients/app/renderer's `screenshot` example), each as a picture and its semantic tree as text
// (src/drawing-primitives/outline.ts). It says, scene by scene, whether the trees are the same and how much of the pictures
// differs, and keeps them all in `--out` to look at. A native host that can't draw a page yet (an older runtime
// version) gives nothing, which it says. It says too how quickly the native host drew each (the harness's
// `--measure`): its first frame, a frame when nothing changes, as its first field is typed in and as its first list
// scrolls, the memory drawing took and what a second window adds, to compare a change's before and after.
import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright-core"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"
import { outline } from "./src/drawing-primitives/outline.ts"

const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const root = here("../../")
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
const out = resolve(option("--out") ?? join(process.env.TMPDIR ?? "/tmp", "scenes"))
const dark = flag("--dark")
const onTheWeb = flag("--web")
// Every scene in a window this size, as a computer's, where it says none of its own.
const sized = option("--size")?.split("x").map(Number)
const against = option("--against") && resolve(option("--against"))
const folders = args.filter((arg, at) => !arg.startsWith("--") && !["--out", "--against", "--size"].includes(args[at - 1])).map((folder) => resolve(folder))
const targets = folders.length ? folders : [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")))]
const harness = join(root, `target/${flag("--release") ? "release" : "debug"}/examples/screenshot`)
// The runtime version the UI library needs, and the native host's: a host whose version is older draws nothing of a
// page written for this one, so there's nothing yet to compare.
const speaks = (file, pattern) => Number(readFileSync(join(root, file), "utf8").match(pattern)?.[1])
const needed = speaks("core/ui/src/drawing-primitives/renderer.ts", /export const RUNTIME_VERSION = (\d+)/)
const hosted = speaks("clients/app/renderer/src/lib.rs", /pub const RUNTIME_VERSION: u32 = (\d+)/)

if (!existsSync(here("dist/build/app.js"))) throw new Error("Build the UI library first: `bun build.js` in core/ui")

// What a page runs to have its semantic tree as text, as `bun record.js --dom` makes it.
const outlined = Bun.spawnSync(["bun", here("record.js"), "--dom"]).stdout.toString()

/// A target's scenes, each its name, page, link, answers, taps and size.
async function scenesOf(target, built, serve) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const all = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(all[scene.like]), taps: undefined, ...scene, answers: { ...whole(all[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(all).map(([name, scene]) => {
      const { component, uses = [], answers = {}, taps = [], size = sized ?? [390, 844], touch = false } = whole(scene)
      const at = whole(scene).page ?? (component ? `${COMPONENTS}/${component}.html` : "client/index.html")
      const [page, link] = at.split("#")
      return { name, page, link, uses, answers, taps, size, touch }
    })
  }
  // The gallery lists its own, drawn at `#scenes`.
  const listed = await serve.text("index.html", "scenes")
  return JSON.parse(listed).map(({ name, taps, size = sized ?? [390, 844], touch = false }) => ({ name, page: "index.html", link: name, answers: {}, taps, size, touch }))
}

/// Where an app's exported components are built (its client, as a server builds it), for a scene that names one
/// (`component`) rather than a page: the component alone, drawn in grouped cards, as Settings draws its sections.
const COMPONENTS = "components"

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

/// Builds `target`'s client into `built`'s COMPONENTS, with a page beside each module it exports that draws it, when one
/// of its scenes names a component. → why it didn't build, if it didn't.
function components(target, built) {
  const all = existsSync(join(target, "client/scenes.json")) ? Object.values(JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))) : []
  const named = [...new Set(all.map((scene) => scene.component).filter(Boolean))]
  if (!named.length) return
  const at = join(built, COMPONENTS)
  const make = Bun.spawnSync(["bun", here("dist/build/app.js"), join(target, "client"), at])
  if (make.exitCode !== 0) return make.stderr.toString().trim()
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

/// A stand-in client for a page in a frame: it answers what the page asks from `answers`, as a client would. The frame
/// is the page's own, or `src`, the renderer's frame that draws it (`web`).
const host = (page, link, answers, [width, height], touch, src = `/${page}${link ? `#${encodeURIComponent(link)}` : ""}`) => `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:transparent}iframe{border:0;width:${width}px;height:${height}px;display:block}</style>
<iframe id="page" src="${src}"></iframe>
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
function server(built, target, used) {
  const pages = new Map()
  const listener = Bun.serve({
    port: 0,
    fetch(request) {
      const url = new URL(request.url)
      if (url.pathname === "/host.html") return new Response(pages.get(url.searchParams.get("scene")), { headers: { "content-type": "text/html" } })
      const path = decodeURIComponent(url.pathname)
      if (path.startsWith("/renderer/")) return new Response(Bun.file(join(rendererFrame, path.slice("/renderer/".length) || "index.html")))
      const [, app, client, ...rest] = path.split("/")
      if (used.has(app) && client === "client") return new Response(Bun.file(join(used.get(app), rest.join("/"))))
      const file = path.startsWith("/ui/client/") ? Bun.file(join(here("dist/client"), path.slice("/ui/client/".length))) : [join(built, path), join(target, path)].map((one) => Bun.file(one)).find((one) => one.size)
      return file?.size ? new Response(file) : new Response("Not found", { status: 404 })
    },
  })
  return {
    url: (key) => `http://localhost:${listener.port}/host.html?scene=${encodeURIComponent(key)}`,
    host: (key, scene) => pages.set(key, host(scene.page, scene.link, scene.answers, scene.size, !!scene.touch)),
    // The renderer's frame, drawing the page at its address here.
    hostWeb: (key, scene) => pages.set(key, host(scene.page, scene.link, scene.answers, scene.size, !!scene.touch, `/renderer/?runtime=${needed}#/${scene.page}${scene.link ? `#${encodeURIComponent(scene.link)}` : ""}`)),
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
  const page = frame.page()
  if (name.key) return page.keyboard.press([...["ctrl", "shift", "alt", "meta"].filter((held) => name[held]).map((held) => ({ ctrl: "Control", shift: "Shift", alt: "Alt", meta: "Meta" })[held]), name.key].join("+"))
  if (name.type) return page.keyboard.type(name.type)
  if (name.x !== undefined) {
    const at = await page.locator("#page").boundingBox()
    return page.mouse.click(at.x + name.x, at.y + name.y)
  }
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
  const page = await browser.newPage({ viewport: { width: scene.size[0], height: scene.size[1] }, deviceScaleFactor: 2, hasTouch: scene.touch, colorScheme: dark ? "dark" : "light" })
  const problems = []
  page.on("pageerror", (error) => problems.push(String(error)))
  // The clock a native host's harness has (clients/app/renderer/src/js.rs's `Clock::fixed`): 1 January 2026, UTC.
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

/// Presses what's named or says `name` on our renderer's canvas, as a person would: found in its ARIA tree
/// (clients/web/renderer/semantics.js) and pressed where it's drawn.
async function press(frame, name) {
  const page = frame.page()
  if (name.key || name.type) return tap(frame, name)
  const at = await page.locator("#page").boundingBox()
  if (name.x !== undefined) return page.mouse.click(at.x + name.x, at.y + name.y)
  const found = typeof name === "object" ? frame.getByRole(name.role, { name: name.name, exact: true }).first() : frame.locator(`#semantics [aria-label="${name.replaceAll('"', '\\"')}"], #semantics :text-is("${name.replaceAll('"', '\\"')}")`).last()
  const box = await found.boundingBox({ timeout: 5000 }).catch(() => null)
  if (!box) throw new Error(`Nothing named or saying "${typeof name === "object" ? name.name : name}" to press`)
  // Its box is the page's, the frame's place in it already in it.
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
}

/// A scene drawn by our renderer on the web, in headless Chromium's WebGPU, as `web.png`, once its first frame is drawn
/// and what moves as it opens has settled, on the native harness's clock, language and time zone, each tap pressed
/// through its ARIA tree.
async function web(browser, serve, key, scene, folder) {
  const page = await browser.newPage({ viewport: { width: scene.size[0], height: scene.size[1] }, deviceScaleFactor: 2, hasTouch: scene.touch, colorScheme: dark ? "dark" : "light", locale: "en-US", timezoneId: "UTC" })
  const problems = []
  page.on("pageerror", (error) => problems.push(String(error)))
  page.on("console", (message) => message.type() === "warning" && !message.text().includes("preferred by this device") && problems.push(message.text()))
  await page.clock.setFixedTime(new Date(Date.UTC(2026, 0, 1)))
  serve.hostWeb(`${key} (web)`, scene)
  await page.goto(serve.url(`${key} (web)`))
  const frame = await (await page.waitForSelector("#page")).contentFrame()
  const drawn = await frame.waitForFunction(() => "drawn" in document.documentElement.dataset, null, { timeout: 20000 }).then(() => true, () => false)
  await page.waitForTimeout(1000)
  for (const name of drawn ? scene.taps : []) await press(frame, name).then(() => page.waitForTimeout(1000))
  if (drawn) await page.screenshot({ path: join(folder, "web.png") })
  await page.close()
  return drawn ? { problems } : { missing: `it drew nothing${problems.length ? `: ${problems.join("; ")}` : ""}` }
}

/// A scene drawn by a native host's headless harness, or nothing where it hasn't been built or can't draw the page.
function native(built, scene, folder, used) {
  if (!existsSync(harness)) return { missing: "the native harness isn't built (`cargo build -p renderer --example screenshot --features fake`)" }
  if (hosted < needed) return { missing: `the native host's runtime version is ${hosted}, the UI library needs ${needed}: nothing to compare until it's ${needed}` }
  // The harness loads the module the folder's `index.html` names, from beside it: the page's, with what it loads.
  const at = join(folder, "page")
  rmSync(at, { recursive: true, force: true })
  mkdirSync(at)
  writeFileSync(join(at, "index.html"), readFileSync(join(built, scene.page), "utf8").replace(/(src|href)="(\.\.\/)+/g, '$1="').replace(/(src|href)="\.\//g, '$1="'))
  // A component's page is beside the modules its client's build made, which it loads from beside it.
  if (scene.page.startsWith(`${COMPONENTS}/`)) {
    for (const entry of readdirSync(join(built, COMPONENTS))) if (!entry.endsWith(".html")) symlinkSync(join(built, COMPONENTS, entry), join(at, entry))
  } else symlinkSync(join(built, "assets"), join(at, "assets"))
  for (const app of scene.uses ?? []) {
    mkdirSync(join(at, app))
    symlinkSync(used.get(app), join(at, app, "client"))
  }
  if (existsSync(join(built, dirname(scene.page), "styles.json"))) cpSync(join(built, dirname(scene.page), "styles.json"), join(at, "styles.json"))
  const answers = { ...scene.answers, "ui/initialize": scene.answers["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  // The harness's clock stands still, so what moves as the page opens and after each tap is moved on to where it
  // settles, as the DOM's is, and what's laid out once it's drawn (onLayout) is drawn again.
  const requests = [{ op: "clock", step: 1000 }, ...scene.taps.flatMap((tap) => [tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...(typeof tap === "object" ? tap : { name: tap }) }, { op: "clock", step: 1000 }])].map((request, at) => JSON.stringify({ v: 1, id: at + 1, ...request })).join("\n")
  const run = Bun.spawnSync(
    [harness, at, join(folder, "native.png"), join(folder, "native.json"), "--ui", here("dist/client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", "2", "--measure", ...(dark ? ["--dark"] : []), ...(scene.touch ? ["--touch"] : []), ...(requests ? ["--requests", "-"] : [])],
    { stdin: requests ? new TextEncoder().encode(requests) : undefined, env: { ...process.env, VK_ICD_FILENAMES: process.env.VK_ICD_FILENAMES ?? "/usr/share/vulkan/icd.d/lvp_icd.json" } },
  )
  if (run.exitCode !== 0 || !existsSync(join(folder, "native.json"))) return { missing: run.stderr.toString().trim().split("\n").at(-1) || "the harness drew nothing" }
  const nodes = JSON.parse(readFileSync(join(folder, "native.json"), "utf8"))
  // Its tree: each node's role, name and what it is (its states, by ARIA's names, beside them), and its words.
  const semantic = ({ id, role, name, children = [], actions, states = {}, ...more }) =>
    role === "text" ? { text: name ?? "", states: {}, children: [] } : { role: role ?? undefined, name: name ?? undefined, states: Object.fromEntries(Object.entries({ ...more, ...states }).map(([state, value]) => [state, String(value)])), children: children.map(semantic) }
  const tree = outline({ states: {}, children: (Array.isArray(nodes) ? nodes : [nodes]).map(semantic) })
  writeFileSync(join(folder, "native.txt"), tree)
  const measured = run.stderr.toString().match(/^measured (.*)$/m)
  return { tree, measured: measured && JSON.parse(measured[1]) }
}

/// How quickly the native host drew a scene, as a line.
const timings = ({ first_frame_ms, worst_paint_ms, idle_ms, typing_ms, scrolling_ms, memory_mb, window_mb }) =>
  [`first frame ${first_frame_ms} ms`, `worst frame ${worst_paint_ms} ms on its thread`, `frame ${idle_ms} ms idle`, typing_ms != null && `${typing_ms} ms typing`, scrolling_ms != null && `${scrolling_ms} ms scrolling`, `${memory_mb} MB`, `${window_mb} MB a window more`].filter(Boolean).join(", ")

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

// Text laid out by its fonts' own advances, as Chrome lays it out on a Mac, Windows, an iPhone and Android: Linux's
// headless Chrome hints by default, rounding each glyph's advance to a whole pixel, which no device people use does.
const browser = await chromium.launch({ args: ["--font-render-hinting=none"] })
// Our renderer on the web, on WebGPU, which a machine with no GPU has only as SwiftShader's, and whose canvas is put on
// the page only when the page is composited on the GPU too (ANGLE on SwiftShader), at a phone's two pixels a point.
// One that hangs (SwiftShader's GPU process, under load) is started again, so it takes no other scene with it.
const webgpuArgs = ["--font-render-hinting=none", "--enable-unsafe-webgpu", "--enable-unsafe-swiftshader", "--enable-gpu", "--use-gl=angle", "--use-angle=swiftshader", "--use-webgpu-adapter=opengles", "--force-device-scale-factor=2"]
let webgpu = onTheWeb && (await chromium.launch({ args: webgpuArgs }))
// The renderer's frame, built once for every scene drawn on the web (clients/web/renderer/build.js).
const rendererFrame = join(out, "renderer")
const framed = onTheWeb && Bun.spawnSync(["bun", join(root, "clients/web/renderer/build.js"), rendererFrame])
if (framed && framed.exitCode !== 0) throw new Error(`The renderer's frame didn't build: ${framed.stderr}`)
const report = []
for (const target of targets) {
  const name = basename(target)
  const built = join(out, name, "build")
  // An app whose scenes are all of components has no page to build, and may have none at all.
  const scenes = existsSync(join(target, "client/scenes.json")) ? JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8")) : {}
  const ofComponent = (scene) => !!scene && (scene.component || ofComponent(scenes[scene.like]))
  const pages = !Object.keys(scenes).length || Object.values(scenes).some((scene) => !ofComponent(scene))
  const packages = install(join(target, "client"))
  if (packages) {
    report.push(`${name}: its packages didn't install\n${packages}`)
    continue
  }
  const make = pages ? Bun.spawnSync(["bun", here("dist/build/app.js"), target, built]) : { exitCode: 0 }
  if (make.exitCode !== 0) {
    report.push(`${name}: didn't build\n${make.stderr.toString().trim()}`)
    continue
  }
  const unbuilt = components(target, built)
  if (unbuilt) {
    report.push(`${name}: its components didn't build\n${unbuilt}`)
    continue
  }
  // The apps whose components its scenes draw, each built once and kept in `out`.
  const used = new Map()
  for (const app of new Set(Object.values(scenes).flatMap((scene) => scene.uses ?? []))) {
    const folder = join(out, ".uses", app)
    if (!existsSync(folder)) {
      const packages = install(join(root, "apps", app, "client"))
      if (packages) throw new Error(`${app}'s packages didn't install: ${packages}`)
      const made = Bun.spawnSync(["bun", here("dist/build/app.js"), join(root, "apps", app, "client"), folder])
      if (made.exitCode !== 0) throw new Error(`${app}'s components didn't build: ${made.stderr}`)
    }
    used.set(app, folder)
  }
  const serve = server(built, target, used)
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
    const drawn = native(built, scene, folder, used)
    const onWeb = !onTheWeb ? {} : await web(webgpu, serve, `${name}/${scene.name}`, scene, folder).catch(async (error) => {
      await webgpu.close().catch(() => {})
      webgpu = await chromium.launch({ args: webgpuArgs })
      return { missing: String(error) }
    })
    const said = [`${name} · ${scene.name}`]
    if (shown.problems.length) said.push(`  DOM: ${shown.problems.join("; ")}`)
    if (drawn.missing) said.push(`  native: ${drawn.missing}`)
    else if (shown.tree !== undefined) compared(said, "DOM and native", shown.tree, drawn.tree, join(folder, "dom.png"), join(folder, "native.png"), join(folder, "diff.png"))
    if (drawn.measured) said.push(`  native: ${timings(drawn.measured)}`)
    if (onWeb.missing) said.push(`  web: ${onWeb.missing}`)
    else if (onTheWeb && !drawn.missing) {
      const part = differs(join(folder, "native.png"), join(folder, "web.png"), join(folder, "web-diff.png"))
      said.push(`  native and web: pictures ${part === undefined ? "aren't the same size" : `${(part * 100).toFixed(1)}% different${part > 0.02 ? " (over 2%)" : ""}`}`)
    }
    const before = against && join(against, name, basename(folder))
    // What changed is our renderer's, where it drew the scene both times, and the DOM's where it didn't.
    const [kind, tree] = drawn.tree !== undefined && before && existsSync(join(before, "native.png")) ? ["native", drawn.tree] : ["dom", shown.tree]
    if (tree === undefined) {
    } else if (before && existsSync(join(before, `${kind}.png`))) compared(said, `before and now (${kind === "dom" ? "DOM" : "native"})`, readFileSync(join(before, `${kind}.txt`), "utf8"), tree, join(before, `${kind}.png`), join(folder, `${kind}.png`), join(folder, "changed.png"))
    else if (before) said.push("  before and now: no scene of this name before")
    said.push(`  ${folder}`)
    report.push(said.join("\n"))
  }
  serve.stop()
}
await browser.close()
if (webgpu) await webgpu.close()
console.log(report.join("\n"))
