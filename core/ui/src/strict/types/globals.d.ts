// What a strict app's JS has beyond the language itself: what its sandbox, QuickJS on a native host
// (clients/app/strict/src/js.rs), and a browser both have. That's ES2024 with ECMA-402's Intl, but DisplayNames,
// DurationFormat, supportedValuesOf, formatRange and selectRange, which a native host's ICU4X doesn't make and
// TypeScript's lib still declares; and the Web APIs a Worker has, the WinterTC Minimum Common API's (web-apis.d.ts),
// but the network's: no `fetch`, `Request`, `Response`, `Headers` or `FormData`, as an app reaches the network through
// `call()`, and no WebAssembly, which QuickJS hasn't. No `document`, `window` or other DOM API: the UI library is how an
// app reaches its screen.
/// <reference path="./web-apis.d.ts" />
declare var console: {
  log(...data: unknown[]): void
  info(...data: unknown[]): void
  warn(...data: unknown[]): void
  error(...data: unknown[]): void
  debug(...data: unknown[]): void
}
declare function setTimeout(handler: () => void, timeout?: number): number
declare function clearTimeout(id: number | undefined): void
declare function setInterval(handler: () => void, timeout?: number): number
declare function clearInterval(id: number | undefined): void
declare function queueMicrotask(callback: () => void): void
declare var self: typeof globalThis
declare var navigator: { readonly userAgent: string; readonly language: string; readonly languages: readonly string[] }
interface Performance extends EventTarget {
  readonly timeOrigin: DOMHighResTimeStamp
  now(): DOMHighResTimeStamp
  toJSON(): { timeOrigin: DOMHighResTimeStamp }
}
declare var performance: Performance
/** What's told of an error nothing caught (`reportError`) and of a promise rejected with nothing to handle it. */
declare function addEventListener(type: "error", listener: (event: ErrorEvent) => void, options?: boolean | AddEventListenerOptions): void
declare function addEventListener(type: "unhandledrejection", listener: (event: PromiseRejectionEvent) => void, options?: boolean | AddEventListenerOptions): void
declare function removeEventListener(type: "error", listener: (event: ErrorEvent) => void, options?: boolean | EventListenerOptions): void
declare function removeEventListener(type: "unhandledrejection", listener: (event: PromiseRejectionEvent) => void, options?: boolean | EventListenerOptions): void
declare function dispatchEvent(event: Event): boolean
declare var onunhandledrejection: ((event: PromiseRejectionEvent) => void) | null
/** Calls `callback` before the host draws its next frame, with the time, on the clock events' `timeStamp`s are on. What
 * a page moves by hand, as a finger lets go, it moves a frame at a time with this. */
declare function requestAnimationFrame(callback: (time: number) => void): number
declare function cancelAnimationFrame(id: number): void
/** What a page keeps on this device, by its own keys, as the web's `localStorage` keeps it: it may be refused, as in a
 * private window, so each use is in a `try`. */
declare var localStorage: {
  getItem(key: string): string | null
  setItem(key: string, value: string): void
}
