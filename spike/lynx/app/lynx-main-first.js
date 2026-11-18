// A variant for timing only: the app's first render runs on Lynx's main thread itself, as ReactLynx's first screen
// does, so no background load or thread hop comes before the first frame. (Events would need the background to take
// over afterwards, which this variant doesn't do.)
import "./lynx-globals.js"
import "./lynx-main.js"
import { start } from "./screen.js"

const render = globalThis.renderPage
globalThis.renderPage = function (data) {
  render(data)
  start((update) => globalThis.applyOps(update))
}
