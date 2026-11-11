// Accounts: the accounts your apps sign in to, on every page of yours, listed together (GLOSSARY.md): a section per app
// that keeps accounts, its accounts (one that needs signing in again says so), then Add Account; then Add Account for
// Another App, which opens the App Store's apps that sign in to one. A page in the window, as an app's screen is. It
// shows as soon as the apps are known, each app's accounts filling in its own section as they come.
//
// A client's own screen written in React: @one/ui's build (ui/kit, `bun run build`) makes it into
// dist/accounts.html, which the client builds in (ui/src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Grid, Icon, LargeTitle, List, ListRow, LoadFailure, Problem, Skeleton } from "@one/ui"
import { home } from "./lib/model.js"

/// Only what this screen shows: the user's pages and the apps on them, not the widgets Home draws.
async function load() {
  ;[home.data.state, home.data.apps] = await Promise.all([one.api("GET", "/api/state"), one.api("GET", "/api/apps")])
}

/// What the screen draws from, as the apps are now: every page's apps that keep accounts, each once (on the first
/// page that has it, in the pages' order), and where each signs in.
function build() {
  const pages = home.pages(), seen = new Set(), withAccounts = []
  for (const p of pages) {
    for (const a of home.accountsOn(p)) {
      if (seen.has(a.name)) continue
      seen.add(a.name)
      withAccounts.push({ a, p, signIn: home.settings(a, p, true), store: home.inStore(a) })
    }
  }
  withAccounts.sort((x, y) => home.label(x.a).localeCompare(home.label(y.a)))
  return { may: pages.some((p) => home.canArrange(p.scope)) && !!home.addAccountApp(), withAccounts }
}

/// Add Account: the user's accounts in the Vault this app wants and doesn't use yet, to switch it on for one; with
/// none, its sign-in.
async function addAccount(a, p, at, stale, reload) {
  const signIn = home.settings(a, p, true)
  // Only its accounts: not what it needs besides (Google's apps' sign-in client).
  const types = Object.values(a.vault || {}).filter((need) => need.accounts).map((need) => need.type)
  const items = types.length ? await one.api("GET", "/api/vault").catch(() => []) : []
  const there = items.filter((i) => types.includes(i.type) && (i.wanted_by || []).includes(a.name) && !(i.apps || []).includes(a.name))
  if (!there.length) return signIn()
  const pick = there.map((i) => ({
    title: i.title,
    run: async () => {
      try {
        await one.api("POST", "/api/vault", { scope: i.scope, id: i.id, apps: [...i.apps, a.name] })
        stale.add(a.name)
        await reload()
      } catch (err) {
        home.failed(err)
      }
    },
  }))
  home.menu([...pick, { title: "Add Account", run: signIn }], at)
}

/// An app's accounts: a row each, one that needs signing in again saying so, with its Sign In.
function AppAccounts({ item, list }) {
  const { a, signIn, store } = item
  if (list === undefined) return <Skeleton role="status" aria-label="Loading" className="h-14 w-full max-w-2xl rounded-xl" />
  if (list instanceof Error) return <Problem warn title={one.plain(list).title} />
  // An app with no sign-in of its own, like Calendar, gets its accounts from a device: with none, what it needs,
  // opening its App Store page, which says where it can go.
  if (!list.length) {
    return !signIn && a.needs && store
      ? <List><ListRow title={a.needs} onClick={store} /></List>
      : null
  }
  return (
    <List>
      {list.map((x) => (
        <ListRow
          key={x.account}
          title={x.account}
          sub={x.ok ? undefined : <span className="inline-flex items-center gap-1 text-warning"><Icon name="warning-circle-fill" />{one.plain(x.why || "Needs signing in again").title}</span>}
          trailing={x.ok || !signIn ? undefined : <Button variant="outline" size="sm" onClick={() => signIn()}>Sign In</Button>}
        />
      ))}
    </List>
  )
}

