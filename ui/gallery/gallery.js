// What the gallery's own buttons do. Everything else on the page is the kit's elements as written in index.html.
import { confirm, toast } from "../dist/ui.js"

const $ = (id) => document.getElementById(id)

// Light, dark, or as the device has it: the theme follows `color-scheme` wherever it's set.
for (const item of $("appearance").children) item.addEventListener("click", () => (document.documentElement.style.colorScheme = item.dataset.scheme))

for (const item of $("menu-open").children) item.addEventListener("click", () => ($("menu-said").textContent = `${item.textContent} was chosen.`))

$("sheet-button").addEventListener("click", () => $("the-sheet").show())
$("sheet-form").addEventListener("submit", (event) => {
  event.preventDefault()
  $("the-sheet").close()
  const sent = [...new FormData(event.target)].map(([name, value]) => `${name}: ${value.name ?? value}`)
  toast(`Saved ${sent.join(", ")}`)
})

$("alert-button").addEventListener("click", async () => {
  const yes = await confirm({ heading: "Delete this expense?", text: "It can't be brought back.", action: "Delete", destructive: true })
  $("alert-said").textContent = yes ? "Deleted." : "Kept."
})

$("toast-button").addEventListener("click", () => toast("Sent to finance"))
$("undo-button").addEventListener("click", () => toast("Deleted Coffee with Sam", { action: { label: "Undo", run: () => toast("Put back") } }))

$("screen-back").addEventListener("back", (event) => {
  event.preventDefault()
  toast("Back was pressed")
})

// Files as a user would have chosen them: a picture drawn here, and something that isn't one.
const picture = Object.assign(document.createElement("canvas"), { width: 96, height: 96 })
const pen = picture.getContext("2d")
pen.fillStyle = "#f5f1e8"
pen.fillRect(0, 0, 96, 96)
pen.fillStyle = "#8a8578"
for (let line = 0; line < 6; line++) pen.fillRect(16, 16 + line * 12, line % 3 === 2 ? 36 : 64, 4)
picture.toBlob((photo) => {
  $("file-chosen").files = [new File([photo, new Uint8Array(248_000)], "coffee-receipt.png", { type: "image/png" }), new File([new Uint8Array(1_240_000)], "taxi-invoice.pdf", { type: "application/pdf" })]
  $("file-one").files = [new File([photo, new Uint8Array(96_000)], "hotel.png", { type: "image/png" })]
})

// A list beside what it opens: each row fills the detail and shows it.
for (const row of $("the-split").querySelectorAll("ui-row[button]")) {
  row.addEventListener("click", () => {
    $("split-detail").heading = row.heading
    $("split-amount").textContent = row.textContent
    for (const other of row.parentElement.children) other.current = other === row
    $("the-split").show()
  })
}

// Rows come and go: a row added opens up where it goes, and one taken out closes up.
let added = 0
$("row-add").addEventListener("click", () => {
  const row = Object.assign(document.createElement("ui-row"), { heading: `Lunch ${++added}`, text: "Meals · 14 Nov" })
  row.append(Object.assign(document.createElement("span"), { slot: "end", textContent: "$18.00" }))
  $("rows").children[1].before(row)
})
$("row-remove").addEventListener("click", () => $("rows").children.length > 1 && $("rows").children[1].remove())
