// What the native app's menu bar item (its tray item, on other computers) opens, as the Mac app's did: your devices,
// which are on and which this one is, then the widgets on your pages, each once, as they're arranged there. A tap on a
// widget follows its link, or opens its app, in the app's window. What's under it (Open, Open at Login, Quit) is the
// app's own, around this screen. A Mac's menu bar is AppKit's own, not this (client/native/mac).
//
// A client's own screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into
// dist/panel.html, which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Skeleton, WidgetCard, cn } from "@one/ui"
import { home } from "./lib/model.js"

/// Each widget on the user's pages once, in their pages' order, at its medium size.
function widgets() {
  const seen = new Set(), out = []
  for (const p of home.pages()) {
    for (const i of home.items(p)) {
      const w = i.widget
      if (!w || seen.has(`${w.app}.${w.widget}`)) continue
      seen.add(`${w.app}.${w.widget}`)
      out.push({ w: { ...w, title: w.title || home.label(home.app(w.app)) || w.widget }, p })
    }
  }
  return out
}

/// What a widget's buttons and links do here: run its tool as the user, then draw again; open where it links, or
/// its app, in the app's window.
const host = (p) => ({
  async run(app, tool, input) {
    try {
      await home.call(app, tool, input)
    } catch (err) {
      return one.plain(err).title
    }
    await home.loadWidgets([p.scope])
    draw()
  },
  follow(link, w) {
    if (typeof link === "string") return void (/^https?:/i.test(link) && one.link(link))
    one.open(link?.screen || "main", link?.params || {}, w.app, { scope: p.scope })
  },
})

/// The user's devices, this one first.
function devices() {
  // This device: the computer's own node, as the app says (`here`), not whichever node answered.
  const state = home.data.state, me = state?.user?.id, here = one.params?.here
  const mine = (state?.network?.devices || []).filter((d) => !d.user || d.user === me)
  return mine.map((d) => ({ key: d.key, name: String(d.name || "").replace(/\.local$/, ""), online: d.online === true, here: !!here && d.key === here }))
    .sort((a, b) => b.here - a.here || a.name.localeCompare(b.name))
}

let draw = () => {}

function Panel() {
  const [, setN] = useState(0)
  draw = () => setN((n) => n + 1)
  useEffect(() => {
    home.load(draw).catch(() => draw())
    home.watch(draw)
  }, [])
  const state = home.data.state
  if (!state) return <div className="flex flex-col gap-2 p-4"><Skeleton className="h-5 w-32" /><Skeleton className="h-4 w-48" /></div>
  const shown = widgets()
  return (
    <div className="flex flex-col gap-4 p-4 text-sm">
      <section className="flex flex-col gap-1.5">
        <h2 className="text-xs font-medium text-muted-foreground">Your Devices</h2>
        {devices().map((d) => (
          <div key={d.key} className="flex items-center gap-2">
            <span className={cn("size-[7px] shrink-0 rounded-full", d.online || d.here ? "bg-foreground" : "bg-muted-foreground/40")} />
            <span className="min-w-0 flex-1 truncate">{d.name}</span>
            {d.here ? <span className="text-muted-foreground">This device</span> : !d.online && <span className="text-muted-foreground/70">Offline</span>}
          </div>
        ))}
      </section>
      {shown.length > 0 && (
        <section className="flex flex-col gap-2">
          <h2 className="text-xs font-medium text-muted-foreground">Widgets</h2>
          {shown.map(({ w, p }) => {
            const h = host(p), kind = `${w.app}.${w.widget}`
            return (
              <WidgetCard key={kind} w={w} host={h} tree={home.tree(w, "medium")} className="cursor-pointer rounded-2xl"
                onClick={(e) => !e.defaultPrevented && h.follow(home.tree(w, "medium")?.link || { screen: "main" }, w)} />
            )
          })}
        </section>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Panel />)