function Accounts() {
  const [phase, setPhase] = useState("loading")
  const [problem, setProblem] = useState(null)
  const [view, setView] = useState(null)
  const [slow, setSlow] = useState(false)
  // Each app's accounts by its name: its list, the error that stopped it, or nothing yet.
  const [accounts, setAccounts] = useState({})
  const stale = useRef(new Set())
  const asked = useRef(new Set())
  const names = useRef([])

  // Each app's accounts, asked for all at once and listed once; a change to the app lists them again. Each answer
  // replaces only its own app's.
  function listAccounts(withAccounts) {
    names.current = withAccounts.map(({ a }) => a.name)
    for (const { a } of withAccounts) {
      if (asked.current.has(a.name) && !stale.current.has(a.name)) continue
      stale.current.delete(a.name)
      asked.current.add(a.name)
      home.call(a.name, home.accountsTool(a)).then((list) => (Array.isArray(list) ? list : [])).catch((err) => err).then((out) => setAccounts((all) => ({ ...all, [a.name]: out })))
    }
  }

  /// Reads the apps, then draws what they say.
  async function show() {
    await load()
    const next = build()
    setView(next)
    setPhase("ready")
    if (!next.missing) listAccounts(next.withAccounts)
  }
  const reload = () => show().catch((err) => (setProblem(one.plain(err)), setPhase("error")))

  useEffect(() => {
    const wait = setTimeout(() => setSlow(true), 100)
    reload()
    // An app's own change (a sign-in finished on its settings screen) lists its accounts again; a change to the apps or
    // the pages draws the screen again a moment later, so a burst of them draws it once.
    let soon
    one.on("changed", (e) => {
      ;(e.type === "changed" ? (e.apps?.length ? e.apps : names.current) : []).forEach((n) => stale.current.add(n))
      if (!["changed", "devices", "network"].includes(e.type)) return
      clearTimeout(soon)
      soon = setTimeout(() => show().catch(() => {}), 250)
    })
    return () => (clearTimeout(wait), clearTimeout(soon))
  }, [])

  if (phase === "error") return <LoadFailure problem={problem} retry={() => (setPhase("loading"), reload())} />
  if (phase !== "ready") {
    return slow ? (
      <div role="status" aria-label="Loading" className="flex max-w-2xl flex-col gap-3">
        <Skeleton className="h-9 w-40" />
        <Skeleton className="h-14 w-full rounded-xl" />
        <Skeleton className="h-14 w-full rounded-xl" />
      </div>
    ) : null
  }
  const { withAccounts, may } = view
  return (
    <div className="flex flex-col gap-8">
      <LargeTitle>Accounts</LargeTitle>
      <Grid min="20rem" max={3} gap="2rem" className="items-start">
      {withAccounts.map((item) => {
        const { a, p, signIn } = item
        return (
          <section key={a.name} className="flex min-w-0 flex-col gap-2">
            <div className="flex items-center gap-3">
              <AppIcon app={a} size={32} />
              <h2 className="min-w-0 flex-1 truncate text-base font-semibold">{home.label(a)}</h2>
              {signIn && accounts[a.name] !== undefined && !(accounts[a.name] instanceof Error) && (
                <Button variant="secondary" size="sm" onClick={(e) => addAccount(a, p, e.currentTarget, stale.current, async () => (await show()))}>Add Account</Button>
              )}
              <Button variant="ghost" size="icon-sm" aria-label={`More for ${home.label(a)}`} onClick={(e) => home.menu(home.actions(a, p), e.currentTarget)}>
                <Icon name="dots-three-outline-fill" className="size-[18px]" />
              </Button>
            </div>
            <AppAccounts item={item} list={accounts[a.name]} />
          </section>
        )
      })}
      </Grid>
      {!withAccounts.length && <Empty action={may ? { label: "Add Account", run: () => home.addAccountApp()?.() } : undefined}>No accounts yet.</Empty>}
      {may && withAccounts.length > 0 && <Button variant="outline" className="self-start" onClick={() => home.addAccountApp()?.()}>Add Account for Another App</Button>}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Accounts />)
