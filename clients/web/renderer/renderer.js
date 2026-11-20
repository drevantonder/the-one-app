// An app's page drawn by the renderer (clients/app/renderer), as a computer's and a phone's windows draw it, in this
// frame of the web client's: the renderer is WebAssembly on the browser's WebGPU (`renderer::platform::web`), and the
// page's JS is this frame's own, which draws through it as a native host's sandbox does (`globalThis.mutations`, the
// UI library's native build). The frame's address names the page's (`#<its address>`) and the runtime version it
// needs (`runtime`), as the web client's `address` has it. A classic script, so the import map it writes comes before
// any module loads.
;(async () => {
  const started = performance.now()
  const page = new URL(location.hash.slice(1), location.href)
  // A frame's page (the UI library's `Frame`), another app's in one of the page's, which the frame's page draws, as a
  // native host's frame is drawn in its page's window: this is only its JS, as a native host's runs on a thread of its
  // own. Its client knows it by its number (`frame`), as its own app's.
  const framed = new URLSearchParams(location.search).get("frame")
  const pageHtml = fetch(page).then((response) => response.text())
  if (framed) return frame(page, pageHtml, framed)
  // What takes longest starts at once, each beside the others: the renderer, downloaded and compiled as it comes (and
  // preloaded by index.html), the page's own HTML, and the UI library's tables, which are at the network's root.
  const here = /** @type {HTMLScriptElement} */ (document.currentScript).src
  const compiled = WebAssembly.compileStreaming(fetch(new URL("web_renderer_bg.wasm", here)))
  const libraryTables = Promise.all(["/ui/client/tokens.json", "/ui/client/styles.json"].map(async (table) => (await fetch(new URL(table, page))).text()))
  /// How long each step took, from the frame's script, which its first frame tells (`seen`).
  /** @type {Record<string, number>} */
  const took = {}
  const step = (/** @type {string} */ name) => void (took[name] = Math.round(performance.now() - started))
  /// What happened, said once of each kind to the frame's client, which may tell Telemetry (check.js): the first
  /// frame, the edits its real field had, a screen reader's presses.
  /** @type {Set<string>} */
  const said = new Set()
  const seen = (/** @type {string} */ kind, /** @type {unknown} */ value = true) => {
    if (said.has(kind)) return
    said.add(kind)
    parent.postMessage({ jsonrpc: "2.0", method: "notifications/one/seen", params: { [kind]: value } }, "*")
  }
  /** Where the renderer can't open the page: its client shows that it didn't open, and why. */
  const unopened = (/** @type {unknown} */ why) => {
    console.warn("the renderer didn't open this page:", why)
    seen("unopened", String(why))
  }
  try {
    if (!navigator.gpu) throw new Error("this browser has no WebGPU")
    const { base, script } = mapped(page, await pageHtml)
    step("page")
    // Its tables: the UI library's tokens and styles, and its app's styles, which its build writes beside every page.
    const [tables, [renderer, { textInput }, { semantics }, { videos }]] = await Promise.all([
      Promise.all([libraryTables, fetch(new URL("styles.json", base)).then((response) => response.text())]).then(([library, own]) => (step("tables"), [...library, own])),
      Promise.all([import("./web_renderer.js"), import("./text-input.js"), import("./semantics.js"), import("./video.js")]),
    ])
    await renderer.default({ module_or_path: await compiled.then((module) => (step("compiled"), module)) })
    step("renderer")
    // A page built with this renderer's own UI library (check/) names none.
    const needs = Number(new URLSearchParams(location.search).get("runtime"))
    if (!(needs <= renderer.runtime_version())) throw new Error(`it needs runtime version ${needs}`)
    // What the page's JS draws through, and asks its client through, as a native host's sandbox has them; its videos
    // are the browser's to connect and play too (video.js).
    const streams = videos(renderer)
    Object.assign(globalThis, {
      mutations: {
        create: (/** @type {number} */ id, /** @type {string} */ type, /** @type {string} */ value) => (type === "video" && streams.created(id), renderer.create(id, type, value)),
        set: (/** @type {number} */ id, /** @type {string} */ name, /** @type {unknown} */ value) => (streams.set(id, name, value), renderer.set(id, name, value)),
        insert: renderer.insert,
        remove: renderer.remove,
        drop: (/** @type {number} */ id) => (streams.dropped(id), renderer.drop(id)),
        frame: true,
        list: 2,
        video: true,
      },
      host: { post: (/** @type {object} */ message) => renderer.post(JSON.stringify(message)) },
    })
    const canvas = /** @type {HTMLCanvasElement} */ (document.querySelector("canvas"))
    // Typing, through a real field (text-input.js), and what a screen reader reads (semantics.js).
    // What the pointer does on the field typed in is the canvas's, as it would be with no field over it.
    const pointed = (/** @type {PointerEvent | WheelEvent} */ event) => canvas.dispatchEvent(event instanceof WheelEvent ? new WheelEvent(event.type, event) : new PointerEvent(event.type, event))
    const typing = textInput((method, args) => renderer.text_input_client(method, args), (edit) => seen(`edit:${edit}`), pointed)
    // What's selected of a text that isn't a field, as the page's own selection, so the browser's Copy (its key, and its
    // menu where the mirror is under a right click) copies it, as a web page's text is copied.
    const selection = /** @type {HTMLElement} */ (document.getElementById("selection"))
    const mirror = (/** @type {string | null} */ text) => {
      selection.textContent = text ?? ""
      if (text && document.activeElement !== typing.field) getSelection()?.selectAllChildren(selection)
      else if (getSelection()?.anchorNode && selection.contains(getSelection()?.anchorNode ?? null)) getSelection()?.removeAllRanges()
    }
    const spoken = semantics(/** @type {HTMLElement} */ (document.getElementById("semantics")), (id, action) => (seen(`screen reader:${action}`), renderer.act(id, action)))
    const sized = (/** @type {number} */ width, /** @type {number} */ height) => [Math.max(1, Math.round(width)), Math.max(1, Math.round(height))]
    ;[canvas.width, canvas.height] = sized(innerWidth * devicePixelRatio, innerHeight * devicePixelRatio)
    await renderer.open(canvas, devicePixelRatio, base.href, decodeURIComponent(page.hash.slice(1)), tables, page.searchParams.get("theme"), matchMedia("(pointer: coarse)").matches, {
      post: (/** @type {object} */ message) => parent.postMessage(message, "*"),
      cursor: (/** @type {string} */ cursor) => void (canvas.style.cursor = cursor),
      textInput: (/** @type {[string, unknown][]} */ calls) => calls.forEach(([method, args]) => typing.call(method, args)),
      semantics: spoken,
      selected: (/** @type {string | null} */ text) => mirror(text),
      ...frames(renderer, (message) => parent.postMessage(message, "*")),
      // Its first frame, marked on the frame's own element for whoever measures it.
      drawn: () => {
        document.documentElement.dataset.drawn = String(Math.round(performance.now()))
        seen("first frame ms", Math.round(performance.now()))
        const wasm = /** @type {PerformanceResourceTiming | undefined} */ (performance.getEntriesByName(new URL("web_renderer_bg.wasm", here).href)[0])
        step("drawn")
        seen("took", { ...took, script: Math.round(started), wasmDownload: wasm && Math.round(wasm.responseEnd - wasm.startTime), wasmBytes: wasm?.transferSize })
        console.info(`first frame ${Math.round(performance.now() - started)} ms after the frame's script, ${Math.round(performance.now())} ms after it was opened`)
      },
      unopened,
    })
    step("opened")
    addEventListener("message", ({ source, data }) => source === parent && !answered(data) && renderer.message(data))
    // The canvas as the DOM says it is, in device pixels, and how many to a CSS pixel (the display's and the zoom),
    // and what happens on it.
    new ResizeObserver(([entry]) => {
      const [box] = entry.devicePixelContentBoxSize ?? [{ inlineSize: entry.contentRect.width * devicePixelRatio, blockSize: entry.contentRect.height * devicePixelRatio }]
      if (entry.contentRect.width) renderer.resized(...sized(box.inlineSize, box.blockSize), box.inlineSize / entry.contentRect.width)
    }).observe(canvas)
    // A finger is the renderer's to pan with, as a phone's app has it (`touch`: past the slop it scrolls what's under it,
    // and flings as it's let go); a mouse and a pen point and press.
    const pointer = (/** @type {string} */ kind) => (/** @type {PointerEvent} */ event) =>
      event.pointerType === "touch" ? kind !== "leave" && renderer.touch(kind, event.offsetX, event.offsetY, event.pointerId) : (streams.pointer(kind, event), renderer.pointer(kind, event.offsetX, event.offsetY, event.button, event.shiftKey))
    canvas.addEventListener("pointermove", (event) => {
      pointer("move")(event)
      // What's dragged is drawn where the browser predicts the pointer is going, so it keeps up with a finger.
      const ahead = event.buttons ? event.getPredictedEvents?.().at(-1) : undefined
      if (ahead) renderer.predicted(ahead.offsetX, ahead.offsetY)
    })
    // A press takes the keyboard for the frame itself, not by the browser's default after it, which would take it
    // back from a field the press gave it to (the screen gives a field the keyboard as it's pressed).
    canvas.addEventListener("pointerdown", (event) => {
      event.preventDefault()
      // A right click over what's selected is the mirror's, so the browser's menu has Copy; any other press puts it away.
      const right = event.button === 2 && !!selection.textContent
      Object.assign(selection.style, right ? { left: `${event.clientX - 4}px`, top: `${event.clientY - 8}px`, pointerEvents: "auto" } : { left: "-9999px", pointerEvents: "none" })
      if (right) getSelection()?.selectAllChildren(selection)
      if (document.activeElement !== typing.field) canvas.focus({ preventScroll: true })
      if (event.isTrusted) canvas.setPointerCapture(event.pointerId)
      pointer("down")(event)
    })
    // What a press gives the keyboard is focused in the press itself, which is when a phone brings up its keyboard.
    canvas.addEventListener("pointerup", (event) => (pointer("up")(event), renderer.flush()))
    canvas.addEventListener("pointerleave", pointer("leave"))
    canvas.addEventListener("pointercancel", pointer("cancel"))
    // A wheel's lines and pages, in CSS pixels, as a browser scrolls them; one that says lines (Firefox's) turned by
    // notches, an event each.
    const unit = [1, 16, innerHeight]
    const notch = (event, by) => (event.deltaMode === WheelEvent.DOM_DELTA_LINE ? Math.sign(by) : 0)
    // A wheel over a video turns the screen's, not the page.
    canvas.addEventListener("wheel", (event) => (event.preventDefault(), streams.wheel(event, unit[event.deltaMode]) || renderer.wheel(event.offsetX, event.offsetY, event.deltaX * unit[event.deltaMode], event.deltaY * unit[event.deltaMode], notch(event, event.deltaX), notch(event, event.deltaY))), { passive: false })
    // A key is the page's first, as a Mac's are (`platform::mac`): one that types nothing, or with ⌘ or Ctrl held.
    // What types something goes to the field being typed in, and every key while its input method composes, and so do
    // copy, cut and paste, so they're the browser's own clipboard's.
    addEventListener("keydown", (event) => {
      if (event.defaultPrevented) return
      const field = typing.field && document.activeElement === typing.field
      if (field && (typing.composing || event.isComposing || event.keyCode === 229)) return
      const chord = event.metaKey || event.ctrlKey
      if (field && !chord && [...event.key].length === 1) return
      if (field && chord && ["c", "x", "v"].includes(event.key.toLowerCase())) return
      // A key while a video was last pressed is its screen's.
      if (!field && streams.key(event)) return event.preventDefault()
      if (field || event.key === "Tab" || event.key === " " || event.key.startsWith("Arrow") || (chord && event.key.length === 1)) event.preventDefault()
      renderer.key(event.key, event.shiftKey, event.altKey, event.ctrlKey, event.metaKey)
    })
    addEventListener("focus", () => renderer.focused(true))
    addEventListener("blur", () => renderer.focused(false))
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", ({ matches }) => renderer.appearance_changed(matches))
    canvas.focus()
    // The page's place in itself, a link's fragment, which its client names after its address in the frame's
    // (client.js's `load`), and which a native host tells a page as its host context's `link`: as it opens, and as its
    // client takes it somewhere else in it.
    addEventListener("hashchange", () => {
      const link = decodeURIComponent(new URL(location.hash.slice(1), location.href).hash.slice(1))
      renderer.message({ jsonrpc: "2.0", method: "ui/notifications/host-context-changed", params: { link } })
    })
    try {
      await import(new URL(script, base).href)
      step("module")
      renderer.started()
    } catch (error) {
      renderer.started(String(/** @type {Error} */ (error)?.stack ?? error))
    }
  } catch (error) {
    unopened(error)
  }
})()

