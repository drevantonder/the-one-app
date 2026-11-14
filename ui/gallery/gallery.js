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
  toast(`Saved ${JSON.stringify(Object.fromEntries(new FormData(event.target)))}`)
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
