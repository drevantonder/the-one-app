// The App Store: every app, built in or made here, as tiles that fill the room, each with Add or Open; and, where there's
// room, the app opened beside them.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Grid, Hint, LargeTitle, LoadingButton, Pending, Problem, SearchInput, Section, Split, useWidth } from "@one/ui"
import { AppPage } from "../lib/app-page.jsx"
import { addAccount, onAPage, openApp, said, useApps } from "../lib/store.jsx"

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
  const small = (label, run) => <LoadingButton size="sm" variant="secondary" run={run}>{label}</LoadingButton>
  const action = a.deleted
    ? a.mine && small("Restore", () => restore(a))
    : onAPage(a, places)
      ? a.opens ? small("Open", () => openApp(a, places)) : a.signs_in ? small("Add Account", () => addAccount(a, places)) : <span className="text-sm text-muted-foreground">Added</span>
      : (a.deployed || admin) && small("Add", () => add(a))
  // The app is a button and its Add or Open another, side by side: one button never holds another.
  return (
    <div
      ref={me}
      data-selected={selected || undefined}
      className="flex min-w-0 items-center gap-3 rounded-xl p-2 hover:bg-accent/50 data-[selected]:bg-accent"
    >
      <button type="button" className="flex min-w-0 flex-1 cursor-pointer items-center gap-3 rounded-lg text-left outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50" onClick={() => open(a)}>
        <AppIcon app={a} size={56} />
        <div className="min-w-0 flex-1 text-sm">
          <div className="truncate font-medium">{a.title}</div>
          <div className="truncate text-muted-foreground">{a.tagline || a.description}</div>
        </div>
      </button>
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
  const [note, setNote] = useState("")
  const [ref, width] = useWidth()
  const beside = width >= WIDE

  const add = (a) => {
    setError("")
    setNote("")
    // Nothing of it appears to open: its own page says what comes next (an account, or the device it needs).
    apps.add(a).then((also) => (setNote(also), a.opens) || (beside ? setShown(a.name) : one.open("app", { app: a.name }))).catch((err) => setError(said(err).title))
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
    const failed = (a) => !a.deleted && apps.data.failing?.includes(a.name)
    const section = (title, of) => of.length > 0 && (
      <Section key={title} title={title}>
        <Grid min="17rem" max={4} gap="0.25rem">
          {of.map((a) => <Tile key={a.name} a={a} list={apps.data} selected={beside && shown === a.name} open={open} add={add} restore={restore} />)}
        </Grid>
      </Section>
    )
    tiles = found.length
      ? <>{section("Schedule Failed", found.filter(failed))}{section("Built In", found.filter((a) => a.built_in && !a.deleted && !failed(a)))}{section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted && !failed(a)))}{section("Recently Deleted", found.filter((a) => a.deleted))}</>
      : <Empty>{query.trim() ? `Nothing matches “${query.trim().toLowerCase()}”.` : "No results."}</Empty>
  } else tiles = <Pending call={apps} rows={8} tiles="17rem" />

  const top = (
    <div className="flex flex-col gap-3">
      <SearchInput className="max-w-md" placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
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
        list={<div className="flex flex-col gap-6"><LargeTitle>App Store</LargeTitle>{top}{error && <Problem title={error} />}{note && <Hint>{note}</Hint>}{tiles}</div>}
        detail={beside && shown && apps.data && (
          <AppPage key={shown} name={shown} apps={apps} onOpen={setShown} onGone={() => setShown(null)} />
        )}
      />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