/**
 * The page's own HTML, `html`, at `page`: the import map a server serves it with written here, before any module
 * loads, and its module and the UI library it imports fetched now, beside the renderer, rather than each as the one
 * before it has loaded and named it. → where what it names is from, and the module it loads.
 * @param {URL} page @param {string} html
 */
function mapped(page, html) {
  const parsed = new DOMParser().parseFromString(html, "text/html")
  const base = new URL(parsed.querySelector("base")?.getAttribute("href") ?? page.href, page)
  const script = parsed.querySelector('script[type="module"][src]')?.getAttribute("src")
  if (!script) throw new Error("its page loads no module")
  /** @type {Record<string, string>} */
  const imports = JSON.parse(parsed.querySelector('script[type="importmap"]')?.textContent || "{}").imports ?? {}
  // The UI library's renderer and components are those built for a native host, as a native host's loader has them,
  // whether a module names them by where they're served or by name.
  const native = Object.fromEntries(["web.js", "ui.js"].map((file) => [new URL(`/ui/client/${file}`, base).href, new URL(`/ui/client/native/${file}`, base).href]))
  for (const [name, to] of Object.entries(imports)) imports[name] = native[new URL(to, base).href] ?? new URL(to, base).href
  Object.assign(imports, native)
  document.head.append(Object.assign(document.createElement("script"), { type: "importmap", textContent: JSON.stringify({ imports }) }))
  for (const module of [new URL(script, base).href, ...new Set(Object.values(imports).filter((to) => to.endsWith(".js")))]) document.head.append(Object.assign(document.createElement("link"), { rel: "modulepreload", href: module, crossOrigin: "anonymous" }))
  return { base, script }
}

