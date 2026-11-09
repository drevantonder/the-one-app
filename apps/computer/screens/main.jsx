// Computer Use: the user's own devices agents may use, each with a switch: a live picture, filling the room, while an
// agent uses one, and a row otherwise. Looked at again every 5 seconds while it can be seen.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Grid, Icon, List, ListRow, Problem, Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle, Skeleton, Switch } from "@one/ui"

const without = (o, k) => {
  const { [k]: _, ...rest } = o
  return rest
}
const picture = (r) => {
  const img = (r.content || []).find((c) => c.type === "image")
  return img ? `data:${img.mimeType};base64,${img.data}` : null
}

function Pic({ src, ratio = "16 / 10", className = "", children }) {
  return (
    <div className={`relative grid place-items-center overflow-hidden rounded-xl bg-muted text-muted-foreground ring-1 ring-border ring-inset ${className}`} style={{ aspectRatio: ratio }}>
      {src ? <img src={src} alt="" className="size-full object-cover" /> : children}
    </div>
  )
}

// macOS can't capture the screen without Screen Recording, so a device missing it doesn't answer capture calls.
function look(d, { asking, note, fix }) {
  const gate = d.permissions && d.kind === "mac"
  const missing = gate && (!d.permissions.accessibility || !d.permissions.screen_recording)
  const title = d.title || d.device
  const box = note ? { title: note }
    : missing ? { title: `${!d.permissions.accessibility ? "Accessibility" : "Screen Recording"} is off`, text: `Agents can’t use ${title} until it’s on.`, warn: true, fix }
    : d.why ? { title: "Not answering", text: `Check that ${title} is on and online.`, warn: true }
    : null
  const line = box ? "" : asking || d.asking ? "Asking on the device…" : !d.on ? d.missing || "" : d.using ? `${d.using.agent} is using it` : ""
  return { title, line, box, off: !!(d.why || d.missing || asking || missing) }
}

// A Mac's iPhone Mirroring: the picture while it's open, and how to open it when it isn't.
function Mirror({ d, src, open }) {
  if (d.mirroring?.running) return <Pic src={src} ratio="9 / 16" className="w-24" />
  return (
    <div className="flex items-center gap-3 text-sm">
      <Icon name="device-mobile" className="size-5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="font-medium">iPhone Mirroring</div>
        <div className="text-muted-foreground">Pair it in your iPhone’s Settings › General.</div>
      </div>
      <Button size="sm" variant="secondary" onClick={() => open(d.device)}>Open</Button>
    </div>
  )
}

// A device in use is its live picture; the rest are rows, since a picture of nothing says nothing.
function Tile({ d, shot, mirror, mirroring, note, allow, fix, asking, openMirror }) {
  const v = look(d, { asking, note, fix: () => fix(d.device) })
  return (
    <div className="flex min-w-0 flex-col gap-2">
      <Pic src={shot} />
      <div className="flex items-center gap-2">
        <div className="min-w-0 flex-1 truncate text-sm font-medium">{v.title}</div>
        <Switch checked={!!d.on} disabled={v.off} onCheckedChange={(on) => allow(d.device, on)} aria-label="Let agents use it" />
      </div>
      {v.line && <div className="-mt-1 text-sm text-muted-foreground">{v.line}</div>}
      {mirroring && d.kind === "mac" && <div className="mt-2"><Mirror d={d} src={mirror} open={openMirror} /></div>}
    </div>
  )
}

function Row({ d, mirror, mirroring, note, allow, fix, asking, openMirror }) {
  const v = look(d, { asking, note, fix: () => fix(d.device) })
  return (
    <div className="flex flex-col gap-2 px-3 py-2.5">
      <div className="flex items-center gap-3 text-sm">
        <Icon name={d.kind === "mac" ? "laptop" : "desktop"} className="size-5 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <div className="truncate font-medium">{v.title}</div>
          {v.line && <div className="truncate text-muted-foreground">{v.line}</div>}
        </div>
        <Switch checked={!!d.on} disabled={v.off} onCheckedChange={(on) => allow(d.device, on)} aria-label="Let agents use it" />
      </div>
      {v.box && <Problem warn={v.box.warn} title={v.box.title} text={v.box.text} action={v.box.fix && { label: "Fix", run: v.box.fix }} />}
      {mirroring && d.kind === "mac" && <Mirror d={d} src={mirror} open={openMirror} />}
    </div>
  )
}

function Fix({ device, onClose }) {
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
          <SheetDescription>Agents need both to use this Mac.</SheetDescription>
        </SheetHeader>
        <div className="flex flex-col gap-6 p-4 pt-0">
          <List>
            <ListRow leading={<AppIcon app={{ name: "accessibility", icon: "cursor-click" }} size={32} />} title="Accessibility" sub="Clicks, typing and windows" trailing={<Off on={a11y} />} />
            <ListRow leading={<AppIcon app={{ name: "screen", icon: "desktop" }} size={32} />} title="Screen Recording" sub="What’s on screen" trailing={<Off on={sr} />} />
          </List>
          <section className="flex flex-col gap-2">
            <h2 className="px-1 text-sm font-medium text-muted-foreground">Turn Them On</h2>
            <List>
              <Step n={1} title={`Open System Settings on ${name}`}>
                Go to <b>Privacy &amp; Security</b> → <b>{a11y ? "Screen & System Audio Recording" : "Accessibility"}</b>.
              </Step>
              {!(a11y && sr) && (
                <Step n={2} title={`Turn On ${one.config.name}`}>
                  Approve with Touch ID{a11y ? <>, then choose <b>Quit &amp; Reopen</b></> : ""}.
                </Step>
              )}
            </List>
          </section>
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
  const [data, setData] = useState(null) // { devices, mirroring }
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
    const shot = async (key, tool, device) => {
      try {
        const src = picture(await one.call(tool, { device, width: 640 }))
        if (src) setPics((p) => ({ ...p, [key]: src }))
      } catch {}
    }
    await Promise.all([
      ...devices.filter((d) => d.on && d.using).map((d) => shot(d.device, "screen", d.device)),
      ...devices.filter((d) => d.on && d.mirroring?.running).map((d) => shot(`${d.device}:mirror`, "_mirror_screen", d.device)),
    ])
  }

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
  const inUse = devices.filter((d) => d.on && d.using)
  const rest = devices.filter((d) => !inUse.includes(d))
  const part = (d) => ({ d, mirror: pics[`${d.device}:mirror`], mirroring: data.mirroring, asking: !!asking[d.device], note: notes[d.device], allow, fix: setFixing, openMirror })
  if (!devices.length) return <Empty>Agents can use your Mac once you let them, in its menu bar.</Empty>
  return (
    <div className="flex flex-col gap-6">
      {inUse.length > 0 && (
        <Grid min="20rem" max={4} gap="1.5rem" className="items-start">
          {inUse.map((d) => <Tile key={d.device} shot={pics[d.device]} {...part(d)} />)}
        </Grid>
      )}
      {rest.length > 0 && <List>{rest.map((d) => <Row key={d.device} {...part(d)} />)}</List>}
      <Fix device={fixing && devices.find((d) => d.device === fixing)} onClose={() => setFixing(null)} />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
