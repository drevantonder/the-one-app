// What the window's page asks of where it runs, which in the app is its Rust half (`src/lib.rs`), through Tauri's
// commands and events. The web client gives the same page its own (`clients/web/host.js`).
/** @type {typeof import("./host.js").invoke} */
export const invoke = window.__TAURI__.core.invoke
/** @type {typeof import("./host.js").listen} */
export const listen = window.__TAURI__.event.listen

/// Where an app's page is: under its app's name, then who it's for if that isn't its user, last part first.
/// Android's and Windows' web views load only http, so there the app's own scheme is the start of a host's name.
/** @type {typeof import("./host.js").address} */
export const address = (view) => `${location.protocol === "tauri:" ? "oneapp://" : "http://oneapp."}${view.host}.localhost/${view.entrypoint}`

/// What an app's frame may do. Its address is its own, so it reaches nothing else there.
export const sandbox = "allow-scripts allow-same-origin allow-forms"

/// A web address an app opens: the window is sent there, and the app hands it to the device's own browser.
/** @type {typeof import("./host.js").openLink} */
export const openLink = (url) => location.assign(url)

/// Light or dark for what's the window's own, not its page: a computer's title bar, an iPhone's status bar (`theme`
/// in lib.rs), and Android's system bars, which are `MainActivity`'s.
/** @type {typeof import("./host.js").setTheme} */
export const setTheme = (theme) => {
  if (globalThis.androidTheme) return globalThis.androidTheme.postMessage(theme ?? "")
  return invoke("theme", { theme })
}

/// A haptic: Android's app plays it as Android's own (`MainActivity`), and elsewhere Tauri's haptics plugin, as
/// UIKit's feedback on an iPhone, and as nothing on a computer. Each follows the system's own setting.
/** @type {typeof import("./host.js").haptic} */
export const haptic = (kind) => {
  if (globalThis.androidHaptic) return globalThis.androidHaptic.postMessage(kind)
  if (kind === "selection") return invoke("plugin:haptics|selection_feedback")
  if (kind === "success" || kind === "warning" || kind === "error") return invoke("plugin:haptics|notification_feedback", { type: kind })
  return invoke("plugin:haptics|impact_feedback", { style: kind })
}
