// Backups: how they're going first, in words; then each place they go to, and the snapshots. Only admins add and
// remove places and edit them in the Vault, where the places are; everyone may back up now.
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/backups.html.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Icon, Input, Label, LargeTitle, List, ListAdd, ListRow, ListSkeleton, LoadFailure, Problem, ResponsiveDialog, Section, Spinner, Split, Tabs, TabsList, TabsTrigger } from "@one/ui"

const api = one.api
const name = (device) => String(device || "").replace(/\.local$/, "")
const list = (items) => [...items].join(", ")
const said = (err) => one.plain(err).title

// A time for users: "14:00" (today) or "3 Nov, 14:00"; with `day`, "Today, 14:00", "Yesterday, 14:00" or
// "Monday 3 November, 14:00".
function when(ms, day) {
  const d = new Date(ms), time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(ms).setHours(0, 0, 0, 0)) / 86400000)
  if (!day) return days === 0 ? time : `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })}, ${time}`
  if (days === 0) return `Today, ${time}`
  if (days === 1) return `Yesterday, ${time}`
  return d.toLocaleString(undefined, { weekday: "long", day: "numeric", month: "long", hour: "numeric", minute: "2-digit" })
}

function size(bytes) {
  const units = ["bytes", "KB", "MB", "GB", "TB"]
  let n = bytes, u = 0
  while (n >= 1000 && u < units.length - 1) (n /= 1000), u++
  return `${n >= 100 || u === 0 ? Math.round(n) : n.toFixed(1)} ${units[u]}`
}

// A place's icon: a disk for a folder, else a cloud.
const placeApp = (kind) => ({ name: kind, icon: kind === "folder" ? "hard-drive" : "cloud", color: "#8e8e93" })

// Whose data a snapshot holds, by name: Everyone, a user, a group.
const whose = (network, scope) => scope === "global" ? "Everyone"
  : scope.startsWith("user:") ? (network.users || []).find((u) => `user:${u.id}` === scope)?.name || "Someone"
  : scope.startsWith("group:") ? (network.groups || []).find((g) => `group:${g.id}` === scope)?.name || "A group"
  : scope

function Backups() {
  const [backups, setBackups] = useState(null)
  const [admin, setAdmin] = useState(false)
  const [network, setNetwork] = useState({})
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  // Backing up, from the tap until the places say how it went.
  const [waiting, setWaiting] = useState(false)
  // What's open beside the list: a place (by its id) or a snapshot (by its time).
  const [open, setOpen] = useState(null)
  const [adding, setAdding] = useState(false)
  const timer = useRef(0)

  async function load() {
    try {
      const [got, state] = await Promise.all([api("POST", "/v1/backups/status", {}), api("GET", "/v1/state")])
      setBackups(got)
      setAdmin(state.user?.role === "admin")
      setNetwork(state.network || {})
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.on("changed", load)
    // Times like "12 minutes ago" stay true.
    const every = setInterval(load, 30000)
    load()
    return () => (clearInterval(every), clearTimeout(timer.current))
  }, [])

  async function backUpNow() {
    setWaiting(true)
    setError("")
    try {
      await api("POST", "/v1/backups/now", {})
    } catch (e) {
      setError(said(e))
      setWaiting(false)
    }
    clearTimeout(timer.current)
    timer.current = setTimeout(() => (setWaiting(false), load()), 10000)
    load()
  }

  // Restores the newest backup of each place again, and says how it went.
  async function checkAgain() {
    setError("")
    try {
      await api("POST", "/v1/backups/check", {})
    } catch (e) {
      setError(said(e))
    }
    load()
  }

  if (!backups) {
    return (
      <div className="flex max-w-2xl flex-col gap-4">
        <LargeTitle>Backups</LargeTitle>
        {problem ? <LoadFailure problem={problem} retry={load} /> : <ListSkeleton rows={2} />}
      </div>
    )
  }

  const places = backups.places || []
  const snapshots = (backups.snapshots || []).slice(0, 8)
  const failing = places.filter((p) => p.problem).sort((a, b) => a.problem.since - b.problem.since)[0]
  const trouble = failing?.problem
  // Fine says nothing: a problem shows on top, with what fixes it, and its place's row says what's wrong in a few
  // words; otherwise each place says when it last backed up. The last backup is worth saying once it's missed a run.
  const missed = failing?.last && Date.now() - failing.last.time >= 3600000
  const place = open?.place && places.find((p) => p.id === open.place)
  const snapshot = open?.snapshot && snapshots.find((s) => s.time === open.snapshot)
  const selected = place || snapshot

  return (
    <>
      <Split
        title={place?.title || (snapshot && when(snapshot.time, true))}
        onClose={() => setOpen(null)}
        list={
          <div className="flex max-w-2xl flex-col gap-6">
            <LargeTitle action={waiting ? <Button disabled role="status"><Spinner />Backing Up…</Button> : !!places.length && !trouble && <Button onClick={backUpNow}>Back Up Now</Button>}>Backups</LargeTitle>

            {trouble ? (
              <Problem
                title={trouble.title || `Couldn’t back up to ${failing.title}`}
                text={[!trouble.check && missed && `Last backed up ${one.ago(failing.last.time, { long: true })}.`, trouble.fix].filter(Boolean).join(" ")}
                action={trouble.check ? { label: "Check Again", run: checkAgain } : waiting ? undefined : { label: "Back Up Now", run: backUpNow }}
              />
            ) : !places.length && <Empty action={admin ? { label: "Add Place", run: () => setAdding(true) } : undefined}>Nothing is backed up yet.</Empty>}
            {error && <Problem title={error} />}

            {!!places.length && (
              <Section title="Places">
                <List>
                  {places.map((p) => (
                    <ListRow
                      key={p.id}
                      leading={<AppIcon app={placeApp(p.kind)} size={30} />}
                      title={p.title}
                      sub={p.problem
                        ? <span className="flex items-center gap-1 text-warning"><Icon name="warning-circle-fill" /><span className="truncate">{p.problem.status}</span></span>
                        : p.last ? `Backed up ${one.ago(p.last.time, { long: true })}` : p.kind === "folder" ? `Disk on ${name(p.device)}` : `Off-site · ${p.where}`}
                      selected={place?.id === p.id}
                      onClick={() => setOpen({ place: p.id })}
                    />
                  ))}
                  {admin && <ListAdd onClick={() => setAdding(true)}>Add Place</ListAdd>}
                </List>
              </Section>
            )}

            {!!snapshots.length && (
              <Section title="Snapshots">
                <List>
                  {snapshots.map((s) => <ListRow key={s.time} title={when(s.time, true)} selected={snapshot?.time === s.time} onClick={() => setOpen({ snapshot: s.time })} />)}
                </List>
              </Section>
            )}
          </div>
        }
        detail={place ? <Place key={place.id} place={place} admin={admin} stopped={() => (setOpen(null), load())} />
          : snapshot && <Snapshot snapshot={snapshot} network={network} />}
      />
      {adding && <AddPlace backups={backups} open={adding} setOpen={setAdding} done={load} />}
    </>
  )
}

