// What the window shows of itself, in its middle, over the frames, from the state the window is in.
import { appStore } from "./apps.js"
import { fit } from "./bridge.js"
import { placed } from "./chrome.js"
import { baseName, notes, phone, shown, testflight } from "./client.js"
import { rail } from "./rail.js"
import { byId, report } from "./util.js"
import { machine } from "./window-machine.js"
/** @import { Context } from "./host.js" */

/// What the window says as it opens: the steps it takes to its person's home, the first of which says whether its
/// network answers. With a launcher kept from before, it's starting that alone.
/** @param {Context} context @returns {{ steps: string[], at: number, text?: string }} */
function steps({ connection, starting }) {
  if (starting && connection !== "online") return { steps: [`Opening ${baseName()}`], at: 0 }
  const reached = connection === "offline" || connection === "reconnecting" ? "Can't reach your network" : "Connecting to your network"
  const at = starting || connection === "online" ? 1 : 0
  return { steps: [reached, `Opening ${baseName()}`], at, text: at === 0 && reached !== "Connecting to your network" ? "Check this device is online." : undefined }
}

/// What each problem says: its heading, its one sentence, what's behind it for Show Details, and the button that
/// fixes it, if there is one.
/** @param {import("xstate").SnapshotFrom<typeof machine>} now @returns {[string, string, string | undefined, string | undefined] | undefined} */
function problemOf(now) {
  // One that crashed once it had opened says so, with Reload, as a browser's crashed tab does (src/unopened.js).
  if (now.matches("launcher_failed") && now.context.crashed) return [`${baseName()} stopped working`, "Reload it to start it again.", now.context.why, "Reload"]
  if (now.matches("launcher_failed")) return [`${baseName()} didn't open.`, "Try again in a moment.", now.context.why, "Try Again"]
  if (now.matches("native_failed")) return [`${baseName()} can't open its windows on this device.`, "Its graphics didn't start.", now.context.why, undefined]
  if (now.matches("update_failed")) return ["Couldn't update", now.context.why ?? "", undefined, "Try Again"]
}
/// What a computer's launcher says while its servers say it's behind them: Update Required, with Update.
/** @type {[string, string, undefined, string]} */
const UPDATE_REQUIRED = ["Update Required", "This version of the One App no longer works with your network.", undefined, "Update"]
/// Draws what the window shows of itself, in its middle, over the frames: its mark and steps while it opens, a
/// spinner and a line while it updates, why there's no Home Screen, or a problem; and over the launcher, a banner
/// while its network doesn't answer. The launcher is the frames alone.
/** @type {string | undefined} */
let lastProblem
export function draw() {
  const now = shown.getSnapshot()
  const { context } = now
  byId("state").hidden = now.matches("launcher") || now.matches("app_store") || now.matches("sign_in")
  document.body.ariaBusy = now.matches("opening") || now.matches("updating") ? "true" : null
  const opening = now.matches("opening")
  byId("mark").hidden = byId("steps").hidden = !opening
  const opened = steps(context)
  byId("reconnect").hidden = !opening || opened.text === undefined
  if (opening) {
    byId("steps").replaceChildren(
      ...opened.steps.map((step, i) => {
        const item = Object.assign(document.createElement("li"), { className: i < opened.at ? "done" : i === opened.at ? "now" : "" })
        const mark = i < opened.at ? icon(CHECK) : i === opened.at ? spinner() : Object.assign(document.createElement("span"), { className: "next" })
        const words = Object.assign(document.createElement("span"), { textContent: step })
        if (i === opened.at && opened.text) words.append(Object.assign(document.createElement("small"), { textContent: opened.text }))
        item.append(mark, words)
        return item
      }),
    )
  }
  // Behind, it's asked again under the alert; updated, under the spinner, with what the update said.
  const behind = now.matches("needs_updating") || (now.matches("checking") && context.behind === true)
  const updating = now.matches("updating") || (now.matches("checking") && !behind)
  byId("status").hidden = !updating
  byId("status-text").textContent = context.message ?? ""
  byId("status-text").hidden = !(updating && context.message)
  byId("empty").hidden = !now.matches("no_launcher")
  byId("empty-text").textContent = context.only ?? (appStore ? "" : "Ask your AI to install a Home Screen")
  byId("empty-text").hidden = !byId("empty-text").textContent
  byId("open-app-store").hidden = Boolean(context.only) || !appStore
  // Behind, a computer's launcher says so where it shows everything else, as one problem box with Update, as its app
  // updates itself (DESIGN.md). A phone's is an alert, as iOS's Update Required, which always has a way on: its app
  // is updated by its network's owner, so there it has Try Again, and Update too where it came from TestFlight, which
  // opens TestFlight.
  const required = /** @type {HTMLDialogElement} */ (byId("update-required"))
  byId("update-button").hidden = phone && !testflight
  byId("update-owner").hidden = !phone || testflight
  byId("try-again-button").hidden = !phone
  byId("try-again-button").toggleAttribute("disabled", now.matches("checking"))
  byId("try-again-button").ariaBusy = now.matches("checking") ? "true" : null
  if (behind && phone && !required.open) required.showModal()
  if (!(behind && phone) && required.open) required.close()
  const problem = behind && !phone ? UPDATE_REQUIRED : problemOf(now)
  byId("problem-button").dataset.variant = behind ? "default" : "outline"
  byId("problem-button").toggleAttribute("disabled", now.matches("checking"))
  byId("problem-button").ariaBusy = now.matches("checking") ? "true" : null
  byId("problem").hidden = !problem
  if (problem) {
    const [heading, text, details, action] = problem
    byId("problem-heading").textContent = heading
    byId("problem-text").textContent = text
    byId("problem-details").hidden = !details
    byId("problem-details-text").textContent = details ?? ""
    byId("problem-action").hidden = !action
    byId("problem-button").textContent = action ?? ""
  }
  // What's shown is reported once, as it's shown, with what Show Details says.
  const said = behind ? "Update Required: This version of the One App no longer works with your network." : problem && [problem[0], problem[1], problem[2]].filter(Boolean).join(": ")
  if (said && said !== lastProblem) report("one.client.problem", said)
  lastProblem = said
  fit()
  notes()
  rail()
  // Over the launcher: its network not answering, and trying again once it has lost it. Answering, nothing.
  const banner = now.matches("launcher") && (context.connection === "offline" || context.connection === "reconnecting")
  if (byId("banner").hidden === banner) {
    byId("banner").hidden = !banner
    placed()
  }
  byId("banner-spinner").hidden = context.connection !== "reconnecting"
  byId("banner-text").textContent = context.connection === "reconnecting" ? "Reconnecting…" : "Can't reach your network"
  byId("banner-retry").hidden = context.connection !== "offline"
}

/// The UI library's check icon and spinner, which the window draws its steps with, as the UI library does.
const CHECK = "M229.66,77.66l-128,128a8,8,0,0,1-11.32,0l-56-56a8,8,0,0,1,11.32-11.32L96,188.69,218.34,66.34a8,8,0,0,1,11.32,11.32Z"
/** @param {string} path */
function icon(path) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg")
  svg.setAttribute("class", "ui-icon")
  svg.setAttribute("viewBox", "0 0 256 256")
  svg.setAttribute("aria-hidden", "true")
  const drawn = document.createElementNS("http://www.w3.org/2000/svg", "path")
  drawn.setAttribute("d", path)
  svg.append(drawn)
  return svg
}
function spinner() {
  const shown = /** @type {HTMLElement} */ (byId("banner-spinner").cloneNode(true))
  shown.removeAttribute("id")
  shown.hidden = false
  return shown
}
