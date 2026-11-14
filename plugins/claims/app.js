// Claims' app: your expenses and your claims, and the claims that wait on an approver. Plain HTML (app.html) and
// this module, on the kit's elements, calling the plugin's tools.
import { call, changed, confirm, leave, upload } from "../ui/ui.js"

const $ = (id) => document.getElementById(id)
const make = (tag, properties, ...inside) => {
  const made = Object.assign(document.createElement(tag), properties)
  made.append(...inside)
  return made
}
const ask = async (tool, inputs) => JSON.parse(await call(`claims_${tool}`, inputs))

const money = new Intl.NumberFormat(navigator.language, { style: "currency", currency: "AUD" })
/// The day something was spent, which is a date with no time, and the day something happened, which is a moment.
const spent = new Intl.DateTimeFormat(navigator.language, { day: "numeric", month: "short", timeZone: "UTC" })
const on = new Intl.DateTimeFormat(navigator.language, { day: "numeric", month: "short" })
const categories = Object.fromEntries([...document.querySelectorAll("option")].map((option) => [option.value, option.label]))
const what = (expense) => expense.note || categories[expense.category]

/// The claim that's open beside the lists: its `id`, whether it's open as its approver sees it (`approving`), and
/// the claim itself once it's been answered.
let open = null
/// The expenses not in a claim yet, as last listed.
let unclaimed = []
/// The expense the sheet is open on, or nothing while it adds one.
let editing = null

const changes = (claim) => !open.approving && ["draft", "sent_back"].includes(claim.status)

/// Where a claim is up to, in a few words.
const where = (claim) => ({
  draft: "Not sent yet",
  sent: `Sent ${on.format(new Date(claim.sent_at))}`,
  sent_back: `Sent back: ${claim.reason}`,
  approved: `Approved ${on.format(new Date(claim.decided_at))}`,
  paid: `Paid ${on.format(new Date(claim.paid_at))}`,
})[claim.status]

// Receipts. A receipt's file never changes, so it's asked for once, and its small picture is drawn once. The
// client keeps the file, and its address here is where the client keeps it.

const files = new Map()
const smalls = new Map()

/// A receipt's file, as the address of its picture.
function file({ id }) {
  if (!files.has(id)) files.set(id, call("claims_receipt", { receipt: id }).catch((error) => (files.delete(id), Promise.reject(error))))
  return files.get(id)
}

/// A receipt's small picture, for a row.
async function small(receipt) {
  if (smalls.has(receipt.id)) return smalls.get(receipt.id)
  const image = Object.assign(new Image(), { src: await file(receipt) })
  await image.decode()
  const scale = 96 / Math.min(image.naturalWidth, image.naturalHeight)
  const canvas = make("canvas", { width: Math.round(image.naturalWidth * scale), height: Math.round(image.naturalHeight * scale) })
  canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height)
  smalls.set(receipt.id, canvas.toDataURL("image/jpeg", 0.8))
  return smalls.get(receipt.id)
}

/// What a row starts with: its expense's first receipt, small.
function thumb(expense) {
  const receipt = expense.receipts.find(({ type }) => type.startsWith("image/"))
  if (!receipt) return make("span", { slot: "start", className: "thumb" }, ...(expense.receipts.length ? [make("ui-icon", { name: "file" })] : []))
  const image = make("img", { slot: "start", className: "thumb", alt: "" })
  if (smalls.has(receipt.id)) image.src = smalls.get(receipt.id)
  else small(receipt).then((address) => (image.src = address), () => {})
  return image
}

// The lists

function expenseRow(expense, pressed) {
  const row = make("ui-row", {
    heading: what(expense),
    text: [expense.note && categories[expense.category], spent.format(new Date(expense.date)), !expense.receipts.length && "No receipt"].filter(Boolean).join(" · "),
    button: true,
  }, thumb(expense), make("span", { slot: "end", textContent: money.format(expense.amount) }))
  row.addEventListener("click", () => pressed(expense))
  return row
}

function claimRow(claim, approving) {
  const owed = make("span", { slot: "end", className: "owed" }, money.format(claim.total))
  // Sent back to you is the one thing here that needs you.
  if (claim.status === "sent_back" && !approving) owed.prepend(make("ui-icon", { name: "warning-circle" }))
  const row = make("ui-row", { heading: claim.title, text: where(claim), button: true, current: open?.id === claim.id && open.approving === approving }, owed)
  row.addEventListener("click", () => {
    open = { id: claim.id, approving }
    drawClaim({ ...claim, expenses: [] })
    $("split").show()
    show()
  })
  return row
}

/// How many times the lists have been asked for: an answer to an earlier ask than the last isn't drawn.
let asked = 0

