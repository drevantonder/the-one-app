// One device, to watch and use: its screen fills the view, titled with what it's doing, under a line saying whose it is
// (or the profile it's signing in to), with Done Signing In while it's signing in to a profile.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Empty, LoadingButton, Loading, Problem } from "@one/ui"

const at = one.params.on ? { device: one.params.device, on: one.params.on } : { device: one.params.device }

const nameOf = (c) => {
  const b = c.borrowed
  return b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : c.spare ? "Spare device" : c.name.charAt(0).toUpperCase() + c.name.slice(1)
}
const lineOf = (c) => {
  const b = c.borrowed
  return b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : ""
}

// The device's screen, as it is: the shell zooms its screens to its text size, and this doesn't zoom with it.
function Frame({ url }) {
  const ref = useRef(null)
  useEffect(() => {
    const unzoom = () => {
      ref.current.style.zoom = String(1 / (parseFloat(getComputedStyle(document.documentElement).zoom) || 1))
    }
    unzoom()
    ref.current.focus()
    addEventListener("resize", unzoom)
    return () => removeEventListener("resize", unzoom)
  }, [])
  return (
    <iframe
      ref={ref}
      src={url}
      title="Device screen"
      allow="clipboard-read; clipboard-write; fullscreen; microphone; keyboard-map"
      className="absolute inset-0 size-full border-0 bg-black [color-scheme:dark]"
    />
  )
}

function Device() {
  const [device, setDevice] = useState(null)
  const [url, setUrl] = useState(null)
  const [note, setNote] = useState({ text: "Opening…", loading: true })
  const [signedIn, setSignedIn] = useState(false)
  const mem = useRef({ shown: null, framed: false, timer: null })

  function say(text, error) {
    mem.current.framed = false
    setUrl(null)
    setNote({ text, error })
  }

  async function open() {
    const r = await one.call("_view", { ...at, minutes: 240 })
    const address = typeof r === "string" ? r.split("\n")[0] : r.url
    if (!address || mem.current.framed) return
    mem.current.framed = true
    setNote(null)
    setUrl(address)
  }

  async function look() {
    const m = mem.current
    try {
      const all = (await one.call("_list", {})).devices || []
      const c = all.find((c) => c.name === at.device && (!at.on || c.on === at.on))
      if (!c) return m.shown && say("It’s been given back.")
      m.shown = c
      setDevice(c)
      one.title(nameOf(c))
      if (!m.framed) await open()
    } catch (e) {
      say(one.plain(e).title, true)
    }
  }

  async function done() {
    const m = mem.current
    try {
      await one.call("save_profile", { item: m.shown.borrowed.item })
      setSignedIn(true)
      say(`Saved to the ${m.shown.borrowed.profile || "browser"} profile.`)
      m.shown = null
      clearInterval(m.timer)
    } catch (e) {
      say(one.plain(e).title, true)
    }
  }

  useEffect(() => {
    look()
    mem.current.timer = setInterval(look, 5000)
    return () => clearInterval(mem.current.timer)
  }, [])

  const line = device ? lineOf(device) : ""
  const signing = !signedIn && !!device?.borrowed?.signing_in && !!device.borrowed.item
  return (
    <div className="fixed inset-0 flex flex-col bg-background">
      {(line || signing) && (
        <div className="flex h-[52px] shrink-0 items-center gap-3 px-4">
          <div className="min-w-0 flex-1 truncate text-sm text-muted-foreground">{line}</div>
          {signing && <LoadingButton run={done}>Done Signing In</LoadingButton>}
        </div>
      )}
      <div className="relative min-h-0 flex-1">
        {url && <Frame url={url} />}
        {note && (
          <div className="p-4">
            {note.error ? <Problem title={note.text} action={{ label: "Try Again", run: look }} />
              : note.loading ? <Loading>{note.text}</Loading>
              : <Empty>{note.text}</Empty>}
          </div>
        )}
      </div>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Device />)
