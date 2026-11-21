// What the other modules of the page share and nothing of it depends on: reporting to Telemetry, and finding the page's own elements.
import { invoke } from "./host.js"

/// Reports what went wrong here to Telemetry, through the client, which keeps it until a server can be told: what
/// this page showed its person, an error of its own or of an app's page, an app that never drew. Names, files and
/// what was shown only, never what anyone typed.
/** @param {string} name @param {string} error @param {Record<string, unknown>} [attributes] */
export const report = (name, error, attributes = {}) => invoke("report", { record: { name, error, attributes } }).catch(() => {})
/// An error as OpenTelemetry's `exception` says one: its message, and the file and line it was thrown at, by its
/// path alone, which leaves out anything a query had in it, and its stack, which a browser's client puts in the
/// source it was built from (`clients/web/host.js`).
/** @param {unknown} message @param {unknown} [file] @param {unknown} [line] @param {string} [app] @param {unknown} [stack] */
export function exception(message, file, line, app, stack) {
  const path = typeof file === "string" && URL.canParse(file) ? new URL(file).pathname : undefined
  report("exception", String(message).slice(0, 500), { "code.file.path": path, "code.line.number": typeof line === "number" ? line : undefined, "exception.stacktrace": typeof stack === "string" && stack ? stack : undefined, "one.app": app })
}
addEventListener("error", ({ message, filename, lineno, error }) => exception(message, filename, lineno, undefined, error?.stack))
addEventListener("unhandledrejection", ({ reason }) => exception(reason?.message ?? reason, undefined, undefined, undefined, reason?.stack))
/** An element of the page's own, which is there. @param {string} id */
export const byId = (id) => /** @type {HTMLElement} */ (document.getElementById(id))
