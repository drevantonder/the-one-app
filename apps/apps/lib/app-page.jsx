// An app's page, in the App Store's list beside it where there's room (main.jsx) and as a screen of its own where there
// isn't (app.jsx): its icon and name, where it runs, its schedules, what it works with, its source, and, for its owners,
// its activity and deleting it.
import { useEffect, useRef, useState } from "react"
import { AppIcon, Button, Empty, List, ListRow, Problem, Switch } from "@one/ui"
import { Section, said, useCall } from "./store.jsx"

const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1)
const ago = (t) => (one.ago(t) === "now" ? "just now" : one.ago(t) + " ago")
function soon(ms) {
  const m = Math.max(0, Math.round((ms - Date.now()) / 60000))
  return m < 1 ? "now" : m < 60 ? "in " + m + "m" : m < 1440 ? "in " + Math.round(m / 60) + "h" : "in " + Math.round(m / 1440) + "d"
}

export const onAPage = (a, places) => a.added.some((id) => places.some((p) => p.id === id))

/// `list` is what the App Store's list call gave; `onOpen(name)` opens another app's page (a row of Works With),
/// `onGone()` leaves this one once it's erased, and `reloadList` asks for the list again. `bar`: this page is a screen
/// of its own, whose bar shows the app's name once the heading scrolls away.
export function AppPage({ name, list, onOpen, onGone, reloadList, bar = false }) {
  const { apps, places } = list
  const a = apps.find((x) => x.name === name)
  const code = useCall("code", { app: name })
  const health = useCall("_health", { app: name })
  const where = useCall("devices", { app: name })
  const [error, setError] = useState("")
  const [busy, setBusy] = useState("")
  const [flipped, setFlipped] = useState({})
  const heading = useRef(null)

  const refresh = () => {
    reloadList()
    code.reload()
    health.reload()
    where.reload()
  }
  const run = async (key, go) => {
    setError("")
    setBusy(key)
    try {
      await go()
    } catch (err) {
      setError(said(err).title)
    }
    setBusy("")
    refresh()
  }

  useEffect(() => {
    if (!bar || !heading.current) return
    const title = a?.title || ""
    one.title("", title)
    const seen = new IntersectionObserver(([e]) => one.title(e.isIntersecting ? "" : title, title))
    seen.observe(heading.current)
    return () => seen.disconnect()
  }, [bar, a?.title])

  if (!a) {
    return <Empty action={{ label: "Back to the App Store", run: () => one.open("main") }}>This app isn't here. It may have been erased, or the link is old.</Empty>
  }

  const add = () => one.open("add", { app: name }, undefined, { sheet: true }).then(refresh)
  const editSource = () => one.open("source", { app: name }, undefined, { sheet: true }).then(refresh)
  const retire = async (tool) => {
    if (tool === "restore") return run("restore", () => one.call("restore", { app: name, confirm: true }))
    const erase = tool === "erase"
    const ok = await one.alert({
      title: erase ? `Erase ${a.title} for good?` : `Delete ${a.title}?`,
      text: erase ? "Its name can't be used again." : "You can restore it from Recently Deleted.",
      action: erase ? "Erase" : "Delete",
      danger: true,
    })
    if (!ok) return
    await run(tool, () => one.call(tool, { app: name, confirm: true }))
    if (erase) onGone()
  }
  const chooseDevice = (scope) => one.open("devices", { schedules: name, scope }, "settings")

  const link = (n) => apps.find((x) => x.name === n) || { name: n, title: n }
  const tag = a.deleted ? "Deleted " + ago(a.deleted.time) : [a.tagline, !a.screens && "Tools\u00a0only"].filter(Boolean).join(" · ")

  // Schedules run only on a device chosen for the app's schedules: one that's on with none says so, with the fix.
  // One a deploy removed no longer runs: it isn't the app's any more.
  const schedules = (health.data?.schedules || []).filter((s) => !s.removed)
  const unrun = schedules.find((s) => !s.invalid && s.on && s.runs_as && !(s.run_by || []).length && s.may_set_app_schedules)
  const manyPlaces = new Set(schedules.map((s) => s.scope)).size > 1
  const line = (s) => {
    if (s.invalid) return <span className="text-destructive">{s.invalid}</span>
    if (s.last && !s.last.ok) return <span className="text-destructive">Failed {ago(s.last.time)}: {said(s.last.error).title}</span>
    if (s.on && s.runs_as) {
      return cap(s.when) + (manyPlaces ? " · " + s.scope_name : "") + (s.next ? " · next " + soon(s.next) : "") + ((s.run_by || []).length ? " · on " + s.run_by.join(", ") : "")
    }
    if (s.on) return "Off. Switch it on to run it as you."
    return cap(s.when) + (s.switched ? " · off since " + ago(s.switched.time) : "")
  }
  const switchIt = (s, on) => {
    const key = s.name + "|" + s.scope
    setFlipped((f) => ({ ...f, [key]: on }))
    run("switch", () => one.call("switch", { app: name, schedule: s.name, scope: s.scope, on })).then(() => setFlipped((f) => ({ ...f, [key]: undefined })))
  }

  const w = where.data
  const move = () => one.open("add", { app: name }, undefined, { sheet: true }).then(refresh)
  const owner = a.mine && a.added && !a.deleted
  const busyAny = !!health.data && !!(health.data.activity?.length || health.data.insights?.errors?.length || health.data.insights?.usage?.length || health.data.insights?.cost?.length)
  const repo = a.source && /^http:[/][/](localhost|127[.]0[.]0[.]1):[0-9]+[/]repos[/]([a-z0-9][a-z0-9._-]*?)([.]git)?[/]?$/.exec(a.source)?.[2]

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4">
        <div className="flex gap-4">
          <AppIcon app={a} size={100} />
          <div className="flex min-w-0 flex-1 flex-col">
            <h1 ref={heading} className="text-2xl leading-tight font-semibold">{a.title}</h1>
            {tag && <p className="truncate text-muted-foreground">{tag}</p>}
            <div className="mt-auto pt-2">
              {a.deleted
                ? a.mine && <Button variant="secondary" size="sm" disabled={busy === "restore"} onClick={() => retire("restore")}>Restore</Button>
                : <Button variant="secondary" size="sm" onClick={add}>{onAPage(a, places) ? "Manage" : "Add to…"}</Button>}
            </div>
          </div>
        </div>
        {a.description && <p className="text-sm leading-relaxed">{a.description}</p>}
      </div>

      {error && <Problem title={error} />}

      {w?.problem && <Problem warn title={w.problem.title} text={w.problem.text} action={w.problem.move ? { label: "Move", run: move } : undefined} />}
      {w && w.runs_on.length > 0 && (
        <Section title="Runs On">
          <List>
            {w.runs_on.map((r) => (
              <ListRow
                key={r.node + r.scope}
                title={r.name || "A removed node"}
                sub={r.name && (!r.online ? "Off" : r.problem)}
                trailing={w.nodes.length > 0 && <Button size="sm" variant="outline" onClick={move}>Move</Button>}
              />
            ))}
          </List>
        </Section>
      )}

      {schedules.length > 0 && (
        <div className="flex flex-col gap-4">
          {unrun && (
            <Problem
              warn
              title={`${a.title || name} needs a device to run its schedules`}
              text="Its schedules run on a device you choose, while it’s on."
              action={{ label: "Choose a Device", run: () => chooseDevice(unrun.scope) }}
            />
          )}
          <Section title="Schedules">
            <List>
              {schedules.map((s) => {
                const key = s.name + "|" + s.scope
                const on = flipped[key] ?? !!(s.on && s.runs_as)
                return (
                  <ListRow
                    key={key}
                    title={s.description || s.name}
                    sub={line(s)}
                    trailing={!s.invalid && <Switch checked={on} disabled={!s.may_switch || busy === "switch"} aria-label={s.description || s.name} onCheckedChange={(v) => switchIt(s, v)} />}
                  />
                )
              })}
            </List>
          </Section>
        </div>
      )}

      {a.uses.apps.length > 0 && (
        <Section title="Works With">
          <List>{a.uses.apps.map((n) => <AppRow key={n} x={link(n)} onOpen={onOpen} />)}</List>
        </Section>
      )}
      {a.from && (
        <Section title="Forked From">
          <List><AppRow x={link(a.from)} onOpen={onOpen} /></List>
        </Section>
      )}
      {a.forks.length > 0 && (
        <Section title="Forks">
          <List>{a.forks.map((n) => <AppRow key={n} x={link(n)} onOpen={onOpen} />)}</List>
        </Section>
      )}

      {/* Where its code is kept: Source opens it, a repo in Repos or a page in the browser; its owners add or change it. */}
      {(a.source || (owner)) && (
        <div className="flex flex-col items-start gap-2">
          {a.source && <List><ListRow title="Source" onClick={() => (repo ? one.open("repo", { repo }, "repos") : one.link(a.source))} /></List>}
          {owner && <Button variant="outline" size="sm" onClick={editSource}>{a.source ? "Edit Source Link" : "Add Source Link"}</Button>}
        </div>
      )}

      {(busyAny || owner || (a.mine && a.deleted)) && (
        <div className="flex flex-col items-start gap-4">
          {busyAny && <List><ListRow title="Activity" onClick={() => one.open("activity", { app: a.name })} /></List>}
          {owner && <Button variant="destructive" size="sm" disabled={busy === "delete"} onClick={() => retire("delete")}>Delete App</Button>}
          {a.mine && a.deleted && <Button variant="destructive" size="sm" disabled={busy === "erase"} onClick={() => retire("erase")}>Erase App</Button>}
        </div>
      )}

      <p className="text-center text-sm text-muted-foreground">
        Made by {a.owner_name}
        {code.data && <> · <button className="underline-offset-4 hover:underline" onClick={() => one.open("code", { app: a.name, dir: "" })}>View code</button></>}
      </p>
    </div>
  )
}

function AppRow({ x, onOpen }) {
  return <ListRow leading={<AppIcon app={x} size={30} />} title={x.title} onClick={() => onOpen(x.name)} />
}
