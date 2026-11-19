// The sandbox's own globals, run after LLRT's Web APIs (TextEncoder, TextDecoder, crypto) and before the interpreter
// (core/codemode/run.js): what OpenCode's interpreter and Effect use of a browser's. `__now` (ms since the run started)
// and `__log` are the host's (lib.rs). It evaluates to what the host calls: `fire`, which runs the timers that are due
// and says how long until the next, or -1 for none.
const timers = new Map()
let nextTimer = 1
globalThis.setTimeout = (callback, delay = 0, ...args) => {
  const id = nextTimer++
  timers.set(id, { due: __now() + Math.max(0, delay), run: () => callback(...args) })
  return id
}
globalThis.clearTimeout = (id) => void timers.delete(id)
globalThis.setImmediate = (callback, ...args) => setTimeout(callback, 0, ...args)
globalThis.clearImmediate = globalThis.clearTimeout
const ORIGIN = Date.now()
globalThis.performance = { timeOrigin: ORIGIN, now: () => __now() }
const say = (...parts) => __log(parts.map((part) => (typeof part === "string" ? part : part instanceof Error ? `${part}\n${part.stack}` : JSON.stringify(part) ?? String(part))).join(" "))
globalThis.console = { log: say, info: say, debug: say, warn: say, error: say }
// QuickJS has no Intl. The interpreter lists in English the names a script could have meant (`ListFormat`).
globalThis.Intl = {
  ListFormat: class ListFormat {
    #word
    constructor(_locale, { type = "conjunction" } = {}) {
      this.#word = type === "disjunction" ? "or" : "and"
    }
    format(list) {
      const all = [...list]
      return all.length < 3 ? all.join(` ${this.#word} `) : `${all.slice(0, -1).join(", ")}, ${this.#word} ${all.at(-1)}`
    }
  },
}
;({
  fire() {
    const now = __now()
    // Those due now, and not those they set, which wait for the jobs these leave to run first.
    for (const [id, timer] of [...timers].filter(([, timer]) => timer.due <= now)) {
      if (!timers.delete(id)) continue
      try {
        timer.run()
      } catch (error) {
        say(error)
      }
    }
    let next = -1
    for (const timer of timers.values()) next = next < 0 ? timer.due - now : Math.min(next, timer.due - now)
    return Math.max(next, next < 0 ? -1 : 0)
  },
})
