// The sandbox's own globals, run after LLRT's Web APIs and before the app's code. `__post`, `__now`, `__log`, `FIXED`
// (ms since 1970 for a fixed clock, or null), `LOCALE`, `TOUCH_POINTS`, `__threw` and `shim` (event-target-shim.js) are
// the host's (js.rs), and so is `mutations`, what the renderer draws with. It evaluates to what the host itself calls:
// its timers, and what it tells of promises nothing handled. What throws in a timer, a frame or a listener, or a promise
// nothing handled, is reported (`reportError`), as a browser does, and runs nothing less; it's told to the host too
// (`__threw`), where a page whose every turn throws has crashed, as one out of memory has (quickjs.rs's `crashed`).
const timers = new Map()
let nextTimer = 1
// How deep the timer running now, and what its promises run after it, is nested in timers that set each other, as
// HTML's timer nesting level: past five, a timer waits at least 4 ms, so one that sets itself again at once (a page
// waiting on its answers) doesn't spin, and the display's frames come between.
let nesting = 0
globalThis.host = { post: (message) => __post(JSON.stringify(message)) }
globalThis.setTimeout = (callback, delay = 0, ...args) => {
  const id = nextTimer++
  const level = nesting + 1
  timers.set(id, { due: __now() + Math.max(level > 5 ? 4 : 0, delay), level, run: () => callback(...args) })
  return id
}
globalThis.clearTimeout = (id) => void timers.delete(id)
globalThis.setInterval = (callback, delay = 0, ...args) => {
  const id = nextTimer++
  const every = Math.max(1, delay)
  const run = () => {
    timers.set(id, { due: __now() + every, every, run })
    callback(...args)
  }
  timers.set(id, { due: __now() + every, every, run })
  return id
}
globalThis.clearInterval = globalThis.clearTimeout
// The next frame's, run as the host's display shows one (`frame`), told the time it's shown at, as a browser's.
const frames = new Map()
globalThis.requestAnimationFrame = (callback) => {
  const id = nextTimer++
  frames.set(id, callback)
  return id
}
globalThis.cancelAnimationFrame = (id) => void frames.delete(id)
const say = (level) => (...parts) => __log(level, parts.map((part) => typeof part === "string" ? part : part instanceof Error ? `${part}\n${part.stack}` : JSON.stringify(part) ?? String(part)).join(" "))
globalThis.console = { log: say("log"), info: say("info"), debug: say("debug"), warn: say("warn"), error: say("error") }

