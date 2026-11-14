// The tracker's app: every tracker with its total, and inside one, its entries. Plain HTML (app.html) and this
// module, on the kit's elements, calling the plugin's tools.
import { call, changed, leave } from "../ui/ui.js"

const $ = (id) => document.getElementById(id)
const number = new Intl.NumberFormat(navigator.language, { maximumFractionDigits: 2 })
const when = new Intl.DateTimeFormat(navigator.language, { day: "numeric", month: "short", hour: "numeric", minute: "2-digit" })

/// The tracker whose entries are showing, or nothing while every tracker is. It's where this page is in the
/// browser's history, so the device's own way back leads out of a tracker too.
const inside = () => history.state

async function show() {
  const tracker = inside()
  let rows
  try {
    const listed = JSON.parse(await call("tracker_list", tracker ? { tracker } : {}))
    rows = tracker ? listed.toReversed().map(entry) : listed.map(total)
  } catch (error) {
    rows = null
    $("problem").text = error.message
  }
  // A tracker opened or left while this was asking has drawn already.
  if (tracker !== inside()) return
  $("screen").heading = tracker ?? "Tracker"
  $("loading").hidden = true
  $("problem").hidden = rows !== null
  $("empty").hidden = rows === null || rows.length > 0
  $("list").replaceChildren(...(rows ?? []))
}

function total({ tracker, total, entries }) {
  const row = Object.assign(document.createElement("ui-row"), {
    heading: tracker,
    text: entries === 1 ? "1 entry" : `${number.format(entries)} entries`,
    button: true,
  })
  row.append(Object.assign(document.createElement("span"), { slot: "end", textContent: number.format(total) }))
  row.addEventListener("click", () => {
    history.pushState(tracker, "")
    show()
  })
  return row
}

function entry({ at, amount, note }) {
  const row = Object.assign(document.createElement("ui-row"), { heading: number.format(amount), text: note })
  row.append(Object.assign(document.createElement("span"), { slot: "end", textContent: when.format(new Date(at)) }))
  return row
}

addEventListener("popstate", show)
changed(show)
$("retry").addEventListener("click", show)

// Back out of a tracker is the browser's own. Back from the first screen leaves the app.
$("screen").addEventListener("back", (event) => {
  if (inside()) return
  event.preventDefault()
  leave()
})

$("add").addEventListener("click", () => {
  $("form").reset()
  $("form").elements.tracker.value = inside() ?? ""
  $("form").elements.amount.value = "1"
  $("refused").hidden = true
  $("sheet").show()
})

$("form").addEventListener("submit", async (event) => {
  event.preventDefault()
  const { tracker, amount, note } = Object.fromEntries(new FormData(event.target))
  $("save").loading = true
  try {
    await call("tracker_add", { tracker: tracker.trim(), amount: Number(amount), ...(note && { note }) })
    $("sheet").close()
    show()
  } catch (error) {
    Object.assign($("refused"), { text: error.message, hidden: false })
  }
  $("save").loading = false
})

show()
