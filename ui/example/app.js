// A small tracker, written as an agent writes an app: plain HTML (index.html) and this module, on the kit's elements.
import { toast } from "../dist/ui.js"

/// Calls one of this app's tools. A stand-in until an app can reach its server: the expenses live in this page's
/// memory. Replace what's inside with the real call, and nothing else here changes.
const kept = []
async function call(tool, input) {
  const at = kept.findIndex((expense) => expense.id === input?.id)
  if (tool === "list") return kept.toSorted((a, b) => b.date.localeCompare(a.date))
  if (tool === "save") return void (at < 0 ? kept.push(input) : (kept[at] = input))
  if (tool === "remove") return void kept.splice(at, 1)
  throw new Error(`No tool named ${tool}`)
}

const $ = (id) => document.getElementById(id)
const money = new Intl.NumberFormat(navigator.language, { style: "currency", currency: "AUD" })
const day = new Intl.DateTimeFormat(navigator.language, { day: "numeric", month: "short", timeZone: "UTC" })
const categories = Object.fromEntries([...document.querySelectorAll("option")].map((option) => [option.value, option.label]))

/// The expense the sheet is open on, or nothing while it adds one.
let open = null

async function show() {
  let expenses
  try {
    expenses = await call("list")
  } catch {
    expenses = null
  }
  $("loading").hidden = true
  $("problem").hidden = expenses !== null
  $("empty").hidden = expenses === null || expenses.length > 0
  $("list").replaceChildren(...(expenses ?? []).map(row))
}

function row(expense) {
  const row = Object.assign(document.createElement("ui-row"), {
    heading: expense.what,
    text: `${categories[expense.category]} · ${day.format(new Date(expense.date))}`,
    button: true,
  })
  row.append(Object.assign(document.createElement("span"), { slot: "end", textContent: money.format(expense.amount) }))
  row.addEventListener("click", () => edit(expense))
  return row
}

function edit(expense) {
  open = expense
  const filled = expense ?? { what: "", amount: "", category: "meals", date: new Date().toLocaleDateString("en-CA") }
  $("form").reset()
  for (const field of $("form").elements) if (field.name) field.value = String(filled[field.name])
  $("sheet").heading = expense ? "Expense" : "Add expense"
  $("remove").hidden = !expense
  $("sheet").show()
}

$("add").addEventListener("click", () => edit(null))
$("retry").addEventListener("click", show)

$("form").addEventListener("submit", async (event) => {
  event.preventDefault()
  const typed = Object.fromEntries(new FormData(event.target))
  $("save").loading = true
  await call("save", { ...typed, amount: Number(typed.amount), id: open?.id ?? crypto.randomUUID() })
  $("save").loading = false
  $("sheet").close()
  show()
})

$("remove").addEventListener("click", async () => {
  const removed = open
  await call("remove", removed)
  $("sheet").close()
  show()
  toast(`Deleted ${removed.what}`, { action: { label: "Undo", run: () => call("save", removed).then(show) } })
})

show()
