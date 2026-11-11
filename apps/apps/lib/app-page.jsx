// An app's page, in the App Store's list beside it where there's room (main.jsx) and as a screen of its own where there
// isn't (app.jsx): its icon and name, where it runs, its schedules and their last runs, what it works with, its source,
// and, for its owners, its insights and deleting it.
import { useEffect, useRef, useState } from "react"
import { AppIcon, Button, Empty, Grid, Hint, Icon, List, ListRow, ListSkeleton, Problem, Section, Switch, useCall } from "@one/ui"
import { PagesButton } from "./pages.jsx"
import { addAccount, onAPage, openApp, said } from "./store.jsx"

const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1)
const ago = (t) => (one.ago(t) === "now" ? "just now" : one.ago(t) + " ago")
function soon(ms) {
  const m = Math.max(0, Math.round((ms - Date.now()) / 60000))
  return m < 1 ? "now" : m < 60 ? "in " + m + "m" : m < 1440 ? "in " + Math.round(m / 60) + "h" : "in " + Math.round(m / 1440) + "d"
}

/// `apps` is the App Store's list and adding (`useApps`); `onOpen(name)` opens another app's page (a row of Works With),
/// `onGone()` leaves this one once it's erased. `bar`: this page is a screen of its own, whose bar shows the app's name
/// once the heading scrolls away.
export function AppPage({ name, apps: store, onOpen, onGone, bar = false }) {
  const { apps, places, admin } = store.data
  const a = apps.find((x) => x.name === name)
  const code = useCall("code", { app: name })
  const health = useCall("_health", { app: name })
  const where = useCall("devices", { app: name })
  const [error, setError] = useState("")
  const [busy, setBusy] = useState("")
  const [flipped, setFlipped] = useState({})
  const heading = useRef(null)

  const refresh = () => {
    store.reload()
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
    return <Empty action={{ label: "Back to the App Store", run: () => one.open("main") }}>This app isn’t here.</Empty>
  }

  const editSource = () => one.open("source", { app: name }, undefined, { sheet: true }).then(refresh)
  const retire = async (tool) => {
    if (tool === "restore") return run("restore", () => one.call("restore", { app: name, confirm: true }))
    const erase = tool === "erase"
    const ok = await one.alert({
      title: erase ? `Erase ${a.title} for good?` : `Delete ${a.title}?`,
      text: erase ? "Its name can’t be used again." : "You can restore it from Recently Deleted.",
      action: erase ? "Erase" : "Delete",
      danger: true,
    })
    if (!ok) return
    await run(tool, () => one.call(tool, { app: name, confirm: true }))
    if (erase) onGone()
  }
  const chooseDevice = (scope) => one.open("devices", { schedules: name, scope }, "settings")

  const link = (n) => apps.find((x) => x.name === n) || { name: n, title: n }
  const tag = a.deleted ? "Deleted " + ago(a.deleted.time) : !a.screens ? "Used by your AI" : ""

  // Schedules run only on a device chosen for the app's schedules: one that's on with none says so, with the fix.
  // One a deploy removed no longer runs: it isn't the app's any more.
  const schedules = (health.data?.schedules || []).filter((s) => !s.removed)
  const unrun = schedules.find((s) => !s.invalid && s.on && s.runs_as && !(s.run_by || []).length && s.may_set_app_schedules)
  const manyPlaces = new Set(schedules.map((s) => s.scope)).size > 1
  // Each schedule's last runs there, newest first: when, as whom, and what failed.
  const runsOf = (s) => (health.data?.activity || []).filter((r) => r.schedule === s.name && r.scope === s.scope).slice(0, 3)
  const ran = (r) => r.ok
    ? <span key={r.time} className="block truncate">Ran {ago(r.time)} as {r.user}</span>
    : <span key={r.time} className="block truncate text-destructive">Failed {ago(r.time)} as {r.user}: {said(r.error).title}</span>
  const line = (s) => {
    if (s.invalid) return <span className="text-destructive">{s.invalid}</span>
    if (s.on && s.runs_as) {
      return cap(s.when) + (manyPlaces ? " · " + s.scope_name : "") + (s.next ? " · next " + soon(s.next) : "") + ((s.run_by || []).length ? " · on " + s.run_by.join(", ") : "")
    }
    if (s.on) return "Off. Turn it on to run it as you."
    return cap(s.when) + (s.switched ? " · off since " + ago(s.switched.time) : "")
  }
  const switchIt = (s, on) => {
    const key = s.name + "|" + s.scope
    setFlipped((f) => ({ ...f, [key]: on }))
    run("switch", () => one.call("switch", { app: name, schedule: s.name, scope: s.scope, on })).then(() => setFlipped((f) => ({ ...f, [key]: undefined })))
  }

  const w = where.data
  // A computer to run its service on, from `at` (the button pressed).
  const move = async (at) => {
    const running = w.runs_on.find((r) => r.name)?.name
    const i = await one.menu(w.computers.map((n) => ({ title: n, checked: n === running })), at)
    if (i != null && w.computers[i] !== running) run("move", () => one.call("move", { app: name, computer: w.computers[i] }))
  }
  const owner = a.mine && a.deployed && !a.deleted
  const repo = a.source && /^http:[/][/](localhost|127[.]0[.]0[.]1):[0-9]+[/]repos[/]([a-z0-9][a-z0-9._-]*?)([.]git)?[/]?$/.exec(a.source)?.[2]

  const on = onAPage(a, places)
  const more = [
    ...(owner ? [{ title: a.source ? "Edit Source Link" : "Add Source Link", run: editSource }] : []),
    ...(owner ? [{ title: "Delete App", danger: true, section: true, run: () => retire("delete") }] : []),
    ...(a.mine && a.deleted ? [{ title: "Erase App", danger: true, run: () => retire("erase") }] : []),
  ]
  const showMore = async (e) => {
    const i = await one.menu(more, e.currentTarget)
    if (i != null) more[i].run()
  }
  const adds = a.deployed || admin
  // What's asked for apart comes in as one, rather than each pushing down what's under it.
  const answered = [code, health, where].every((c) => c.of !== null || c.error)

  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-col gap-4">
        <div className="flex gap-4">
          <AppIcon app={a} size={100} />
          <div className="flex min-w-0 flex-1 flex-col">
            <h1 ref={heading} className="text-2xl leading-tight font-semibold">{a.title}</h1>
            {tag && <p className="truncate text-muted-foreground">{tag}</p>}
            <div className="mt-auto flex items-center gap-2 pt-2">
              {a.deleted
                ? a.mine && <Button size="sm" disabled={busy === "restore"} onClick={() => retire("restore")}>Restore</Button>
                : <>
                  {on && a.opens && <Button size="sm" onClick={() => openApp(a, places)}>Open</Button>}
                  {on && !a.opens && a.signs_in && <Button size="sm" onClick={() => addAccount(a, places)}>Add Account</Button>}
                  {adds ? <PagesButton a={a} store={store} variant={on ? "secondary" : "default"} /> : <span className="text-sm text-muted-foreground">Ask an admin to add it first</span>}
                </>}
              {more.length > 0 && <Button variant="ghost" size="icon-sm" aria-label="More" onClick={showMore}><Icon name="dots-three" className="size-5" /></Button>}
            </div>
          </div>
        </div>
        {a.description && <p className="max-w-2xl text-sm leading-relaxed">{a.description}</p>}
      </div>

      {error && <Problem title={error} />}

      {answered ? <>
      {w?.problem && <Problem warn title={w.problem.title} text={w.problem.text} action={w.problem.move ? { label: "Move", run: () => move() } : undefined} />}
      <Grid min="24rem" max={3} gap="1.5rem" className="items-start">
      {w && w.runs_on.length > 0 && (
        <Section title="Runs On">
          <List>
            {w.runs_on.map((r) => (
              <ListRow
                key={r.runtime + r.scope}
                title={r.name || "A removed device"}
                sub={r.name && (r.online ? r.problem || "Online" : one.seen(r))}
                trailing={w.computers.length > 0 && <Button size="sm" variant="outline" onClick={(e) => move(e.currentTarget)}>Move</Button>}
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
                    sub={<><span className="block truncate">{line(s)}</span>{runsOf(s).map(ran)}</>}
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

      {(a.source || code.data || owner) && (
        <List>
          {/* Where its code is kept: a repo in Repos or a page in the browser. */}
          {a.source && <ListRow title="Source" onClick={() => (repo ? one.open("repo", { repo }, "repos") : one.link(a.source))} />}
          {code.data && <ListRow title="Code" onClick={() => one.open("code", { app: a.name, dir: "" })} />}
          {owner && <ListRow title="Insights" onClick={() => one.open("insights", { service: a.name }, "settings")} />}
        </List>
      )}
      </Grid>

      <Hint>Made by {a.owner_name}</Hint>
      </> : <ListSkeleton rows={3} />}
    </div>
  )
}

function AppRow({ x, onOpen }) {
  return <ListRow leading={<AppIcon app={x} size={30} />} title={x.title} onClick={() => onOpen(x.name)} />
}