// A heading over a part of the screen.
// Facts as rows: a name, and what it is, which can be selected.
function Info({ rows }) {
  const shown = rows.filter(([, value]) => value)
  return (
    <List>
      {shown.map(([label, value]) => (
        <div key={label} className="flex min-h-11 items-center gap-3 px-3 py-2 text-sm">
          <span className="shrink-0 font-medium">{label}</span>
          <span className="min-w-0 flex-1 text-right break-words text-muted-foreground select-text">{value}</span>
        </div>
      ))}
    </List>
  )
}

// A place's own page: what's wrong and its fix, where it is, its last backup, its item in the Vault, and stopping.
function Place({ place: p, admin, stopped }) {
  const [error, setError] = useState("")
  const [busy, setBusy] = useState(false)
  const where = p.kind === "folder" ? [["Folder", p.where], ["Disk on", name(p.device)]] : [["Bucket", p.where], ["Endpoint", p.endpoint]]

  async function stop() {
    const ok = await one.alert({ title: `Stop backing up to ${p.title}?`, text: "What’s there now stays there.", action: "Stop Backing Up", danger: true })
    if (!ok) return
    one.tap("medium")
    setBusy(true)
    setError("")
    try {
      await api("POST", "/v1/backups/remove_place", { place: p.id })
      stopped()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <div className="flex items-center gap-4 pr-8">
        <AppIcon app={placeApp(p.kind)} size={56} />
        <h2 className="min-w-0 flex-1 text-xl font-semibold break-words select-text">{p.title}</h2>
      </div>
      {p.problem && <Problem title={one.plain(p.problem.text).title} text={p.problem.fix} />}
      <Info rows={[...where, ["Last backup", p.last && `${when(p.last.time, true)}, ${name(p.last.device)}`], ["Restore checked", p.checked && !p.checked.error ? `${when(p.checked.time, true)}, ${name(p.checked.device)}` : null]]} />
      {admin && <List><ListRow title="Edit in Vault" onClick={() => one.open("vault", { scope: "global", id: p.id })} /></List>}
      {error && <Problem title={error} />}
      {admin && <Button variant="destructive" className="self-start" disabled={busy} onClick={stop}>Stop Backing Up Here</Button>}
    </div>
  )
}

// A snapshot: when, where it went, whose data it holds, and which devices wrote it.
function Snapshot({ snapshot: s, network }) {
  const taken = new Date(s.time).toLocaleString(undefined, { dateStyle: "long", timeStyle: "short" })
  return (
    <div className="flex flex-col gap-6">
      <h2 className="pr-8 text-xl font-semibold">{when(s.time, true)}</h2>
      <Info rows={[["Taken", taken], ["Kept in", list(s.places)], ["Holds", list(s.scopes.map((x) => whose(network, x)))], ["Written by", list(s.devices.map(name))]]} />
    </div>
  )
}

const quiet = { autoCapitalize: "none", autoCorrect: "off", spellCheck: false }

// A labelled value in a list of them, written in the row.
function Field({ label, value, set, ...input }) {
  return (
    <Label className="px-3 py-1.5 font-normal">
      <span className="w-28 shrink-0 font-medium">{label}</span>
      <Input value={value} onChange={(e) => set(e.target.value)} {...quiet} className="h-8 min-w-0 flex-1 border-0 bg-transparent shadow-none focus-visible:ring-0 dark:bg-transparent" {...input} />
    </Label>
  )
}

// Adds a place: S3-compatible storage by its endpoint, bucket and key, or a disk plugged into the device, the disk first
// where one's plugged in. Each is tried before it's added, so a typo shows here, not in a failed backup; one that works
// just appears in the list.
function AddPlace({ backups, open, setOpen, done }) {
  const disks = backups.disks || []
  const [cloud, setCloud] = useState(!disks.length)
  const [busy, setBusy] = useState(false)
  const [problem, setProblem] = useState("")
  const [v, setV] = useState({ endpoint: "", bucket: "", key_id: "", secret: "", title: "", path: "" })
  const set = (k) => (value) => setV((all) => ({ ...all, [k]: value }))
  const filled = (k) => v[k].trim() !== ""
  const ready = cloud ? ["endpoint", "bucket", "key_id", "secret"].every(filled) : filled("path")

  async function add() {
    if (!ready || busy) return
    setBusy(true)
    setProblem("")
    const keys = cloud ? ["endpoint", "bucket", "key_id", "secret"] : ["path"]
    const body = Object.fromEntries(keys.map((k) => [k, v[k]]))
    if (filled("title")) body.title = v.title
    try {
      const reply = await api("POST", "/v1/backups/place", body)
      if (reply.id) {
        done()
        return setOpen(false)
      }
      setProblem(said(reply.tried.find((t) => t.error).error))
    } catch (e) {
      setProblem(said(e))
    }
    setBusy(false)
  }

  return (
    <ResponsiveDialog
      open={open}
      onOpenChange={setOpen}
      title="Add Place"
      action={<Button size="sm" disabled={!ready || busy} onClick={add}>{busy && <Spinner />}Add</Button>}
    >
      <form className="flex flex-col gap-4 overflow-y-auto p-4" onSubmit={(e) => (e.preventDefault(), add())}>
        <Tabs value={cloud ? "cloud" : "disk"} onValueChange={(t) => (setCloud(t === "cloud"), setProblem(""))}>
          <TabsList>
            <TabsTrigger value="cloud">Cloud Storage</TabsTrigger>
            <TabsTrigger value="disk">Disk</TabsTrigger>
          </TabsList>
        </Tabs>
        {cloud ? (
          <>
            <Section title="S3-Compatible Storage">
              <List>
                <Field label="Endpoint" value={v.endpoint} set={set("endpoint")} type="url" placeholder="https://….r2.cloudflarestorage.com" autoFocus />
                <Field label="Bucket" value={v.bucket} set={set("bucket")} placeholder="one-backups" />
                <Field label="Access Key ID" value={v.key_id} set={set("key_id")} />
                <Field label="Secret Key" value={v.secret} set={set("secret")} type="password" autoComplete="off" />
                <Field label="Name" value={v.title} set={set("title")} placeholder="Cloudflare R2" />
              </List>
            </Section>
            <p className="-mt-2 px-1 text-sm text-muted-foreground">Encrypted before it leaves. Use a key that reaches only this bucket.</p>
          </>
        ) : (
          <>
            <Section title={`A Disk on ${name(backups.here)}`}>
              <List>
                {disks.map((d) => (
                  <button
                    key={d.path}
                    type="button"
                    onClick={() => setV((all) => ({ ...all, path: d.path, title: d.name }))}
                    className="flex w-full items-center gap-3 px-3 py-2.5 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50"
                  >
                    <AppIcon app={placeApp("folder")} size={30} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate font-medium">{d.name}</span>
                      <span className="block truncate text-muted-foreground">{`${size(d.free)} free`}</span>
                    </span>
                    {v.path === d.path && <Icon name="check" className="size-4" />}
                  </button>
                ))}
                <Field label="Folder" value={v.path} set={(value) => setV((all) => ({ ...all, path: value }))} placeholder="/Volumes/…" />
              </List>
            </Section>
            <p className="-mt-2 px-1 text-sm text-muted-foreground">It’s backed up while it’s plugged in.</p>
          </>
        )}
        {problem && <Problem title={problem} />}
        <button type="submit" hidden />
      </form>
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Backups />)
