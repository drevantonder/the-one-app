// An app's page drawn by the renderer (clients/app/renderer), as a computer's and a phone's windows draw it, in this
// frame of the web client's: the renderer is WebAssembly on the browser's WebGPU (`renderer::platform::web`), and the
// page's JS is this frame's own, which draws through it as a native host's sandbox does (`globalThis.mutations`, the
// UI library's native build). The frame's address names the page's (`#<its address>`) and the runtime version it
// needs (`runtime`), as the web client's `address` has it. A classic script, so the import map it writes comes before
// any module loads.
;(async () => {
  const started = performance.now()
  const page = new URL(location.hash.slice(1), location.href)
  /// What happened, said once of each kind to the frame's client, which may tell Telemetry (check.js): the first
  /// frame, the edits its real field had, a screen reader's presses.
  /** @type {Set<string>} */
  const said = new Set()
  const seen = (/** @type {string} */ kind, /** @type {unknown} */ value = true) => {
    if (said.has(kind)) return
    said.add(kind)
    parent.postMessage({ jsonrpc: "2.0", method: "notifications/one/seen", params: { [kind]: value } }, "*")
  }
  /** What the page's own page is drawn as, where this can't: the DOM. */
  const unopened = (/** @type {unknown} */ why) => {
    console.warn("the renderer didn't open this page:", why)
    seen("unopened", String(why))
    location.replace(page)
  }
  try {
    if (!navigator.gpu) throw new Error("this browser has no WebGPU")
    // The page's own: where what it names is from, the module it loads, and the import map a server serves it with.
    const html = new DOMParser().parseFromString(await (await fetch(page)).text(), "text/html")
    const base = new URL(html.querySelector("base")?.getAttribute("href") ?? page.href, page)
    const script = html.querySelector('script[type="module"][src]')?.getAttribute("src")
    if (!script) throw new Error("its page loads no module")
    /** @type {Record<string, string>} */
    const imports = JSON.parse(html.querySelector('script[type="importmap"]')?.textContent || "{}").imports ?? {}
    // The UI library's renderer and components are those built for a native host, as a native host's loader has them,
    // whether a module names them by where they're served or by name.
    const native = Object.fromEntries(["web.js", "ui.js"].map((file) => [new URL(`/ui/client/${file}`, base).href, new URL(`/ui/client/native/${file}`, base).href]))
    for (const [name, to] of Object.entries(imports)) imports[name] = native[new URL(to, base).href] ?? new URL(to, base).href
    Object.assign(imports, native)
    document.head.append(Object.assign(document.createElement("script"), { type: "importmap", textContent: JSON.stringify({ imports }) }))
    // Its tables: the UI library's tokens and styles, and its app's styles, which its build writes beside every page.
    const tables = await Promise.all(["/ui/client/tokens.json", "/ui/client/styles.json", "styles.json"].map(async (table) => (await fetch(new URL(table, base))).text()))
    const [renderer, { textInput }, { semantics }, { videos }] = await Promise.all([import("./web_renderer.js"), import("./text-input.js"), import("./semantics.js"), import("./video.js")])
    await renderer.default()
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
        unsupported: renderer.unsupported,
        frame: false,
        video: true,
      },
      host: { post: (/** @type {object} */ message) => renderer.post(JSON.stringify(message)) },
    })
    const canvas = /** @type {HTMLCanvasElement} */ (document.querySelector("canvas"))
    // Typing, through a real field (text-input.js), and what a screen reader reads (semantics.js).
    // What the pointer does on the field typed in is the canvas's, as it would be with no field over it.
    const pointed = (/** @type {PointerEvent | WheelEvent} */ event) => canvas.dispatchEvent(event instanceof WheelEvent ? new WheelEvent(event.type, event) : new PointerEvent(event.type, event))
    const typing = textInput((method, args) => renderer.text_input_client(method, args), (edit) => seen(`edit:${edit}`), pointed)
    const spoken = semantics(/** @type {HTMLElement} */ (document.getElementById("semantics")), (id, action) => (seen(`screen reader:${action}`), renderer.act(id, action)))
    const sized = (/** @type {number} */ width, /** @type {number} */ height) => [Math.max(1, Math.round(width)), Math.max(1, Math.round(height))]
    ;[canvas.width, canvas.height] = sized(innerWidth * devicePixelRatio, innerHeight * devicePixelRatio)
    await renderer.open(canvas, devicePixelRatio, base.href, decodeURIComponent(page.hash.slice(1)), tables, page.searchParams.get("theme"), matchMedia("(pointer: coarse)").matches, {
      post: (/** @type {object} */ message) => parent.postMessage(message, "*"),
      cursor: (/** @type {string} */ cursor) => void (canvas.style.cursor = cursor),
      textInput: (/** @type {[string, unknown][]} */ calls) => calls.forEach(([method, args]) => typing.call(method, args)),
      semantics: spoken,
      // Its first frame, marked on the frame's own element for whoever measures it.
      drawn: () => {
        document.documentElement.dataset.drawn = String(Math.round(performance.now()))
        seen("first frame ms", Math.round(performance.now()))
        console.info(`first frame ${Math.round(performance.now() - started)} ms after the frame's script, ${Math.round(performance.now())} ms after it was opened`)
      },
      unopened,
    })
    addEventListener("message", ({ source, data }) => source === parent && renderer.message(data))
    // The canvas as the DOM says it is, in device pixels, and how many to a CSS pixel (the display's and the zoom),
    // and what happens on it.
    new ResizeObserver(([entry]) => {
      const [box] = entry.devicePixelContentBoxSize ?? [{ inlineSize: entry.contentRect.width * devicePixelRatio, blockSize: entry.contentRect.height * devicePixelRatio }]
      if (entry.contentRect.width) renderer.resized(...sized(box.inlineSize, box.blockSize), box.inlineSize / entry.contentRect.width)
    }).observe(canvas)
    const pointer = (/** @type {string} */ kind) => (/** @type {PointerEvent} */ event) => (streams.pointer(kind, event), renderer.pointer(kind, event.offsetX, event.offsetY, event.button, event.shiftKey))
    canvas.addEventListener("pointermove", pointer("move"))
    // A press takes the keyboard for the frame itself, not by the browser's default after it, which would take it
    // back from a field the press gave it to (the screen gives a field the keyboard as it's pressed).
    canvas.addEventListener("pointerdown", (event) => {
      event.preventDefault()
      if (document.activeElement !== typing.field) canvas.focus({ preventScroll: true })
      if (event.isTrusted) canvas.setPointerCapture(event.pointerId)
      pointer("down")(event)
    })
    // What a press gives the keyboard is focused in the press itself, which is when a phone brings up its keyboard.
    canvas.addEventListener("pointerup", (event) => (pointer("up")(event), renderer.flush()))
    canvas.addEventListener("pointerleave", pointer("leave"))
    // A wheel's lines and pages, in CSS pixels, as a browser scrolls them.
    const unit = [1, 16, innerHeight]
    // A wheel over a video turns the screen's, not the page.
    canvas.addEventListener("wheel", (event) => (event.preventDefault(), streams.wheel(event, unit[event.deltaMode]) || renderer.wheel(event.offsetX, event.offsetY, event.deltaX * unit[event.deltaMode], event.deltaY * unit[event.deltaMode])), { passive: false })
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
      renderer.started()
    } catch (error) {
      renderer.started(String(/** @type {Error} */ (error)?.stack ?? error))
    }
  } catch (error) {
    unopened(error)
  }
})()
