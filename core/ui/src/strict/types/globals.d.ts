// What a strict app's JS has beyond the language itself (ES2024): what its sandbox, QuickJS on a native host, and a
// browser both have. No `document`, `window` or other Web API: the UI library is how an app reaches its screen and
// its network.
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
