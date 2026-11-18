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
