// What the window's page asks of where it runs, which in the app is its Rust half (`src/lib.rs`), through Tauri's
// commands and events. The web client gives the same page its own (`clients/web/host.js`).
/** @type {typeof import("./host.js").invoke} */
export const invoke = window.__TAURI__.core.invoke
/** @type {typeof import("./host.js").listen} */
export const listen = window.__TAURI__.event.listen

/// Where an app's page is: under its plugin's name, then who it's for if that isn't its user, last part first.
/// Android's and Windows' web views load only http, so there the app's own scheme is the start of a host's name.
/** @type {typeof import("./host.js").address} */
export const address = (view) => `${location.protocol === "tauri:" ? "oneapp://" : "http://oneapp."}${view.host}.localhost/${view.app}`

/// What an app's frame may do. Its address is its own, so it reaches nothing else there.
export const sandbox = "allow-scripts allow-same-origin allow-forms"

/// A web address an app opens: the window is sent there, and the app hands it to the device's own browser.
/** @type {typeof import("./host.js").openLink} */
export const openLink = (url) => location.assign(url)