// What's the host's in the WinterTC Minimum Common API, as Cloudflare Workers and Deno have it: `self`, events and
// aborting, a clock that's the sandbox's own (paused and stepped by the inspector), the host's name and language, and
// globalThis as an EventTarget that's told of errors nothing caught. Node's extras that LLRT adds beside the Web's go.
globalThis.self = globalThis
delete globalThis.Buffer
for (const name of ["createHash", "createHmac", "randomBytes", "randomInt", "randomFillSync", "randomFill"]) delete crypto[name]
// LLRT's File isn't drawn from its Blob, so it lacks the one method of Blob's it doesn't have itself.
Object.defineProperty(File.prototype, "stream", { configurable: true, writable: true, value: function stream() { return new Blob([this]).stream() } })
const { Event, EventTarget, defineEventAttribute, setErrorHandler } = shim
globalThis.Event = Event
globalThis.EventTarget = EventTarget
setErrorHandler((error) => uncaught(error))
globalThis.CustomEvent = class CustomEvent extends Event {
  #detail
  constructor(type, init = {}) {
    super(type, init)
    this.#detail = init.detail ?? null
  }
  get detail() { return this.#detail }
}
globalThis.ErrorEvent = class ErrorEvent extends Event {
  #init
  constructor(type, init = {}) {
    super(type, init)
    this.#init = init
  }
  get message() { return this.#init.message ?? "" }
  get filename() { return this.#init.filename ?? "" }
  get lineno() { return this.#init.lineno ?? 0 }
  get colno() { return this.#init.colno ?? 0 }
  get error() { return this.#init.error }
}
globalThis.PromiseRejectionEvent = class PromiseRejectionEvent extends Event {
  #init
  constructor(type, init) {
    super(type, init)
    this.#init = init
  }
  get promise() { return this.#init.promise }
  get reason() { return this.#init.reason }
}
// AbortController and AbortSignal, as the DOM Standard says.
const made = Symbol()
let abort
globalThis.AbortSignal = class AbortSignal extends EventTarget {
  #aborted = false
  #reason
  #dependents = new Set()
  constructor(key) {
    if (key !== made) throw new TypeError("Illegal constructor")
    super()
  }
  get aborted() { return this.#aborted }
  get reason() { return this.#reason }
  throwIfAborted() {
    if (this.#aborted) throw this.#reason
  }
  static abort(reason) {
    const signal = new AbortSignal(made)
    abort(signal, reason)
    return signal
  }
  static timeout(ms) {
    const signal = new AbortSignal(made)
    setTimeout(() => abort(signal, new DOMException("The operation timed out.", "TimeoutError")), ms)
    return signal
  }
  static any(signals) {
    const signal = new AbortSignal(made)
    const aborted = [...signals].find((source) => source.aborted)
    if (aborted) abort(signal, aborted.reason)
    else for (const source of signals) source.#dependents.add(signal)
    return signal
  }
  static {
    abort = (signal, reason = new DOMException("This operation was aborted", "AbortError")) => {
      if (signal.#aborted) return
      const dependents = [...signal.#dependents].filter((dependent) => !dependent.#aborted)
      signal.#dependents.clear()
      for (const each of [signal, ...dependents]) {
        each.#aborted = true
        each.#reason = reason
      }
      for (const each of [signal, ...dependents]) each.dispatchEvent(new Event("abort"))
    }
  }
}
defineEventAttribute(AbortSignal.prototype, "abort")
globalThis.AbortController = class AbortController {
  #signal = new AbortSignal(made)
  get signal() { return this.#signal }
  abort(reason) { abort(this.#signal, reason) }
}
const START = __now()
const ORIGIN = FIXED ?? Date.now()
globalThis.Performance = class Performance extends EventTarget {
  get timeOrigin() { return ORIGIN }
  now() { return __now() - START }
  toJSON() { return { timeOrigin: ORIGIN } }
}
globalThis.performance = new Performance()
globalThis.navigator = { userAgent: "One", language: LOCALE, languages: [LOCALE], maxTouchPoints: TOUCH_POINTS }
const events = new EventTarget()
for (const name of ["addEventListener", "removeEventListener", "dispatchEvent"]) globalThis[name] = events[name].bind(events)
// React Native's `window` is its global: Vite's preload helper in the UI library reports a failed `import()` (a component's) to it.
globalThis.window = globalThis
globalThis.onunhandledrejection = null
// Whether nothing cancelled it: a listener or the `on…` handler.
const told = (event, handler) => {
  dispatchEvent(event)
  if (typeof handler === "function") handler.call(globalThis, event)
  return !event.defaultPrevented
}
globalThis.reportError = (error) => {
  const event = new ErrorEvent("error", { cancelable: true, error, message: String(error?.message ?? error) })
  if (told(event)) console.error("Uncaught", error)
}
// What threw past the page's own code, told to the host first, so it's counted even where reporting it runs out of memory.
const uncaught = (error) => {
  __threw(error)
  reportError(error)
}

if (FIXED !== null) {
  const RealDate = Date
  globalThis.Date = class extends RealDate {
    constructor(...args) {
      if (args.length) super(...args)
      else super(FIXED + __now() - START)
    }
    static now() { return FIXED + __now() - START }
  }
  let seed = 0x9e3779b9
  Math.random = () => {
    seed = (seed + 0x6d2b79f5) | 0
    let t = Math.imul(seed ^ (seed >>> 15), 1 | seed)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
  crypto.getRandomValues = (array) => {
    const bytes = new Uint8Array(array.buffer, array.byteOffset, array.byteLength)
    for (let at = 0; at < bytes.length; at++) bytes[at] = Math.floor(Math.random() * 256)
    return array
  }
  crypto.randomUUID = () => {
    const bytes = crypto.getRandomValues(new Uint8Array(16))
    bytes[6] = (bytes[6] & 0x0f) | 0x40
    bytes[8] = (bytes[8] & 0x3f) | 0x80
    const hex = [...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("")
    return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
  }
}

;({
  due: () => Math.min(...[...timers.values()].map((timer) => timer.due)),
  timers: () => JSON.stringify([...timers.values()].map((timer) => ({ in: timer.due - __now(), every: timer.every }))),
  fire: () => {
    const now = __now()
    nesting = 0
    for (const [id, timer] of [...timers]) {
      if (timer.due > now || !timers.has(id)) continue
      timers.delete(id)
      nesting = timer.level ?? 0
      try {
        timer.run()
      } catch (error) {
        uncaught(error)
      }
    }
  },
  // Whether it asked for a frame, and the frame, at `time`: what asked for it runs, and what that asks for waits for
  // the next, as a browser's do.
  framing: () => frames.size > 0,
  frame: (time) => {
    nesting = 0
    const asked = [...frames.values()]
    frames.clear()
    for (const callback of asked) {
      try {
        callback(time)
      } catch (error) {
        uncaught(error)
      }
    }
  },
  // Promises rejected with nothing to handle them once its jobs ran, each `[promise, reason]`.
  unhandled: (rejected) => {
    for (const [promise, reason] of rejected) {
      __threw(reason)
      const event = new PromiseRejectionEvent("unhandledrejection", { cancelable: true, promise, reason })
      if (told(event, globalThis.onunhandledrejection)) console.error("Uncaught (in promise)", reason)
    }
  },
})