/// What the frame's page asked its client itself (a frame's place), by its id, each answered here, not by the page.
/** @type {Map<string, (result: any) => void>} */
const asking = new Map()
/** @param {any} data → whether it answered what this asked. */
const answered = (data) => {
  const done = typeof data?.id === "string" && data.method === undefined && asking.get(data.id)
  if (done) (asking.delete(data.id), done(data.result))
  return Boolean(done)
}

/**
 * The frames of the page drawn here (`Frame`), each another app's page whose JS runs in a frame of its own of this
 * one's, unseen: where its client says it is (`one/frame`), as a native host starts a frame's page in a sandbox of its
 * own (`clients/app/src/native`). What it gives and hears passes between it and its screen, by its frame's node.
 * @param {any} renderer @param {(message: object) => void} post its client's
 */
function frames(renderer, post) {
  /** @type {Map<number, { iframe: HTMLIFrameElement, frame?: string, src: string }>} */
  const shown = new Map()
  const ask = (/** @type {string} */ method, /** @type {object} */ params) =>
    new Promise((resolve) => {
      const id = `frame:${crypto.randomUUID()}`
      asking.set(id, resolve)
      post({ jsonrpc: "2.0", id, method, params })
    })
  const to = (/** @type {number} */ node, /** @type {object} */ message) => shown.get(node)?.iframe.contentWindow?.postMessage(message, "*")
  addEventListener("message", ({ source, data }) => {
    const [node, one] = [...shown].find(([, one]) => source && one.iframe.contentWindow === source) ?? []
    if (node === undefined || !one) return
    if (data?.one === "page") renderer.framed(node, one.src, data.base, data.styles)
    else if (data?.one === "posted") renderer.frame_posted(node, data.calls, data.ran, data.framing)
    else if (data?.one === "started") renderer.frame_started(node, data.error)
    else if (data?.one === "heard") renderer.frame_message(node, data.message)
  })
  return {
    frame: async (/** @type {[number, string, string | null]} */ [node, src, subject]) => {
      const iframe = document.createElement("iframe")
      shown.set(node, { iframe, src })
      const { url, sandbox, frame } = await ask("one/frame", { src, ...(subject && { subject }) })
      if (shown.get(node)?.iframe !== iframe) return void ask("one/frame", { forget: frame })
      Object.assign(shown.get(node) ?? {}, { frame })
      iframe.setAttribute("sandbox", sandbox)
      iframe.setAttribute("aria-hidden", "true")
      Object.assign(iframe.style, { position: "fixed", width: "0", height: "0", border: "0", opacity: "0", pointerEvents: "none" })
      iframe.src = url
      document.body.append(iframe)
    },
    unframe: (/** @type {number} */ node) => {
      const one = shown.get(node)
      shown.delete(node)
      one?.iframe.remove()
      if (one?.frame) void ask("one/frame", { forget: one.frame })
    },
    frameCall: (/** @type {[number, string, unknown]} */ [node, to_, message]) => to(node, { one: "call", to: to_, message }),
    frameAsk: (/** @type {[number, unknown]} */ [node, message]) => to(node, { one: "ask", message }),
    frameFrame: (/** @type {[number, number]} */ [node, time]) => to(node, { one: "frame", time }),
  }
}

