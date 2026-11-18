// What the window's page reaches of Tauri in an app's own window (`host.rs`), where there's no Tauri: `invoke` and
// `listen`, which go to this window's process and on to the app's window (`hosts.rs`), and come back by
// `__ONE_HOST__`. Only the window's own page runs this, and each message carries a key no app's frame knows.
;(() => {
  const key = "__KEY__"
  /** @type {Map<number, [(value: unknown) => void, (error: unknown) => void]>} */
  const asked = new Map()
  /** @type {Map<string, Set<(event: { event: string, payload: unknown }) => void>>} */
  const hearing = new Map()
  let next = 0
  /** @param {ArrayBuffer} buffer */
  const base64 = (buffer) => {
    const bytes = new Uint8Array(buffer)
    let text = ""
    for (let at = 0; at < bytes.length; at += 0x8000) text += String.fromCharCode(...bytes.subarray(at, at + 0x8000))
    return btoa(text)
  }
  /** @param {string} command @param {any} [args] @param {{ headers?: Record<string, string> }} [options] */
  const invoke = (command, args = {}, options = {}) =>
    new Promise((resolve, reject) => {
      const id = ++next
      asked.set(id, [resolve, reject])
      // A file sent as a request's body (`upload`) goes as base64, with the app it's for.
      if (args instanceof ArrayBuffer) args = { body: base64(args), app: options.headers?.app }
      // @ts-ignore: wry's
      window.ipc.postMessage(JSON.stringify({ key, id, command, args }))
    })
  /** @param {string} event @param {(event: { event: string, payload: unknown }) => void} handler */
  const listen = async (event, handler) => {
    if (!hearing.has(event)) hearing.set(event, new Set())
    hearing.get(event)?.add(handler)
    return () => hearing.get(event)?.delete(handler)
  }
  Object.defineProperty(window, "__TAURI__", { value: Object.freeze({ core: Object.freeze({ invoke, addPluginListener: async () => {} }), event: Object.freeze({ listen }) }) })
  Object.defineProperty(window, "__ONE_HOST__", {
    value: Object.freeze({
      /** @param {number} id @param {boolean} ok @param {unknown} value */
      answer: (id, ok, value) => {
        const [resolve, reject] = asked.get(id) ?? []
        asked.delete(id)
        ;(ok ? resolve : reject)?.(value)
      },
      /** @param {string} event @param {unknown} payload */
      event: (event, payload) => hearing.get(event)?.forEach((handler) => handler({ event, payload })),
    }),
  })
})()
