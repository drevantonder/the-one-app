// Computer Use: a tile for each of the person's own devices agents may use, with a switch, a live picture while an agent
// uses it, and the recent sessions below. The tiles fill the room: more columns, and bigger pictures, where there's
// more of it. Looked at again every 5 seconds while it can be seen.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Grid, Icon, List, ListRow, Problem, Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, Skeleton, Switch } from "@one/ui"

const host = (s) => String(s || "").split(".")[0]
const without = (o, k) => {
  const { [k]: _, ...rest } = o
  return rest
}
const picture = (r) => {
  const img = (r.content || []).find((c) => c.type === "image")
  return img ? `data:${img.mimeType};base64,${img.data}` : null
}

/// A device's picture while an agent uses it, else a grey tile with a laptop; filling the width its tile has.
function Pic({ src, ratio = "16 / 10", className = "", children }) {
  return (
    <div className={`relative grid place-items-center overflow-hidden rounded-xl bg-muted text-muted-foreground ring-1 ring-border ring-inset ${className}`} style={{ aspectRatio: ratio }}>
      {src ? <img src={src} alt="" className="size-full object-cover" /> : children}
    </div>
  )
}

const Laptop = () => (
  <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.2">
    <rect x="4" y="5" width="16" height="11" rx="1.5" />
    <path d="M2 19h20" />
  </svg>
)
const Phone = () => (
  <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.2">
    <rect x="7" y="2.5" width="10" height="19" rx="2" />
    <path d="M11 18.5h2" />
  </svg>
)

/// What a tile shows of a device. macOS asks that screen capture can't happen without Screen Recording, so a device
/// missing it doesn't answer capture calls: its switch stays off and its tile says so.
function look(d, { asking, note, fix }) {
  const gate = d.permissions && d.kind === "mac"
  const missing = gate && (!d.permissions.accessibility || !d.permissions.screen_recording)
  const title = d.title || d.device
  // What needs the person is one problem box on its tile; the line under the name is only ever grey.
  const box = note ? { title: note }
    : missing ? { title: `${!d.permissions.accessibility ? "Accessibility" : "Screen Recording"} is off`, text: `Agents can't use ${title} until it's on.`, warn: true, fix }
    : d.why ? { title: "Not answering", text: `Check that ${title} is on and online.`, warn: true }
    : null
  const line = box ? "" : asking || d.asking ? "Asking on the device…" : !d.on ? d.missing || "" : d.using ? `${d.using.agent} is using it` : ""
  return { title, line, box, off: !!(d.why || d.missing || asking || missing) }
}

