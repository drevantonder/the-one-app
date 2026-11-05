// Device Activity: a card for each device with its memory, CPU, GPU and disk, and the containers it hosts below. The
// cards fill the room: more columns where there's more of it. Looked at again every 5 seconds while it can be seen.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Card, Empty, Grid, Problem, Skeleton } from "@one/ui"

const GB = 1e9
const GiB = 2 ** 30
// Bytes as GB: memory counts them in 2^30s, as macOS and Activity Monitor do, disks in 10^9s.
const size = (b, g = GB) => (b >= 10 * g ? `${Math.round(b / g)} GB` : b >= g ? `${(b / g).toFixed(1)} GB` : `${Math.round(b / (g / 1000))} MB`)
const mem = (b) => size(b, GiB)
const pct = (s) => (s && s.total ? Math.round((s.used * 100) / s.total) : null)

/// A bar for how much is used: neutral until it needs you, amber from 70%, red from 85%.
function Bar({ of, className = "" }) {
  const used = pct(of)
  if (used == null) return null
  const color = used >= 85 ? "bg-destructive" : used >= 70 ? "bg-warning" : "bg-foreground"
  return (
    <div role="progressbar" aria-valuenow={used} aria-valuemin={0} aria-valuemax={100} className={`h-1.5 overflow-hidden rounded-full bg-muted ${className}`}>
      <div className={`h-full rounded-full ${color}`} style={{ width: `${used}%` }} />
    </div>
  )
}

/// The last two minutes of a percentage, drawn as a line filling the room it has.
function Spark({ values, className }) {
  if (!values.length) return <div className="h-1.5 rounded-full bg-muted" />
  const n = Math.max(values.length, 24), w = 100, h = 28
  const pts = values.map((v, i) => [((n - values.length + i) / (n - 1)) * w, h - 1 - (v / 100) * (h - 2)])
  const d = pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(" ")
  return (
    <svg viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className={`block h-7 w-full ${className}`}>
      <path d={`${d} L${w} ${h} L${pts[0][0].toFixed(1)} ${h} Z`} fill="currentColor" opacity=".15" />
      <path d={d} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

function Stat({ label, value, children }) {
  return (
    <div className="min-w-0">
      <div className="mb-1 flex justify-between gap-2 text-sm whitespace-nowrap">
        <span className="text-muted-foreground">{label}</span>
        <span className="font-medium">{value}</span>
      </div>
      {children}
    </div>
  )
}

function Device({ d, guests }) {
  const sub = [d.os, d.host ? `on ${d.host}` : "", d.here ? "this device" : ""].filter(Boolean).join(" · ")
  const cpu = d.cpu[d.cpu.length - 1]
  const gpu = d.gpu[d.gpu.length - 1]
  const any = d.online && (d.memory || d.cpu.length || d.gpu.length || d.disk)
  return (
    <Card className={`@container min-w-0 gap-4 p-4 ${d.online ? "" : "opacity-55"}`}>
      <div className="flex items-baseline justify-between gap-2">
        <h2 className="min-w-0 truncate text-base font-semibold">{d.name}</h2>
        <span className="shrink-0 truncate text-sm text-muted-foreground">{d.online ? sub : "Offline"}</span>
      </div>
      {any && (
        <div className="grid grid-cols-1 gap-4 @sm:grid-cols-2">
          {d.memory && (
            <Stat label="Memory" value={d.memory.total ? `${mem(d.memory.used)} of ${mem(d.memory.total)}` : mem(d.memory.used)}>
              <Bar of={d.memory} />
            </Stat>
          )}
          {d.cpu.length > 0 && <Stat label="CPU" value={`${cpu}%`}><Spark values={d.cpu} className="text-foreground" /></Stat>}
          {d.gpu.length > 0 && <Stat label="GPU" value={`${gpu}%`}><Spark values={d.gpu} className="text-muted-foreground" /></Stat>}
          {d.disk && (
            <Stat label="Disk" value={`${size(d.disk.total - d.disk.used)} free`}>
              <Bar of={d.disk} />
            </Stat>
          )}
        </div>
      )}
      {guests.length > 0 && (
        <div className="-mx-4 -mb-4 divide-y border-t">
          {guests.map((g) => (
            <div key={g.name} className="flex items-center gap-3 px-4 py-2.5 text-sm">
              <span className="min-w-0 flex-1 truncate">{g.name}</span>
              <span className="min-w-0 shrink truncate text-muted-foreground">{g.kind === "podman" ? "podman" : "container"}</span>
              <Bar of={g.memory} className="w-16 min-w-4 shrink-[3]" />
              <span className="whitespace-nowrap text-muted-foreground">{g.memory ? mem(g.memory.used) : ""}</span>
            </div>
          ))}
        </div>
      )}
    </Card>
  )
}

function Main() {
  const [state, setState] = useState({})
  const busy = useRef(false)

  useEffect(() => {
    // One look at a time, and only while the screen can be seen; one that fails keeps what's there.
    const poll = async () => {
      if (busy.current || document.visibilityState === "hidden") return
      busy.current = true
      try {
        const { devices } = await one.call("_list", {})
        setState({ devices })
      } catch (error) {
        setState((s) => (s.devices ? s : { error }))
      } finally {
        busy.current = false
      }
    }
    poll()
    const every = setInterval(poll, 5000)
    document.addEventListener("visibilitychange", poll)
    return () => (clearInterval(every), document.removeEventListener("visibilitychange", poll))
  }, [])

  const { devices, error } = state
  if (error) return <Problem title={one.plain(error).title} text={one.plain(error).next} />
  if (!devices) return <Waiting />
  // Every device, and under each the containers it hosts that aren't devices themselves.
  const cards = []
  for (let i = 0; i < devices.length; i++) {
    const d = devices[i]
    if (d.kind) continue
    const guests = []
    for (let j = i + 1; j < devices.length && devices[j].host === d.name; j++) if (devices[j].kind) guests.push(devices[j])
    cards.push(<Device key={d.name} d={d} guests={guests} />)
  }
  if (!cards.length) return <Empty>No devices yet.</Empty>
  return <Grid min="22rem" max={4} className="items-start">{cards}</Grid>
}

/// A first answer that's slow shows as grey shapes, after a moment, so a quick one never flashes.
function Waiting() {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  if (!late) return null
  return (
    <Grid min="22rem" max={4}>
      {[0, 1, 2].map((i) => <Skeleton key={i} className="h-40 rounded-xl" />)}
    </Grid>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
