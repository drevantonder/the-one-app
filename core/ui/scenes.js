// Draws every scene on a native host, and with `--web` on the web too, and compares them: `bun scenes.js [<folder>…] [--scene <words>]
// [--out <dir>] [--against <dir>] [--dark] [--touch] [--size <width>x<height>] [--release] [--web] [--ssh <host> --harness <its build of the
// harness>]` in core/ui, after `bun build.js` (and, for the native half, `cargo build -p renderer
// --example screenshot --features fake`, `--release` too for `--release`, whose timings are a shipped build's). `--against` compares each scene with an earlier run's too, kept in that
// `--out`: what a change did to how things look, drawn before it (on main) and after.
//
// A folder is the gallery (core/ui/gallery, its scenes the components': `*.scenes.tsx`) or an app's (apps/<app>,
// its scenes its screens', in `client/scenes.json`); with none, the gallery and every app that has scenes. An app's
// scenes are what each screen shows, by name, and what it's given to show it, as the tools' answers a client would
// give (record.js's answers): `{ "Inbox": { "answers": { "notifications_inbox": { … } } } }`, with any of `page` (its
// page, `client/index.html` unless said, and a link after `#`), `component` (a component its client exports, drawn alone, in
// place of a page, as another app's page draws it), `uses` (other apps whose components the page draws, as Settings draws its sections: each is built, and served beside the page as `/<app>/client/`), `taps` (what's done first: pressed by what each is named or
// says, or `{ role, name }`; a point pressed, `{ x, y }`; a key, `{ key }` as the web names it, with `ctrl`, `shift`, `alt`
// or `meta` held; or text typed where the keyboard is, `{ type }`), `size` (`[390, 844]`, a phone's, unless said), `touch` (a finger points at it, as on a phone), `errors` (words of the errors it logs, as a page that can't reach its server does: any other the page logs fails the scene), `timeZone` (the IANA name of the device's, `UTC` unless said: a fixture's instants are said in it, as a person in it reads them), `latency` (ms each tool's answer takes to come on the page's clock, as a server's round trip: for checks of what shows meanwhile), and `like` (another scene's name, as Storybook's args are a
// component's: this is that scene, with its own taps, and its answers in place of that one's, tool by tool). An answer `{ "$error": "Couldn't reach it" }` is a tool's
// refusal, and `{ "$image": "data:image/png;base64,…" }` a picture, as a server answers one (MCP's `image`: a screenshot). A method of the client's (`ui/initialize`, `one/client`) is answered as it's named too. It's 1 January 2026,
// 00:00 UTC, as a native host's harness has it, so a fixture's times are said from then.
//
// Each scene is drawn on a native host by its headless harness (clients/app/renderer's `screenshot` example), with
// `--web` by our renderer in headless Chromium too, in a frame a stand-in client answers (its web frame, clients/web/renderer, on Chromium's WebGPU: on a machine with no GPU,
// SwiftShader's, which draws every frame on the CPU, so it's for a change to the web platform; elsewhere native and
// web draw with the same renderer), with `--ssh` on a device's GPU too, by its own build of the harness (a Windows
// one's Direct3D 12, as motion.js plays there: where each node is laid out there and here, and how much of the
// pictures differs, `device.png` beside `afresh.png`, drawn the same way here), each as a picture, and natively its
// semantic tree as text (src/drawing-primitives/outline.ts) too. It says, scene by scene, how much of the pictures
// differs (a web picture more than 1% from its native one fails, `WEB_DIFFERS_MOST`), and keeps them all in `--out` to look at. A native host that can't draw a page yet (an older runtime
// version) gives nothing, which it says. It says too how quickly the native host drew each (the harness's
// `--measure`): its first frame, a frame when nothing changes, as its first field is typed in and as its first list
// scrolls, the memory drawing took and what a second window adds, to compare a change's before and after.
//
// As a page opens there, each tool's answer comes a round trip after it's asked (LATENCY_MS), but a tool's that only
// reads, which a client answers at once from what it keeps: its first content is when it's had every answer, and a
// scene whose page takes longer than OPENS_WITHIN_MS to it, as one that asks again for what an answer said or opens on
// a tool that changes something does, fails, which the exit status says.
//
// A scene under a finger is on a phone, whose window's top and bottom the device keeps for itself (IPHONE's, unless it
// says its own `safeArea`): words or a control drawn under its status bar, or under its home indicator but for what
// scrolls there, fail it. `--touch` draws every scene a phone's width (under COMPACT) under a finger, as on a phone.
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs"
import { basename, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { chromium } from "playwright-core"
import { PNG } from "pngjs"
import pixelmatch from "pixelmatch"
import { outline } from "./src/drawing-primitives/outline.ts"
import { messages } from "./src/words.ts"
import { build, capped, COMPONENTS, copied, env, library, pageFolder, runThere, scenesOf as gallery, stale, stopped, usedApp } from "./motion.js"

const here = (path) => fileURLToPath(new URL(path, import.meta.url))
const root = here("../../")
const args = process.argv.slice(2)
const flag = (name) => args.includes(name)
const option = (name) => (args.includes(name) ? args[args.indexOf(name) + 1] : undefined)
// Its own worktree's, never a folder every agent on the machine shares, where one ran with another's UI build.
const out = resolve(option("--out") ?? join(root, "target", "scenes"))
const dark = flag("--dark")
const touched = flag("--touch")
const onTheWeb = flag("--web")
// Every scene in a window this size, as a computer's, where it says none of its own.
const sized = option("--size")?.split("x").map(Number)
const against = option("--against") && resolve(option("--against"))
// A device over ssh (`--ssh`, with its own build of the harness, `--harness`) draws each scene too, as motion.js plays there.
const ssh = option("--ssh")
// Only the scenes whose names have these words, as motion.js's `--scene`.
const only = option("--scene")?.toLowerCase()
const folders = args.filter((arg, at) => !arg.startsWith("--") && !["--out", "--against", "--size", "--ssh", "--harness", "--scene"].includes(args[at - 1])).map((folder) => resolve(folder))
const targets = folders.length ? folders : [here("gallery"), ...readdirSync(join(root, "apps")).map((app) => join(root, "apps", app)).filter((app) => existsSync(join(app, "client/scenes.json")))]
let harness = join(root, `target/${flag("--release") ? "release" : "debug"}/examples/screenshot`)
// The runtime version the UI library needs, and the native host's: a host whose version is older draws nothing of a
// page written for this one, so there's nothing yet to compare.
const speaks = (file, pattern) => Number(readFileSync(join(root, file), "utf8").match(pattern)?.[1])
const needed = speaks("core/ui/src/drawing-primitives/renderer.ts", /export const RUNTIME_VERSION = (\d+)/)
const hosted = speaks("clients/app/renderer/src/lib.rs", /pub const RUNTIME_VERSION: u32 = (\d+)/)

if (stale(harness)) throw new Error(stale(harness))
harness = copied(harness, out)


/// A target's scenes, each its name, page, link, answers, taps and size, and where a wheel's notch eases what it
/// scrolls (`wheel`: a motion's `eases`, at its trace's first wheel's point).
async function scenesOf(target, built) {
  if (existsSync(join(target, "client/scenes.json"))) {
    const all = JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8"))
    const whole = (scene) => (scene.like ? { ...whole(all[scene.like]), taps: undefined, ...scene, answers: { ...whole(all[scene.like]).answers, ...scene.answers } } : scene)
    return Object.entries(all).map(([name, scene]) => {
      const { component, uses = [], answers = {}, taps = [], size = sized ?? [390, 844], touch = false, timeZone = "UTC", errors, safeArea } = whole(scene)
      const at = whole(scene).page ?? (component ? `${COMPONENTS}/${component}.html` : "client/index.html")
      const [page, link] = at.split("#")
      const eased = Object.values(whole(scene).motions ?? {}).find((motion) => motion.eases && motion.gesture?.trace)
      const wheel = eased && JSON.parse(readFileSync(join(target, "client", eased.gesture.trace), "utf8")).find((event) => event.kind === "wheel")
      const fingered = touch || (touched && size[0] < COMPACT)
      return { name, page, link, uses, answers, taps, size, touch: fingered, timeZone, errors, safeArea: safeArea ?? (fingered ? IPHONE : undefined), wheel }
    })
  }
  // The gallery lists its own, drawn at `#scenes` by the native harness (motion.js).
  return (await gallery(target, built, join(out, basename(target), ".scenes"), true)).map((scene) => {
    const size = sized ?? scene.size
    const fingered = scene.touch || (touched && size[0] < COMPACT)
    return { ...scene, size, touch: fingered, safeArea: scene.safeArea ?? (fingered ? IPHONE : undefined), timeZone: "UTC" }
  })
}

/// A stand-in client for a page in a frame: it answers what the page asks from `answers`, as a client would. The frame
/// is the page's own, or `src`, the renderer's frame that draws it (`web`).
const host = (page, link, answers, [width, height], touch, safeArea, src = `/${page}${link ? `#${encodeURIComponent(link)}` : ""}`) => `<!doctype html><meta charset="utf-8"><style>html,body{margin:0;background:transparent}iframe{border:0;width:${width}px;height:${height}px;display:block}</style>
<iframe id="page" src="${src}"></iframe>
<script>
const answers = ${JSON.stringify(answers)}
window.pending = 0
addEventListener("message", ({ data, source }) => {
  if (!data || data.id === undefined) return
  const reply = (body) => source.postMessage({ jsonrpc: "2.0", id: data.id, ...body }, "*")
  const own = (method) => (method in answers ? answers[method] : undefined)
  if (data.method === "ui/initialize") return reply({ result: own("ui/initialize") ?? { hostContext: { containerDimensions: { width: ${width}, height: ${height} }, deviceCapabilities: { touch: ${touch}, hover: ${!touch} }, theme: ${JSON.stringify(dark ? "dark" : "light")}${safeArea ? `, safeAreaInsets: ${JSON.stringify(insets(safeArea))}` : ""} } } })
  if (data.method === "tools/call") {
    const answer = own(data.params.name)
    if (answer === undefined || answer?.$error) return reply({ result: { isError: true, content: [{ type: "text", text: answer?.$error ?? "No answer for " + data.params.name }] } })
    const [, mimeType, image] = answer?.$image?.match(/^data:(.*?);base64,(.*)$/) ?? []
    return reply({ result: { content: [mimeType ? { type: "image", mimeType, data: image } : { type: "text", text: JSON.stringify(answer) }] } })
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
      // Each to a frame's page too, whose origin is its own, as the edge serves an app's files, the UI library and the
      // renderer's frame (`file` in core/edge/mcp/mcp.ts, clients/web/_headers).
      const cors = { headers: { "access-control-allow-origin": "*" } }
      if (path.startsWith("/renderer/")) return new Response(Bun.file(join(rendererFrame, path.slice("/renderer/".length) || "index.html")), cors)
      const [, app, client, ...rest] = path.split("/")
      if (used.has(app) && client === "client") return new Response(Bun.file(join(used.get(app), rest.join("/"))), cors)
      const file = path.startsWith("/ui/client/") ? Bun.file(join(library("client"), path.slice("/ui/client/".length))) : [join(built, path), join(target, path)].map((one) => Bun.file(one)).find((one) => one.size)
      return file?.size ? new Response(file, cors) : new Response("Not found", { status: 404 })
    },
  })
  return {
    url: (key) => `http://localhost:${listener.port}/host.html?scene=${encodeURIComponent(key)}`,
    // The renderer's frame, drawing the page at its address here.
    hostWeb: (key, scene) => pages.set(key, host(scene.page, scene.link, scene.answers, scene.size, !!scene.touch, scene.safeArea, `/renderer/?runtime=${needed}#/${scene.page}${scene.link ? `#${encodeURIComponent(scene.link)}` : ""}`)),
    stop: () => listener.stop(true),
  }
}

/// The rows (`- listitem`) of a scene's semantic tree (src/drawing-primitives/outline.ts) that have no `list` or `listbox`
/// above them: each as its line.
function strayRows(tree) {
  const lines = tree.split("\n")
  const depth = (line) => line.length - line.trimStart().length
  return lines.flatMap((line, at) => {
    if (!/^\s*- listitem\b/.test(line)) return []
    for (let up = at - 1, floor = depth(line); up >= 0; up--) {
      if (depth(lines[up]) >= floor) continue
      floor = depth(lines[up])
      if (/^\s*- (list|listbox)\b/.test(lines[up])) return []
    }
    return [line.trim()]
  })
}

/// What a screen's words may not say to a person: this codebase's own, as a build error's (`entrypoint`, `runtime_version`,
/// a file and its line), an error's code (JSON-RPC's `-32601`, a system's `ECONNREFUSED`, Erlang's `{:error, :timeout}`,
/// an HTTP status, an exception's name, `TypeError`), or a tool's name (`computer-use_screenshot`, as a tool that didn't
/// answer says it), which belong in Telemetry and to the agents. Each line of a scene's semantic tree that says one.
const INTERNAL = /entrypoint|runtime_version|\.[mc]?[jt]sx?:?\(?\d+[,:]\d+|\berror: /i
const CODE = /(^|[^\d])-32\d{3}\b|\bE(CONN[A-Z]+|NOENT|TIMEDOUT|HOSTUNREACH|NETUNREACH|ADDRINUSE|PIPE|ACCES|PERM)\b|\{:error\b|:(econnrefused|nxdomain|timeout|closed|enoent)\b|\b(HTTP|status) [1-5]\d\d\b|\b[A-Z][a-z]+([A-Z][a-z]+)*Error\b/
function internalWords(tree) {
  return tree.split("\n").filter((line) => INTERNAL.test(line) || CODE.test(line) || line.match(/[\w-]+_\w+/g)?.some((word) => tools.has(word))).map((line) => line.trim())
}

/// Presses what's named or says `name` on our renderer's canvas, as a person would: found in its ARIA tree
/// (clients/web/renderer/semantics.js), scrolled into sight where it isn't as a screen reader has the browser do it,
/// which the renderer reveals as the native harness's tap does (`Screen::reveal`: whole, with 16 points below it),
/// and pressed where it's drawn then.
async function press(frame, name) {
  const page = frame.page()
  if (name.key) return page.keyboard.press([...["ctrl", "shift", "alt", "meta"].filter((held) => name[held]).map((held) => ({ ctrl: "Control", shift: "Shift", alt: "Alt", meta: "Meta" })[held]), name.key].join("+"))
  if (name.type) return page.keyboard.type(name.type)
  const at = await page.locator("#page").boundingBox()
  if (name.x !== undefined) return page.mouse.click(at.x + name.x, at.y + name.y)
  const found = typeof name === "object" ? frame.getByRole(name.role, { name: name.name, exact: true }).first() : frame.locator(`#semantics [aria-label="${name.replaceAll('"', '\\"')}"], #semantics :text-is("${name.replaceAll('"', '\\"')}")`).last()
  let box = await found.boundingBox({ timeout: 5000 }).catch(() => null)
  const out = (/** @type {any} */ box) => box.y < at.y || box.y + box.height > at.y + at.height
  if (box && out(box)) {
    // The browser says it scrolled in its next frame, which the renderer then reveals, and the tree's drawn where it's
    // revealed to a frame or two later: it's pressed once it's there and settled, as Playwright's click waits for what
    // it clicks to be in view.
    const frames = () => frame.evaluate(() => new Promise((shown) => requestAnimationFrame(() => requestAnimationFrame(shown))))
    await found.scrollIntoViewIfNeeded({ timeout: 5000 })
    for (let tries = 0; box && out(box) && tries < 60; tries++) {
      await frames()
      box = await found.boundingBox({ timeout: 5000 }).catch(() => null)
    }
    await frame.evaluate((deadline) => /** @type {any} */ (globalThis).inspector.settle(deadline), SETTLES_WITHIN_MS)
    await frames()
    box = await found.boundingBox({ timeout: 5000 }).catch(() => null)
  }
  if (!box) throw new Error(`Nothing named or saying "${typeof name === "object" ? name.name : name}" to press`)
  // Its box is the page's, the frame's place in it already in it.
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2)
}

/// The longest the web half waits for a scene to settle before it takes its picture anyway, saying what's pending.
const SETTLES_WITHIN_MS = 10000

/// A scene drawn by our renderer on the web, in headless Chromium's WebGPU, as `web.png`, once its first frame is drawn
/// and what moves as it opens has settled, on the native harness's clock, language and time zone, each tap pressed
/// through its ARIA tree.
async function web(browser, serve, key, scene, folder) {
  const page = await browser.newPage({ viewport: { width: scene.size[0], height: scene.size[1] }, deviceScaleFactor: 2, hasTouch: scene.touch, colorScheme: dark ? "dark" : "light", locale: "en-US", timezoneId: scene.timeZone })
  const problems = []
  page.on("pageerror", (error) => problems.push(String(error)))
  page.on("console", (message) => message.type() === "warning" && !message.text().includes("preferred by this device") && problems.push(message.text()))
  await page.clock.setFixedTime(new Date(Date.UTC(2026, 0, 1)))
  // The first fetch of each picture file fails on the network, as a loaded or flaky connection's does: a picture is tried
  // again, and drawn when it comes, never left broken for a failure that passed.
  const failed = new Map()
  await page.route(/\.(png|jpe?g|gif|webp|svg)(\?.*)?$/, (route) => {
    const url = route.request().url()
    failed.set(url, (failed.get(url) ?? 0) + 1)
    return failed.get(url) === 1 ? route.abort("connectionreset") : route.continue()
  })
  serve.hostWeb(`${key} (web)`, scene)
  await page.goto(serve.url(`${key} (web)`))
  const frame = await (await page.waitForSelector("#page")).contentFrame()
  const drawn = await frame.waitForFunction(() => "drawn" in document.documentElement.dataset, null, { timeout: 20000 }).then(() => true, () => false)
  // Each picture is taken once the renderer says nothing is pending, as the native harness's `settle`, and the frame
  // it drew last is on the screen.
  const settled = async () => {
    const { settled, pending } = await frame.evaluate((deadline) => /** @type {any} */ (globalThis).inspector.settle(deadline), SETTLES_WITHIN_MS)
    if (!settled) {
      problems.push(`it didn't settle within ${SETTLES_WITHIN_MS} ms: ${JSON.stringify(pending)}`)
      process.exitCode = 1
    }
    await frame.evaluate(() => new Promise((shown) => requestAnimationFrame(() => requestAnimationFrame(shown))))
  }
  if (drawn) await settled()
  for (const name of drawn ? scene.taps : []) await press(frame, name).then(settled)
  for (const [url, times] of failed) {
    if (times > 1) continue
    problems.push(`${url} failed on the network once and wasn't fetched again, so it's drawn broken`)
    process.exitCode = 1
  }
  if (drawn) await page.screenshot({ path: join(folder, "web.png") })
  // What its screen reader reads, each name a line, as the native tree's are checked for words that aren't a person's.
  const words = drawn ? await frame.evaluate(() => [...document.querySelectorAll("#semantics *")].map((node) => node.getAttribute("aria-label") ?? "").filter(Boolean).join("\n")) : ""
  const jumped = drawn && scene.wheel && (await notched(page, frame, scene.wheel))
  if (jumped?.note) problems.push(jumped.note)
  else if (jumped) {
    problems.push(jumped)
    process.exitCode = 1
  }
  await page.close()
  return drawn ? { problems, internal: internalWords(words) } : { missing: `it drew nothing${problems.length ? `: ${problems.join("; ")}` : ""}` }
}

/// A mouse wheel's notch turned at `at`, as Chrome gives one (CDP's: pixels, marked a notch only by its legacy
/// `wheelDelta` of 120), which eases what it scrolls, as natively (a motion's `eases`): where the nodes of the ARIA tree
/// are, frame by frame over the second after it, what moved furthest hasn't moved all of the notch within a tenth of a
/// second of it, which an ease's spring takes longer than. One whose first move came later than that can't tell (a frame
/// SwiftShader, on the CPU, took long to draw), and is turned again, up to three times. → what's wrong, if it is.
async function notched(page, frame, at) {
  const box = await page.locator("#page").boundingBox()
  await page.mouse.move(box.x + at.x, box.y + at.y)
  for (let turns = 0; turns < 3; turns++) {
    // What the turn before moved has come to rest, so this one's moves are its own.
    await frame.evaluate((deadline) => /** @type {any} */ (globalThis).inspector.settle(deadline), SETTLES_WITHIN_MS)
    const watched = frame.evaluate(
      () =>
        new Promise((done) => {
          const where = () => new Map([...document.querySelectorAll("#semantics [aria-label], #semantics span")].map((node) => [node, node.getBoundingClientRect().y]))
          let [before, turned] = [where(), Infinity]
          addEventListener("wheel", (event) => (turned = Math.min(turned, event.timeStamp)), { capture: true, passive: true, once: true })
          const moves = []
          const since = performance.now()
          let frames = 0
          const frame = (/** @type {number} */ at) => {
            frames++
            const now = where()
            const by = [...now].reduce((most, [node, y]) => (before.has(node) && Math.abs(y - before.get(node)) > Math.abs(most) ? y - before.get(node) : most), 0)
            if (by) moves.push({ after: at - turned, by })
            before = now
            at - since < 1000 ? requestAnimationFrame(frame) : done({ moves, frames })
          }
          requestAnimationFrame(frame)
        }),
    )
    await page.mouse.wheel(0, 100)
    const { moves, frames } = await watched
    const all = Math.abs(moves.reduce((sum, move) => sum + move.by, 0))
    if (all < 1 || frames < 30) continue
    if (moves[0].after >= 100) continue
    return Math.abs(moves[0].by) >= all - 0.5 ? `a wheel's notch moved it ${all.toFixed(0)} px all at once, ${moves[0].after.toFixed(0)} ms after it, where it eases natively` : undefined
  }
  return { note: "a wheel's notch was turned 3 times and each time the page drew too few frames, or moved nothing within a tenth of a second, to tell whether it eased" }
}

// A round trip to a server, as a live network's take, and how soon a page opened should show its first content.
const LATENCY_MS = 150
const OPENS_WITHIN_MS = 300

// Every app's tools, named as a client calls them (`<app>_<tool>`), as each app's `tools/0` says, and those a client
// answers from what it keeps as a page opens: those that only read, as those and One Core's own definitions say
// (`read_only: true`).
const { tools, kept } = (() => {
  const [tools, kept] = [new Set(), new Set()]
  const read = (file, prefix) => {
    const text = readFileSync(file, "utf8")
    const named = [...text.matchAll(/name: "([\w-]+)"/g)]
    named.forEach((match, at) => {
      if (prefix) tools.add(prefix + match[1])
      if (/read_only: true/.test(text.slice(match.index, named[at + 1]?.index ?? text.length))) kept.add(prefix + match[1])
    })
  }
  read(join(root, "core/server/lib/one_core/tools/definitions.ex"), "")
  for (const app of readdirSync(join(root, "apps"))) {
    const server = join(root, "apps", app, "server")
    if (existsSync(server)) for (const file of readdirSync(server, { recursive: true })) if (String(file).endsWith(".ex")) read(join(server, String(file)), `${app}_`)
  }
  return { tools, kept: [...kept] }
})()

/// A scene drawn by a native host's headless harness, or nothing where it hasn't been built or can't draw the page.
function native(built, scene, folder, used) {
  if (!existsSync(harness)) return { missing: "the native harness isn't built (`cargo build -p renderer --example screenshot --features fake`)" }
  if (hosted < needed) return { missing: `the native host's runtime version is ${hosted}, the UI library needs ${needed}: nothing to compare until it's ${needed}` }
  const at = pageFolder(built, scene.page, folder, scene.uses)
  const answers = { ...scene.answers, "ui/initialize": scene.answers["ui/initialize"] ?? { hostContext: { link: scene.link, containerDimensions: { width: scene.size[0], height: scene.size[1] }, deviceCapabilities: { touch: scene.touch, hover: !scene.touch } } } }
  writeFileSync(join(folder, "answers.json"), JSON.stringify(answers))
  // The harness's clock stands still, so what moves as the page opens and after each tap is moved on to where it
  // settles, as the DOM's is, and what's laid out once it's drawn (onLayout) is drawn again.
  // With a device to draw it too, each ends at rest with where every node is and a picture drawn afresh, to compare.
  const requests = [{ op: "clock", step: 1000 }, ...scene.taps.flatMap((tap) => [tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...(typeof tap === "object" ? tap : { name: tap }) }, { op: "clock", step: 1000 }]), ...(scene.safeArea ? [{ op: "query" }] : []), ...(ssh ? DRAWN : [])].map((request, at) => JSON.stringify({ v: 1, id: at + 1, ...request })).join("\n")
  const asked = [at, join(folder, "native.png"), join(folder, "native.json"), "--ui", library("client"), "--answers", join(folder, "answers.json"), "--size", `${scene.size[0]}x${scene.size[1]}`, "--scale", "2", "--measure", "--latency", String(LATENCY_MS), "--cached", kept.join(","), ...(dark ? ["--dark"] : []), ...(scene.touch ? ["--touch"] : []), ...(scene.errors ? ["--errors", JSON.stringify(scene.errors)] : []), ...(scene.safeArea ? ["--safe-area", scene.safeArea.join(",")] : []), "--time-zone", scene.timeZone, "--requests", "-"]
  const run = Bun.spawnSync(capped([harness, ...asked]), { stdin: new TextEncoder().encode(requests), env })
  if (run.exitCode !== 0 || !existsSync(join(folder, "native.json"))) return { missing: stopped(run.exitCode) ?? (run.stderr.toString().trim().split("\n").at(-1) || "the harness drew nothing") }
  const nodes = JSON.parse(readFileSync(join(folder, "native.json"), "utf8"))
  // Its tree: each node's role, name and what it is (its states, by ARIA's names, beside them), and its words.
  const semantic = ({ id, role, name, children = [], actions, states = {}, ...more }) =>
    role === "text" ? { text: name ?? "", states: {}, children: [] } : { role: role ?? undefined, name: name ?? undefined, states: Object.fromEntries(Object.entries({ ...more, ...states }).map(([state, value]) => [state, String(value)])), children: children.map(semantic) }
  const tree = outline({ states: {}, children: (Array.isArray(nodes) ? nodes : [nodes]).map(semantic) })
  writeFileSync(join(folder, "native.txt"), tree)
  const measured = run.stderr.toString().match(/^measured (.*)$/m)
  const said = answered(run.stdout.toString())
  const unsafe = scene.safeArea && underTheDevice(said.at(ssh ? -DRAWN.length - 1 : -1)?.result?.nodes ?? [], scene.size[1], insets(scene.safeArea))
  return { tree, measured: measured && JSON.parse(measured[1]), unsafe, answers: said, at, asked, requests }
}

/// A phone's safe area, top, right, bottom and left, as an iPhone with a Dynamic Island has it upright (14 Pro to 17):
/// its status bar and its home indicator.
const IPHONE = [59, 0, 34, 0]
const insets = ([top, right, bottom, left]) => ({ top, right, bottom, left })
/// The narrowest a window is that isn't a phone's, in points: Material's compact width class ends there.
const COMPACT = 600

/// What a phone draws over a window's words and controls (the harness's `query` of every node, at rest): each drawn under
/// its status bar, or under its home indicator unless it's in what scrolls, which goes under it as UIKit's scroll views'
/// content does. Under the status bar, what's scrolled there is meant to be, as under a bar, and so is what covers the
// whole window, as a menu's backdrop that closes it does. → what's wrong, a line each.
function underTheDevice(nodes, height, { top, bottom }) {
  const byId = new Map(nodes.map((node) => [node.node, node]))
  const parent = new Map(nodes.flatMap((node) => (node.children ?? []).map((child) => [child, node])))
  const above = (node) => {
    const all = []
    for (let up = parent.get(node.node); up; up = parent.get(up.node)) all.push(up)
    return all
  }
  // What scrolls: a scroll view, or the window itself where what's in it is taller, as a browser's document is; and
  // whether it's scrolled, where what's in the one starts above it, or the other's top is above the window's.
  const scrolls = (node) => node.kind === "scroll" || (!parent.has(node.node) && node.rect?.height > height + 0.5)
  const scrolled = (node) => (node.kind === "scroll" ? (node.children ?? []).some((child) => byId.get(child)?.rect?.y < node.rect.y - 0.5) : node.rect.y < -0.5)
  const said = (node) => node.name ?? node.role ?? node.kind
  return nodes
    // A control, or words but a control's own, which it's said for.
    .filter((node) => node.rect && node.opacity > 0 && node.seen > 0 && (node.actions || (node.kind === "text" && !above(node).some((up) => up.actions))))
    .flatMap((node) => {
      const { y, height: tall } = node.rect
      const over = above(node).filter(scrolls)
      if (y <= 0.5 && y + tall >= height - 0.5) return []
      if (y < top - 0.5 && y + tall > 0 && !over.some(scrolled)) return [`"${said(node)}" is drawn at y ${Math.round(y)}, under the status bar, whose safe area ends at ${top}`]
      if (y + tall > height - bottom + 0.5 && y < height && !over.length) return [`"${said(node)}" is drawn down to y ${Math.round(y + tall)}, under the home indicator, whose safe area starts at ${height - bottom}`]
      return []
    })
}

/// What each scene ends with where a device draws it too: at rest, where each node is, and a picture drawn afresh.
const DRAWN = [{ op: "clock", step: 3000 }, { op: "query" }, { op: "snap" }]
const answered = (stdout) => stdout.split("\n").filter(Boolean).map((line) => JSON.parse(line))

/// A scene drawn on `--ssh`'s device by its own GPU, as here: where each node is laid out there and here, and how much
/// of the two pictures differs, beyond what one GPU's rounding does differently from another's. → what it says.
async function onDevice(drawn, folder) {
  const there = await runThere(drawn.at, folder, drawn.asked, drawn.requests)
  const theirs = answered(there.stdout)
  if (there.failed || theirs.length < DRAWN.length) return [`  device: it drew nothing: ${there.stderr.trim().split("\n").at(-1)}`]
  const [[, ours, mine], [, its, picture]] = [drawn.answers, theirs].map((answers) => answers.slice(-DRAWN.length))
  const said = []
  const [here, away] = [ours, its].map((answer) => answer?.result?.nodes ?? [])
  const box = ({ rect }) => (rect ? `${rect.x},${rect.y} ${rect.width}×${rect.height}` : "nowhere")
  const moved = here.filter((node, at) => !away[at] || ["x", "y", "width", "height"].some((edge) => Math.abs((node.rect?.[edge] ?? 0) - (away[at].rect?.[edge] ?? 0)) > 0.5))
  if (here.length !== away.length) said.push(`  device: ${away.length} nodes there, ${here.length} here`)
  if (moved.length) said.push(`  device: ${moved.length} nodes laid out elsewhere, as ${moved.slice(0, 3).map((node) => `${node.role ?? node.kind} "${node.name ?? ""}" at ${box(node)} here, ${box(away[here.indexOf(node)] ?? {})} there`).join("; ")}`)
  if (!mine?.result?.png || !picture?.result?.png) return [...said, `  device: no picture: ${picture?.error ?? mine?.error}`]
  writeFileSync(join(folder, "afresh.png"), Buffer.from(mine.result.png, "base64"))
  writeFileSync(join(folder, "device.png"), Buffer.from(picture.result.png, "base64"))
  const part = differs(join(folder, "afresh.png"), join(folder, "device.png"), join(folder, "device-diff.png"))
  said.push(`  here and on the device: pictures ${part === undefined ? "aren't the same size" : `${(part * 100).toFixed(1)}% different${part > 0.005 ? " (over 0.5%)" : ""}`}`)
  return said
}

/// How quickly the native host drew a scene, as a line.
const timings = ({ first_frame_ms, first_content_ms, worst_paint_ms, idle_ms, typing_ms, scrolling_ms, memory_mb, window_mb }) =>
  [`first frame ${first_frame_ms} ms`, first_content_ms != null && `first content ${first_content_ms} ms`, `worst frame ${worst_paint_ms} ms on its thread`, `frame ${idle_ms} ms idle`, typing_ms != null && `${typing_ms} ms typing`, scrolling_ms != null && `${scrolling_ms} ms scrolling`, `${memory_mb} MB`, `${window_mb} MB a window more`].filter(Boolean).join(", ")

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

/// The most of a scene's web picture that may differ from its native one, from 0 to 1, else the scene fails: our
/// renderer draws both, so more is the web platform's fault, as the widgets' shapes missing from the web were (L30).
const WEB_DIFFERS_MOST = 0.01
/// Scenes whose web picture differs more for a known reason that isn't one, by `<folder> · <scene>`, each with why.
const WEB_DIFFERS = {}

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
  const scenes = existsSync(join(target, "client/scenes.json")) ? JSON.parse(readFileSync(join(target, "client/scenes.json"), "utf8")) : {}
  const unbuilt = build(target, built)
  if (unbuilt) {
    report.push(`${name}: ${unbuilt}`)
    continue
  }
  // The apps whose components its scenes draw, each built once and kept in `out`.
  const used = new Map()
  for (const app of new Set(Object.values(scenes).flatMap((scene) => scene.uses ?? []))) used.set(app, usedApp(app, out))
  const serve = server(built, target, used)
  for (const scene of (await scenesOf(target, built)).filter((scene) => !only || scene.name.toLowerCase().includes(only))) {
    const folder = join(out, name, scene.name.replace(/[^\w.-]+/g, "-"))
    mkdirSync(folder, { recursive: true })
    const drawn = native(built, scene, folder, used)
    const onWeb = !onTheWeb ? {} : await web(webgpu, serve, `${name}/${scene.name}`, scene, folder).catch(async (error) => {
      await webgpu.close().catch(() => {})
      webgpu = await chromium.launch({ args: webgpuArgs })
      return { missing: String(error) }
    })
    const said = [`${name} · ${scene.name}`]
    // A row is drawn in its card by the `List` it's in: one outside any has none, and floats on its screen's background.
    if (name !== "gallery" && drawn.tree !== undefined) for (const row of strayRows(drawn.tree)) said.push(`  a row isn't in a List, so it has no card: ${row}`)
    // A person reads a scene's words: nothing of the build's or the platform's is among them.
    for (const line of drawn.tree === undefined ? [] : internalWords(drawn.tree)) {
      said.push(`  the screen says a word that isn't a person's: ${line}`)
      process.exitCode = 1
    }
    // A page that threw while drawing shows `render`'s Problem, whose scene says nothing of the page.
    if (drawn.tree?.includes(messages.cantShowPage)) {
      said.push(`  native: the page crashed, showing "${messages.cantShowPage}"`)
      process.exitCode = 1
    }
    for (const line of drawn.unsafe ?? []) {
      said.push(`  on a phone: ${line}`)
      process.exitCode = 1
    }
    if (drawn.missing) said.push(`  native: ${drawn.missing}`)
    if (drawn.measured) said.push(`  native: ${timings(drawn.measured)}`)
    if (drawn.measured?.first_content_ms > OPENS_WITHIN_MS) {
      said.push(`  opens slowly: its first content came ${drawn.measured.first_content_ms} ms after it opened, over ${OPENS_WITHIN_MS} ms, with each answer a client doesn't keep ${LATENCY_MS} ms away`)
      process.exitCode = 1
    }
    if (ssh && !drawn.missing) said.push(...(await onDevice(drawn, folder)))
    for (const problem of onWeb.problems ?? []) said.push(`  web: ${problem}`)
    for (const line of onWeb.internal ?? []) {
      said.push(`  web: the screen says a word that isn't a person's: ${line}`)
      process.exitCode = 1
    }
    const excused = WEB_DIFFERS[`${name} · ${scene.name}`]
    if (onWeb.missing) said.push(`  web: ${onWeb.missing}`)
    else if (onTheWeb && !drawn.missing) {
      const part = differs(join(folder, "native.png"), join(folder, "web.png"), join(folder, "web-diff.png"))
      const over = part === undefined || part > WEB_DIFFERS_MOST
      said.push(`  native and web: pictures ${part === undefined ? "aren't the same size" : `${(part * 100).toFixed(1)}% different${over ? ` (over ${WEB_DIFFERS_MOST * 100}%${excused ? `, known: ${excused}` : ""})` : ""}`}`)
      if (over && !excused) process.exitCode = 1
    }
    const before = against && join(against, name, basename(folder))
    // What changed, where our renderer drew the scene both times.
    if (drawn.tree === undefined) {
    } else if (before && existsSync(join(before, "native.png"))) compared(said, "before and now", readFileSync(join(before, "native.txt"), "utf8"), drawn.tree, join(before, "native.png"), join(folder, "native.png"), join(folder, "changed.png"))
    else if (before) said.push("  before and now: no scene of this name before")
    said.push(`  ${folder}`)
    report.push(said.join("\n"))
  }
  serve.stop()
}
if (webgpu) await webgpu.close()
console.log(report.join("\n"))