function Tile({ d, shot, mirror, mirroring, asking, note, allow, fix, openMirror }) {
  const v = look(d, { asking, note, fix: () => fix(d.device) })
  const lab = mirroring && d.kind === "mac"
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Pic src={d.on && d.using ? shot : ""}><Laptop /></Pic>
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1 truncate text-sm font-medium">{v.title}</div>
        <Switch checked={!!d.on} disabled={v.off} onCheckedChange={(on) => allow(d.device, on)} aria-label="Let agents use it" />
      </div>
      {v.line && <div className="-mt-1 text-sm text-muted-foreground">{v.line}</div>}
      {v.box && <Problem warn={v.box.warn} title={v.box.title} text={v.box.text} action={v.box.fix && { label: "Fix", run: v.box.fix }} />}
      {lab && (
        <div className="mt-4 flex gap-4">
          <Pic src={d.mirroring?.running ? mirror : ""} ratio="9 / 16" className="w-24 shrink-0"><Phone /></Pic>
          <div className="flex min-w-0 flex-1 flex-col items-start gap-2">
            <div className="text-sm font-medium">iPhone Mirroring</div>
            {!d.mirroring?.running && (
              <>
                <div className="text-sm text-muted-foreground">Pair on your iPhone's Settings → General → iPhone Mirroring. Capture stays inside the Mirroring window only.</div>
                <Button size="sm" variant="secondary" onClick={() => openMirror(d.device)}>
                  <Icon name="play" /> Open
                </Button>
              </>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

/// What a Fix opens: the two permissions macOS asks, which is on, and how to turn on the rest. It's drawn from the
/// device as the latest look has it, so a permission turned on shows here.
function Fix({ device, onClose }) {
  // Kept while the sheet slides away, so it doesn't empty first.
  const kept = useRef(null)
  if (device) kept.current = device
  const d = kept.current
  const a11y = !!d?.permissions?.accessibility
  const sr = !!d?.permissions?.screen_recording
  const name = d?.title || d?.device
  const Off = ({ on }) => (on ? null : <span className="text-sm text-warning">Off</span>)
  return (
    <Sheet open={!!device} onOpenChange={(open) => !open && onClose()}>
      <SheetContent side="bottom" className="max-h-[90svh] overflow-y-auto rounded-t-xl">
        <SheetHeader>
          <SheetTitle>{name}</SheetTitle>
          <SheetDescription>macOS requires two permissions before agents can view or control this Mac.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 p-4 pt-0">
          <List>
            <ListRow leading={<AppIcon app={{ name: "accessibility", icon: "cursor-click" }} size={32} />} title="Accessibility" sub="Clicks, typing and window control" trailing={<Off on={a11y} />} />
            <ListRow leading={<AppIcon app={{ name: "screen", icon: "desktop" }} size={32} />} title="Screen Recording" sub="Lets agents see what's on screen" trailing={<Off on={sr} />} />
          </List>
          <section className="flex flex-col gap-2">
            <h2 className="px-1 text-sm font-medium text-muted-foreground">How to turn it on</h2>
            <List>
              <Step n={1} title={`Open System Settings on ${name}`}>
                Go to <b>Privacy &amp; Security</b> → <b>{a11y ? "Screen & System Audio Recording" : "Accessibility"}</b>.
              </Step>
              {!(a11y && sr) && (
                <Step n={2} title="Turn on The One App">
                  Enable the switch next to <b>The One App</b> and approve with Touch ID{a11y ? <>, then choose <b>Quit &amp; Reopen</b></> : ""}.
                </Step>
              )}
            </List>
          </section>
          <p className="text-center text-sm text-muted-foreground">This page updates automatically once permission is granted.</p>
        </div>
      </SheetContent>
    </Sheet>
  )
}

function Step({ n, title, children }) {
  return (
    <div className="flex gap-3 px-3 py-2.5 text-sm">
      <span className="grid size-6 shrink-0 place-items-center rounded-full bg-muted text-xs font-semibold text-muted-foreground">{n}</span>
      <div className="min-w-0">
        <div className="font-medium">{title}</div>
        <div className="text-muted-foreground">{children}</div>
      </div>
    </div>
  )
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
    <Grid min="20rem" max={4}>
      {[0, 1].map((i) => <Skeleton key={i} className="aspect-[16/10] rounded-xl" />)}
    </Grid>
  )
}

function Main() {
  const [data, setData] = useState(null) // { devices, sessions, mirroring }
  const [error, setError] = useState(null)
  const [pics, setPics] = useState({}) // by device: a data URL
  const [pending, setPending] = useState({}) // by device: the switch's new state, until the device has it
  const [asking, setAsking] = useState({}) // by device: it's being asked on the device
  const [notes, setNotes] = useState({}) // by device: why a switch went back
  const [fixing, setFixing] = useState(null) // the device a Fix is open for
  const asked = useRef(0)
  const busy = useRef(true)
  const latest = useRef(null)

  useEffect(() => {
    one.title("Computer Use")
  }, [])

  const note = (device, text) => {
    setNotes((n) => ({ ...n, [device]: text }))
    setTimeout(() => setNotes((n) => without(n, device)), 8000)
  }

  async function pictures(devices) {
    const shot = async (key, op, device) => {
      try {
        const src = picture(await one.call(op, { device, width: 640 }))
        if (src) setPics((p) => ({ ...p, [key]: src }))
      } catch {}
    }
    await Promise.all([
      ...devices.filter((d) => d.on && d.using).map((d) => shot(d.device, "screen", d.device)),
      ...devices.filter((d) => d.on && d.mirroring?.running).map((d) => shot(`${d.device}:mirror`, "_mirror_screen", d.device)),
    ])
  }

  // Whether this is the newest answer: an older look that comes back late is dropped. One that fails keeps what's shown.
  async function refresh() {
    const n = ++asked.current
    let r
    try {
      r = await one.call("_devices", {})
    } catch (e) {
      setError(e)
      return
    }
    if (n !== asked.current) return
    setError(null)
    setData(r)
    latest.current = r.devices
    await pictures(r.devices)
  }

  useEffect(() => {
    refresh().finally(() => (busy.current = false))
    const tick = async () => {
      if (busy.current || document.visibilityState === "hidden") return
      busy.current = true
      try {
        await refresh()
      } finally {
        busy.current = false
      }
    }
    const every = setInterval(tick, 5000)
    document.addEventListener("visibilitychange", tick)
    return () => (clearInterval(every), document.removeEventListener("visibilitychange", tick))
  }, [])

  async function allow(device, on) {
    setNotes((n) => without(n, device))
    setPending((p) => ({ ...p, [device]: on }))
    if (on) setAsking((a) => ({ ...a, [device]: true }))
    try {
      await one.call("_allow", { device, on })
    } catch (err) {
      note(device, one.plain(err).title)
    } finally {
      setPending((p) => without(p, device))
      setAsking((a) => without(a, device))
      refresh()
    }
  }

  function openMirror(device) {
    one.call("_mirror_launch", { device }).then(refresh, (err) => note(device, one.plain(err).title))
  }

  if (error && !data) return <Problem title={one.plain(error).title} text={one.plain(error).next} />
  if (!data) return <Waiting />
  const devices = data.devices.map((d) => (d.device in pending ? { ...d, on: pending[d.device] } : d))
  const sessions = data.sessions.slice(0, 10)
  return (
    <div className="flex flex-col gap-8">
      {devices.length ? (
        <Grid min="20rem" max={4} gap="1.5rem" className="items-start">
          {devices.map((d) => (
            <Tile
              key={d.device}
              d={d}
              shot={pics[d.device]}
              mirror={pics[`${d.device}:mirror`]}
              mirroring={data.mirroring}
              asking={!!asking[d.device]}
              note={notes[d.device]}
              allow={allow}
              fix={setFixing}
              openMirror={openMirror}
            />
          ))}
        </Grid>
      ) : (
        <Empty>Agents can use your Mac once you let them, in its menu bar.</Empty>
      )}
      {sessions.length > 0 && (
        <section className="flex flex-col gap-2">
          <h2 className="px-1 text-sm font-medium text-muted-foreground">Recent Sessions</h2>
          <List>
            {sessions.map((s) => (
              <ListRow
                key={s.id}
                title={s.agent}
                sub={`${s.title || s.device} · ${one.ago(s.time, { long: true })}${s.came_from ? ` · asked from ${host(s.came_from)}` : ""}`}
              />
            ))}
          </List>
        </section>
      )}
      <Fix device={fixing && devices.find((d) => d.device === fixing)} onClose={() => setFixing(null)} />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
