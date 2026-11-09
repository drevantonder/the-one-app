// The App Store: every app, built in or made here, as tiles that fill the room, each with Add or Open; and, where there's
// room, the app opened beside them.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Grid, Icon, Input, LargeTitle, Pending, Problem, Section, Split, useWidth } from "@one/ui"
import { AppPage } from "../lib/app-page.jsx"
import { onAPage, openApp, said, useApps } from "../lib/store.jsx"

// The sections: the apps built into the release, and those made in this network (each network words that its own way,
// in its settings).
const MADE_HERE = one.config?.made_here || "Made here"
const WIDE = 900

function Tile({ a, list, selected, open, add, restore }) {
  const { places, admin } = list
  const me = useRef(null)
  // The tiles narrow to one column as an app opens beside them, so the one opened could end up out of sight.
  useEffect(() => {
    if (selected) me.current.scrollIntoView({ block: "nearest" })
  }, [selected])
  const stop = (run) => (e) => (e.stopPropagation(), run())
  const small = (label, run) => <Button size="sm" variant="secondary" onClick={stop(run)}>{label}</Button>
  const action = a.deleted
    ? a.mine && small("Restore", () => restore(a))
    : onAPage(a, places)
      ? a.opens ? small("Open", () => openApp(a, places)) : <span className="text-sm text-muted-foreground">Added</span>
      : (a.deployed || admin) && small("Add", () => add(a))
  // The tile opens the app and its button does its own thing, so the tile is not a button itself (a button has none inside).
  return (
    <div
      ref={me}
      role="button"
      tabIndex={0}
      data-selected={selected || undefined}
      className="flex min-w-0 cursor-pointer items-center gap-3 rounded-xl p-2 outline-none hover:bg-accent/50 focus-visible:bg-accent/50 data-[selected]:bg-accent"
      onClick={() => open(a)}
      onKeyDown={(e) => e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && (e.preventDefault(), open(a))}
    >
      <AppIcon app={a} size={56} />
      <div className="min-w-0 flex-1 text-sm">
        <div className="truncate font-medium">{a.title}</div>
        <div className="truncate text-muted-foreground">{a.tagline || a.description}</div>
      </div>
      {action}
    </div>
  )
}

function Main() {
  const apps = useApps()
  // Accounts' Add Account opens it with { accounts: true }: only the apps that sign in to an account, until All Apps.
  const [accounts, setAccounts] = useState(!!one.params.accounts)
  const [query, setQuery] = useState("")
  const [shown, setShown] = useState(null)
  const [restored, setRestored] = useState([])
  const [error, setError] = useState("")
  const [ref, width] = useWidth()
  const beside = width >= WIDE

  const add = (a) => {
    setError("")
    apps.add(a).catch((err) => setError(said(err).title))
  }
  const restore = (a) => {
    setError("")
    setRestored((r) => [...r, a.name])
    one.call("restore", { app: a.name }).catch((err) => (setRestored((r) => r.filter((n) => n !== a.name)), setError(said(err).title)))
  }
  const open = (a) => (beside ? setShown((s) => (s === a.name ? null : a.name)) : one.open("app", { app: a.name }))

  let tiles
  if (apps.data) {
    const q = query.trim().toLowerCase()
    const found = apps.data.apps
      .map((a) => (restored.includes(a.name) ? { ...a, deleted: null } : a))
      .filter((a) => (!accounts || a.keeps_accounts) && (!q || [a.title, a.name, a.tagline || "", a.description || ""].some((t) => t.toLowerCase().includes(q))))
    const section = (title, of) => of.length > 0 && (
      <Section key={title} title={title}>
        <Grid min="17rem" max={4} gap="0.25rem">
          {of.map((a) => <Tile key={a.name} a={a} list={apps.data} selected={beside && shown === a.name} open={open} add={add} restore={restore} />)}
        </Grid>
      </Section>
    )
    tiles = found.length
      ? <>{section("Built In", found.filter((a) => a.built_in && !a.deleted))}{section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted))}{section("Recently Deleted", found.filter((a) => a.deleted))}</>
      : <Empty>{query.trim() ? `Nothing matches “${query.trim().toLowerCase()}”.` : "No results."}</Empty>
  } else tiles = <Pending call={apps} rows={8} tiles="17rem" />

  const top = (
    <div className="flex flex-col gap-3">
      <div className="relative max-w-md">
        <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground" />
        <Input className="pl-8" placeholder="Search" aria-label="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      {accounts && (
        <div className="flex items-center gap-2">
          <p className="min-w-0 flex-1 text-sm text-muted-foreground">Apps that sign in to an account</p>
          <Button variant="ghost" size="sm" onClick={() => setAccounts(false)}>All Apps</Button>
        </div>
      )}
    </div>
  )

  return (
    <div ref={ref}>
      <Split
        wide={WIDE}
        list={<div className="flex flex-col gap-6"><LargeTitle>App Store</LargeTitle>{top}{error && <Problem title={error} />}{tiles}</div>}
        detail={beside && shown && apps.data && (
          <AppPage key={shown} name={shown} apps={apps} onOpen={setShown} onGone={() => setShown(null)} />
        )}
      />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