/// Asks for everything that's showing, and draws it.
async function show() {
  const mine = ++asked
  let expenses, claims, waiting, claim
  try {
    ;[expenses, claims, waiting] = await Promise.all([ask("expenses"), ask("list"), ask("waiting")])
    // A claim that's gone from the list it was opened from, as one sent back is from its approver's, is closed.
    claim = open && (open.approving ? waiting : claims).some(({ id }) => id === open.id) ? await ask("show", { claim: open.id }) : null
  } catch (error) {
    if (mine !== asked) return
    $("loading").hidden = true
    Object.assign($("problem"), { text: error.message, hidden: !$("tabs").hidden })
    return
  }
  if (mine !== asked) return
  unclaimed = expenses
  $("loading").hidden = $("problem").hidden = true
  $("tabs").hidden = false
  if (open && !claim) $("split").close()
  else if (claim) drawClaim(claim)

  $("none").hidden = expenses.length > 0 || claims.length > 0
  $("unclaimed-part").hidden = expenses.length === 0
  $("claims-part").hidden = claims.length === 0
  $("unclaimed").replaceChildren(...expenses.map((expense) => expenseRow(expense, edit)))
  $("claims").replaceChildren(...claims.map((claim) => claimRow(claim, false)))

  const sent = waiting.filter(({ status }) => status === "sent")
  const approved = waiting.filter(({ status }) => status === "approved")
  $("none-waiting").hidden = waiting.length > 0
  $("sent-part").hidden = sent.length === 0
  $("approved-part").hidden = approved.length === 0
  $("sent").replaceChildren(...sent.map((claim) => claimRow(claim, true)))
  $("approved").replaceChildren(...approved.map((claim) => claimRow(claim, true)))
}

// A claim

function drawClaim(claim) {
  open.claim = claim
  const mine = changes(claim)
  const deciding = open.approving && claim.status === "sent"
  $("claim").heading = claim.title
  $("total").textContent = money.format(claim.total)
  // Sent back, a claim says so, and why, as the problem it is.
  $("where").textContent = claim.status === "sent_back" ? "" : where(claim)
  Object.assign($("sent-back"), { text: claim.reason ?? "", hidden: claim.status !== "sent_back" })
  $("claim-expenses").replaceChildren(...claim.expenses.map((expense) => expenseRow(expense, mine ? edit : look)))
  $("refused").hidden = true
  $("claim-menu").hidden = !mine
  $("send").hidden = !mine
  $("send").textContent = claim.status === "sent_back" ? "Send again" : "Send for approval"
  $("add-to").hidden = !mine || unclaimed.length === 0
  $("approve").hidden = $("send-back").hidden = !deciding
  $("pay").hidden = !(open.approving && claim.status === "approved")
}

/// Does one thing to the open claim, from its button, and shows what the server refused on the claim itself.
async function act(button, tool, inputs = {}) {
  button.loading = true
  try {
    await ask(tool, { claim: open.id, ...inputs })
    await show()
  } catch (error) {
    Object.assign($("refused"), { text: error.message, hidden: false })
  }
  button.loading = false
}

$("send").addEventListener("click", () => act($("send"), "send"))
$("approve").addEventListener("click", () => act($("approve"), "approve"))
$("pay").addEventListener("click", () => act($("pay"), "pay"))

$("discard").addEventListener("click", async () => {
  const { claim } = open
  if (!await confirm({ heading: `Delete ${claim.title}?`, text: "Its expenses stay, not claimed yet.", action: "Delete", destructive: true })) return
  try {
    await call("claims_discard", { claim: claim.id })
    show()
  } catch (error) {
    Object.assign($("refused"), { text: error.message, hidden: false })
  }
})

$("send-back").addEventListener("click", () => {
  $("back-form").reset()
  $("back-refused").hidden = true
  $("back-sheet").show()
})

$("back-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  $("back-save").loading = true
  try {
    await ask("send_back", { claim: open.id, reason: new FormData(event.target).get("reason") })
    $("back-sheet").close()
    show()
  } catch (error) {
    Object.assign($("back-refused"), { text: error.message, hidden: false })
  }
  $("back-save").loading = false
})

$("split").addEventListener("close", () => {
  open = null
  for (const row of document.querySelectorAll("ui-row[current]")) row.current = false
  // What was open over a claim goes with it: the claim may have gone while its approver was writing why.
  for (const sheet of document.querySelectorAll("ui-sheet[open]")) sheet.close()
})

// An expense

