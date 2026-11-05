// Appearance: Light, Dark or Automatic for this device, as small pictures of a screen, the chosen one marked.
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/appearance.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Icon, cn } from "@one/ui"

const CHOICES = [
  { value: "light", title: "Light" },
  { value: "dark", title: "Dark" },
  { value: "automatic", title: "Automatic" },
]

// Half a screen as it looks in one theme: a title, and four rows.
function Half({ dark }) {
  const fg = dark ? "bg-white" : "bg-black"
  return (
    <div className={cn("flex flex-1 flex-col gap-1.5 overflow-hidden pt-2 pl-2", dark ? "bg-[#141414]" : "bg-white")}>
      <b className={cn("block h-[5px] w-[30px] rounded-full opacity-70", fg)} />
      {[0, 1, 2, 3].map((n) => <i key={n} className={cn("block min-h-3.5 w-[50px] rounded-[3px] opacity-[0.12]", fg)} />)}
    </div>
  )
}

// The picture of a choice. Automatic is a screen of both, light on the left, dark on the right, each cut off at the middle.
function Shot({ value }) {
  return (
    <div className="flex h-[120px] w-[66px] overflow-hidden rounded-[10px]">
      {value === "automatic" ? (
        <>
          <Half />
          <div className="flex flex-1 overflow-hidden"><div className="-ml-[25px] flex flex-1"><Half dark /></div></div>
        </>
      ) : <Half dark={value === "dark"} />}
    </div>
  )
}

function Appearance() {
  // Not known until the device says.
  const [chosen, setChosen] = useState(null)

  useEffect(() => {
    one.largeTitle("Appearance")
    one.appearance().then(setChosen)
  }, [])

  async function choose(value) {
    one.tap()
    // Checked at once; then as the device says it is.
    setChosen(value)
    setChosen(await one.appearance(value))
  }

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <h1 className="large-title text-3xl font-bold tracking-tight">Appearance</h1>
      <div role="radiogroup" aria-label="Appearance" className="flex flex-wrap gap-6">
        {CHOICES.map(({ value, title }) => {
          const on = value === chosen
          return (
            <button
              key={value}
              type="button"
              role="radio"
              aria-checked={on}
              onClick={() => choose(value)}
              className="group flex cursor-pointer flex-col items-center gap-2.5 rounded-xl p-1 text-sm font-medium outline-none"
            >
              <span className={cn("rounded-xl p-1 ring-1 ring-border transition-shadow group-focus-visible:ring-[3px] group-focus-visible:ring-ring/50", on && "ring-2 ring-primary")}>
                <Shot value={value} />
              </span>
              <span className="flex items-center gap-1.5">
                <Icon name={on ? "check-circle-fill" : "circle"} className={cn("size-4", !on && "text-muted-foreground")} />
                {title}
              </span>
            </button>
          )
        })}
      </div>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Appearance />)
