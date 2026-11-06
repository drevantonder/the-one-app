// Agent Devices' page: every agent device as a picture, those in use first, each with its name and one short line.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Empty, Grid, Problem } from "@one/ui"

const key = (c) => `${c.on || ""}/${c.name}`
const at = (c) => (c.on ? { device: c.name, on: c.on } : { device: c.name })
const host = (s) => String(s || "").split(".")[0]
const paused = (c) => c.state !== "running"
const screenless = (c) => c.borrowed?.no_screen
const capital = (s) => s.charAt(0).toUpperCase() + s.slice(1)
const titles = { windows: "Windows", android: "Android", iphone: "iPhone" }
const named = (c) => titles[c.name] || capital(c.name)
const off = (c) => (c.state === "paused" ? "Paused" : "Off")
const phone = (c) => c.kind === "android" || c.kind === "iphone"

function Glyph({ browser, c }) {
  const svg = { fill: "none", stroke: "currentColor", strokeWidth: 1.2, viewBox: "0 0 24 24" }
  if (phone(c)) return (
    <svg width="40" height="40" {...svg}>
      <rect x="7" y="2.5" width="10" height="19" rx="2" />
      <path d="M11 18.5h2" />
    </svg>
  )
  if (browser) return (
    <svg width="44" height="44" {...svg}>
      <rect x="3" y="4.5" width="18" height="15" rx="2" />
      <path d="M3 8.5h18" />
    </svg>
  )
  return (
    <svg width="48" height="48" {...svg}>
      <rect x="4" y="5" width="16" height="11" rx="1.5" />
      <path d="M2 19h20" />
    </svg>
  )
}

function Tile({ c, hosts, src }) {
  const b = c.borrowed
  const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : named(c)
  const line = [b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : paused(c) ? off(c) : "", hosts > 1 ? host(c.on) : ""].filter(Boolean).join(" · ")
  const opens = !screenless(c) && (!paused(c) || c.kind !== "linux")
  const Tag = opens ? "button" : "div"
  return (
    <Tag
      data-k={key(c)}
      onClick={opens ? () => one.open("device", at(c)) : undefined}
      className={`group block w-full min-w-0 text-left outline-none ${opens ? "cursor-pointer" : ""}`}
    >
      <div className="relative grid aspect-[16/10] place-items-center overflow-hidden rounded-xl bg-muted text-muted-foreground ring-1 ring-border ring-inset group-focus-visible:outline-3 group-focus-visible:outline-ring/50">
        {src ? (
          <img
            src={src}
            alt=""
            className={phone(c) ? "absolute left-1/2 top-2 h-[calc(100%-1rem)] w-auto -translate-x-1/2 rounded-xl" : "size-full object-cover"}
            style={paused(c) ? { filter: "grayscale(0.9) brightness(0.5)" } : undefined}
          />
        ) : (
          <Glyph browser={screenless(c)} c={c} />
        )}
      </div>
      <div className="mt-2 truncate text-sm font-medium">{name}</div>
      <div className="min-h-5 truncate text-sm text-muted-foreground">{line}</div>
    </Tag>
  )
}

function Page() {
  const [devices, setDevices] = useState(null)
  const [problem, setProblem] = useState(null)
  const [pics, setPics] = useState({}) // by the device it's on/name: a data URL
  const mem = useRef({ devices: null, hashes: {}, asked: {}, retried: {}, listed: "", busy: false })

  const seen = (k) => {
    const el = [...document.querySelectorAll("[data-k]")].find((t) => t.dataset.k === k)
    if (!el) return false
    const r = el.getBoundingClientRect()
    return r.bottom > 0 && r.top < innerHeight
  }
  function pictures() {
    const m = mem.current
    for (const c of m.devices || []) {
      const k = key(c)
      if (paused(c) || screenless(c) || Date.now() - (m.asked[k] || 0) < 20000 || !seen(k)) continue
      m.asked[k] = Date.now()
      one.call("_screen", { ...at(c), width: phone(c) ? 160 : 480, have: m.hashes[k] || "" }).then((r) => {
        if (r.age > 20 && !m.retried[k]) [m.retried[k], m.asked[k]] = [true, Date.now() - 16000]
        if (r.same) return
        const img = (r.content || []).find((x) => x.type === "image")
        if (!img) return
        m.hashes[k] = r.hash
        setPics((p) => ({ ...p, [k]: `data:${img.mimeType};base64,${img.data}` }))
      }).catch(() => {})
    }
  }

  async function refresh() {
    const m = mem.current
    try {
      const r = await one.call("_list", { have: m.listed })
      if (!r.same) {
        m.devices = (r.devices || []).filter((c) => !c.spare || c.borrowed)
        m.listed = r.hash || ""
        setDevices(m.devices)
      }
      setProblem(null)
    } catch (e) {
      m.listed = ""
      setProblem(one.plain(e))
      return
    }
    requestAnimationFrame(pictures)
  }

  async function tick() {
    const m = mem.current
    if (m.busy || document.visibilityState === "hidden") return
    m.busy = true
    try {
      await refresh()
    } finally {
      m.busy = false
    }
  }

  useEffect(() => {
    one.title("Agent Devices")
    tick()
    const timer = setInterval(tick, 5000)
    document.addEventListener("visibilitychange", tick)
    addEventListener("scroll", pictures, { passive: true })
    return () => {
      clearInterval(timer)
      document.removeEventListener("visibilitychange", tick)
      removeEventListener("scroll", pictures)
    }
  }, [])

  useEffect(() => {
    requestAnimationFrame(pictures)
  }, [devices])

  // Tried again every few seconds, so no button.
  if (problem) return <Problem title={problem.title} text={problem.next} />
  if (!devices) return <Empty>Loading…</Empty>
  if (!devices.length) return <Empty>None yet. Agents make one when they need a browser.</Empty>
  const hosts = new Set(devices.map((c) => c.on)).size
  const order = (c) => (c.borrowed ? 0 : paused(c) ? 2 : 1)
  const shown = [...devices].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name))
  return (
    <Grid min="16rem" max={4}>
      {shown.map((c) => <Tile key={key(c)} c={c} hosts={hosts} src={pics[key(c)]} />)}
    </Grid>
  )
}

createRoot(document.getElementById("root")).render(<Page />)
