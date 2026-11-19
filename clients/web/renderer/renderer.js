// An app's page drawn by the renderer (clients/app/renderer), as a computer's and a phone's windows draw it, in this
// frame of the web client's: the renderer is WebAssembly on the browser's WebGPU (`renderer::platform::web`), and the
// page's JS is this frame's own, which draws through it as a native host's sandbox does (`globalThis.mutations`, the
// UI library's native build). The frame's address names the page's (`#<its address>`) and the runtime version it
// needs (`runtime`), as the web client's `address` has it. A classic script, so the import map it writes comes before
// any module loads.
;(async () => {
  const started = performance.now()
  const page = new URL(location.hash.slice(1), location.href)
  /** What the page's own page is drawn as, where this can't: the DOM. */
  const unopened = (/** @type {unknown} */ why) => {
    console.warn("the renderer didn't open this page:", why)
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
    const renderer = await import("./web_renderer.js")
    await renderer.default()
    const needs = Number(new URLSearchParams(location.search).get("runtime"))
    if (!(needs <= renderer.runtime_version())) throw new Error(`it needs runtime version ${needs}`)
    // What the page's JS draws through, and asks its client through, as a native host's sandbox has them.
    Object.assign(globalThis, {
      mutations: { create: renderer.create, set: renderer.set, insert: renderer.insert, remove: renderer.remove, drop: renderer.drop, unsupported: renderer.unsupported, frame: false, video: false },
      host: { post: (/** @type {object} */ message) => renderer.post(JSON.stringify(message)) },
    })
    const canvas = /** @type {HTMLCanvasElement} */ (document.querySelector("canvas"))
    const sized = (/** @type {number} */ width, /** @type {number} */ height) => [Math.max(1, Math.round(width)), Math.max(1, Math.round(height))]
    ;[canvas.width, canvas.height] = sized(innerWidth * devicePixelRatio, innerHeight * devicePixelRatio)
    await renderer.open(canvas, devicePixelRatio, tables, page.searchParams.get("theme"), matchMedia("(pointer: coarse)").matches, {
      post: (/** @type {object} */ message) => parent.postMessage(message, "*"),
      cursor: (/** @type {string} */ cursor) => void (canvas.style.cursor = cursor),
      // Its first frame, marked on the frame's own element for whoever measures it.
      drawn: () => {
        document.documentElement.dataset.drawn = String(Math.round(performance.now()))
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
    const pointer = (/** @type {string} */ kind) => (/** @type {PointerEvent} */ event) => renderer.pointer(kind, event.offsetX, event.offsetY, event.button, event.shiftKey)
    canvas.addEventListener("pointermove", pointer("move"))
    canvas.addEventListener("pointerdown", (event) => (canvas.setPointerCapture(event.pointerId), pointer("down")(event)))
    canvas.addEventListener("pointerup", pointer("up"))
    canvas.addEventListener("pointerleave", pointer("leave"))
    // A wheel's lines and pages, in CSS pixels, as a browser scrolls them.
    const unit = [1, 16, innerHeight]
    canvas.addEventListener("wheel", (event) => (event.preventDefault(), renderer.wheel(event.offsetX, event.offsetY, event.deltaX * unit[event.deltaMode], event.deltaY * unit[event.deltaMode])), { passive: false })
    canvas.addEventListener("keydown", (event) => {
      if (event.key === "Tab" || event.key === " " || event.key.startsWith("Arrow")) event.preventDefault()
      renderer.key(event.key, event.shiftKey, event.altKey, event.ctrlKey, event.metaKey)
    })
    canvas.addEventListener("focus", () => renderer.focused(true))
    canvas.addEventListener("blur", () => renderer.focused(false))
    matchMedia("(prefers-color-scheme: dark)").addEventListener("change", ({ matches }) => renderer.appearance_changed(matches))
    canvas.focus()
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