/**
 * A frame's page, `page`, run here for the frame's page that draws it (`frames`): its JS gives its host what it would
 * give a native host's (`globalThis.mutations`, `host.post`), which is passed to that page, a turn's at once, with how
 * many of the calls it was sent it's run, as the frame that draws it calls it (`ui.event`, `ui.message`) with what
 * its screen says. What it asks its client goes to its client as this frame's, by its number, and what its client
 * says comes back through its screen. Its `requestAnimationFrame` is its screen's, as a native host's page's is.
 * @param {URL} page @param {Promise<string>} pageHtml @param {string} framed its number
 */
async function frame(page, pageHtml, framed) {
  const drawing = parent
  /** @type {unknown[][]} */
  let calls = []
  let [ran, due] = [0, false]
  /** @type {Map<number, FrameRequestCallback>} */
  const asked = new Map()
  let next = 1
  const give = () => {
    due = false
    drawing.postMessage({ one: "posted", calls: calls.splice(0), ran, framing: asked.size > 0 }, "*")
  }
  const soon = () => void (due || ((due = true), queueMicrotask(give)))
  const push = (/** @type {unknown[]} */ call) => (calls.push(call), soon())
  // A value as JSON's data model, as a native host's sandbox reads one: what holds itself is the page's error.
  const plain = (/** @type {unknown} */ value) => {
    const json = JSON.stringify(value)
    return json === undefined ? null : JSON.parse(json)
  }
  Object.assign(globalThis, {
    mutations: {
      create: (/** @type {number} */ id, /** @type {string} */ type, /** @type {string} */ value) => push(["create", id, type, value ?? null]),
      set: (/** @type {number} */ id, /** @type {string} */ name, /** @type {unknown} */ value) => push(["set", id, name, plain(value)]),
      insert: (/** @type {number} */ parent, /** @type {number} */ id, /** @type {number | undefined} */ before) => push(["insert", parent, id, before ?? null]),
      remove: (/** @type {number} */ parent, /** @type {number} */ id) => push(["remove", parent, id]),
      drop: (/** @type {number} */ id) => push(["drop", id]),
      frame: false,
      list: 2,
      video: false,
    },
    host: { post: (/** @type {object} */ message) => push(["post", JSON.stringify(message)]) },
    requestAnimationFrame: (/** @type {FrameRequestCallback} */ callback) => (asked.set(next, callback), soon(), next++),
    cancelAnimationFrame: (/** @type {number} */ number) => void asked.delete(number),
  })
  addEventListener("message", ({ source, data }) => {
    if (source === drawing && data?.one === "call") {
      try {
        /** @type {any} */ (globalThis).ui?.[data.to]?.(data.message)
      } catch (error) {
        push(["threw", String(/** @type {Error} */ (error)?.stack ?? error)])
      }
      ran++
      soon()
    } else if (source === drawing && data?.one === "ask") top?.postMessage({ ...data.message, frame: framed }, "*")
    else if (source === drawing && data?.one === "frame") {
      const due = [...asked.values()]
      asked.clear()
      for (const callback of due) callback(data.time)
      soon()
    } else if (source === top) drawing.postMessage({ one: "heard", message: data }, "*")
  })
  try {
    const { base, script } = mapped(page, await pageHtml)
    const styles = await (await fetch(new URL("styles.json", base))).text()
    drawing.postMessage({ one: "page", base: base.href, styles }, "*")
    await import(new URL(script, base).href)
    give()
    drawing.postMessage({ one: "started" }, "*")
  } catch (error) {
    drawing.postMessage({ one: "started", error: String(/** @type {Error} */ (error)?.stack ?? error) }, "*")
  }
}