/// Opens the sheet on an expense to change it, or on nothing to add one.
async function edit(expense) {
  editing = expense
  const form = $("expense-form")
  form.reset()
  for (const [name, value] of Object.entries({ amount: expense?.amount ?? "", note: expense?.note ?? "", category: expense?.category ?? "travel", date: expense?.date ?? new Date().toLocaleDateString("en-CA") })) form.elements[name].value = String(value)
  // A phone's keyboard comes up for an amount that's about to be typed, and stays down over one that's there.
  form.elements.amount.toggleAttribute("autofocus", !expense)
  $("expense-sheet").heading = expense ? what(expense) : "New expense"
  $("expense-refused").hidden = true
  $("remove").hidden = !expense
  $("take-out").hidden = !expense?.claim
  $("expense-sheet").show()
  // Its receipts are in the field as what's kept of them, each knowing which receipt it is.
  const kept = await Promise.all((expense?.receipts ?? []).map(async ({ id, name, type, size }) => (
    { receipt: id, name, type, size, src: type.startsWith("image/") ? await small({ id }).catch(() => undefined) : undefined }
  )))
  if (editing === expense) form.elements.receipts.files = [...kept, ...form.elements.receipts.files]
}

$("add").addEventListener("click", () => edit(null))

$("expense-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  const form = event.target
  const { amount, note, category, date } = Object.fromEntries(new FormData(form))
  const chosen = form.elements.receipts.files
  $("save").loading = true
  try {
    const said = { amount: Number(amount), note: note.trim(), category, date }
    // Saved again after part of it was refused, it carries on from what was saved.
    const was = editing?.receipts ?? []
    editing = editing ? await ask("change", { expense: editing.id, ...said }) : await ask("record", said)
    for (const receipt of was) if (!chosen.some((file) => file.receipt === receipt.id)) await call("claims_detach", { receipt: receipt.id })
    // A receipt's file goes straight to where the network keeps it, and the expense is told what was sent.
    for (const file of chosen) file.receipt ??= (await ask("attach", { expense: editing.id, name: file.name, type: file.type, upload: await upload(file) })).id
    $("expense-sheet").close()
  } catch (error) {
    Object.assign($("expense-refused"), { text: error.message, hidden: false })
  }
  $("save").loading = false
  show()
})

$("remove").addEventListener("click", async () => {
  if (!await confirm({ heading: `Delete ${what(editing)}?`, text: "It can't be brought back.", action: "Delete", destructive: true })) return
  put("remove")
})
$("take-out").addEventListener("click", () => put("take_out"))

/// Does one thing to the expense the sheet is open on, and closes the sheet.
async function put(tool) {
  try {
    await call(`claims_${tool}`, { expense: editing.id })
    $("expense-sheet").close()
    show()
  } catch (error) {
    Object.assign($("expense-refused"), { text: error.message, hidden: false })
  }
}

/// Opens an expense that can't change, to look at its receipts.
async function look(expense) {
  $("looking-sheet").heading = what(expense)
  $("looking-at").textContent = [money.format(expense.amount), expense.note && categories[expense.category], spent.format(new Date(expense.date)), !expense.receipts.length && "No receipt"].filter(Boolean).join(" · ")
  $("receipts").replaceChildren()
  $("looking-sheet").show()
  for (const receipt of expense.receipts) {
    if (!receipt.type.startsWith("image/")) {
      $("receipts").append(make("ui-row", { heading: receipt.name }, make("ui-icon", { slot: "start", name: "file" })))
      continue
    }
    const picture = make("div", { className: "receipt" }, make("img", { alt: receipt.name, src: await file(receipt).catch(() => "") }))
    picture.addEventListener("click", () => picture.toggleAttribute("data-close"))
    $("receipts").append(picture)
  }
}

// Gathering expenses into a claim: a new one, or the one that's open.

function gather(into) {
  $("gather-form").reset()
  $("gather-sheet").heading = into ? "Add expenses" : "New claim"
  $("gather-save").textContent = into ? "Add" : "Make claim"
  Object.assign($("gather-title"), { hidden: !!into, disabled: !!into, autofocus: !into })
  $("gather-refused").hidden = true
  $("gather-expenses").replaceChildren(...unclaimed.map((expense) => make("ui-checkbox", {
    name: "expenses",
    value: String(expense.id),
    // Everything not claimed yet goes in a new claim unless it's unticked, and into an open one only what's ticked.
    checked: !into,
    label: what(expense),
    hint: `${money.format(expense.amount)} · ${spent.format(new Date(expense.date))}`,
  })))
  $("gather-sheet").show()
}

$("gather").addEventListener("click", () => gather(null))
$("add-to").addEventListener("click", () => gather(open.claim))

$("gather-form").addEventListener("submit", async (event) => {
  event.preventDefault()
  const sent = new FormData(event.target)
  const expenses = sent.getAll("expenses").map(Number)
  $("gather-save").loading = true
  try {
    const claim = await ask("gather", sent.has("title") ? { title: sent.get("title"), expenses } : { claim: open.id, expenses })
    $("gather-sheet").close()
    open = { id: claim.id, approving: false }
    drawClaim(claim)
    $("split").show()
    show()
  } catch (error) {
    Object.assign($("gather-refused"), { text: error.message, hidden: false })
  }
  $("gather-save").loading = false
})

changed(show)
$("retry").addEventListener("click", show)
$("home").addEventListener("back", (event) => {
  event.preventDefault()
  leave()
})

show()
