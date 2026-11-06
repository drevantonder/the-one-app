// The App Store: every app, built in or made here, with Add to… or Manage on each; and, where there's room, the app
// opened beside the list.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Icon, Input, List, ListRow, Problem, Split, useWidth, ListSkeleton } from "@one/ui"
import { AppPage, onAPage } from "../lib/app-page.jsx"
import { Failed, Section, said, useList } from "../lib/store.jsx"

// The sections: the apps built into the release, and those made in this network (each network words that its own way,
// in its settings).
const MADE_HERE = one.config?.made_here || "Made here"
const WIDE = 720

function Row({ a, places, selected, open, add, restore }) {
  const action = a.deleted
    ? a.mine && <Button size="sm" variant="secondary" onClick={(e) => (e.stopPropagation(), restore(a))}>Restore</Button>
    : <Button size="sm" variant="secondary" onClick={(e) => (e.stopPropagation(), add(a))}>{onAPage(a, places) ? "Manage" : "Add to…"}</Button>
  // The row opens the app and its button does its own thing, so the row is not a button itself (a button has none inside).
  return (
    <div
      role="button"
      tabIndex={0}
      className="outline-none focus-visible:bg-accent/50"
      onClick={() => open(a)}
      onKeyDown={(e) => e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && (e.preventDefault(), open(a))}
    >
      <ListRow
        leading={<AppIcon app={a} size={48} />}
        title={a.title}
        sub={[a.tagline || a.description, !a.screens && "Tools\u00a0only"].filter(Boolean).join(" · ")}
        wrap
        trailing={action}
        selected={selected}
        className="cursor-pointer py-2 hover:bg-accent/50 data-[selected]:bg-accent"
      />
    </div>
  )
}

function Main() {
  const list = useList()
  // Accounts' Add Account opens it with { accounts: true }: only the apps that sign in to an account, until All Apps.
  const [accounts, setAccounts] = useState(!!one.params.accounts)
  const [query, setQuery] = useState("")
  const [shown, setShown] = useState(null)
  const [restored, setRestored] = useState([])
  const [error, setError] = useState("")
  const [ref, width] = useWidth()
  const beside = width >= WIDE

  useEffect(() => {
    one.title("App Store")
  }, [])

  const add = (a) => one.open("add", { app: a.name }, undefined, { sheet: true }).then(list.reload)
  const restore = (a) => {
    setError("")
    setRestored((r) => [...r, a.name])
    one.call("restore", { app: a.name }).catch((err) => (setRestored((r) => r.filter((n) => n !== a.name)), setError(said(err).title)))
  }
  const open = (a) => (beside ? setShown(a.name) : one.open("app", { app: a.name }))

  let rows
  if (list.data) {
    const { apps, places } = list.data
    const q = query.trim().toLowerCase()
    const found = apps
      .map((a) => (restored.includes(a.name) ? { ...a, deleted: null } : a))
      .filter((a) => (!accounts || a.keeps_accounts) && (!q || [a.title, a.name, a.tagline || "", a.description || ""].some((t) => t.toLowerCase().includes(q))))
    const section = (title, of) => of.length > 0 && (
      <Section key={title} title={title}>
        <List>{of.map((a) => <Row key={a.name} a={a} places={places} selected={beside && shown === a.name} open={open} add={add} restore={restore} />)}</List>
      </Section>
    )
    rows = found.length
      ? <>{section("Built In", found.filter((a) => a.built_in && !a.deleted))}{section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted))}{section("Recently Deleted", found.filter((a) => a.deleted))}</>
      : <Empty>{query.trim() ? `Nothing matches “${query.trim().toLowerCase()}”.` : "No results."}</Empty>
  } else rows = list.error ? <Failed error={list.error} retry={list.reload} /> : <ListSkeleton rows={4} />

  const top = (
    <div className="flex flex-col gap-3">
      <div className="relative">
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
    <div className="p-4">
      <div ref={ref}>
        <Split
          wide={WIDE}
          list={<div className="flex flex-col gap-6">{top}{error && <Problem title={error} />}{rows}</div>}
          detail={beside && shown && list.data && (
            <AppPage key={shown} name={shown} list={list.data} reloadList={list.reload} onOpen={setShown} onGone={() => setShown(null)} />
          )}
        />
      </div>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
