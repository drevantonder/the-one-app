// A page's Accounts: the accounts its apps sign in to, a section per app that keeps accounts, its accounts (one that
// needs signing in again says so), then Add Account; then Add Account for Another App, which opens the App Store's apps
// that sign in to one. It shows as soon as the apps are known, each app's accounts filling in its own section as they
// come.
//
// A client's own screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into
// dist/accounts.html, which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Icon, List, ListRow, Problem, Skeleton } from "@one/ui"
import { home } from "./lib/model.js"

const scope = one.params?.scope || "user"

/// Only what this screen shows: the person's pages and the apps on them, not the widgets Home draws.
async function load() {
  ;[home.data.state, home.data.apps] = await Promise.all([one.api("GET", "/v1/state"), one.api("GET", "/v1/apps")])
}

/// What the screen draws from, as the apps are now: the page's apps that keep accounts, and where each signs in.
function build() {
  const p = home.page(scope)
  if (!p) return { missing: true }
  return {
    p,
    may: home.canArrange(p.scope) && !!home.addAccountApp(),
    withAccounts: home.accountsOn(p).map((a) => ({ a, signIn: home.settings(a, p, true), store: home.inStore(a) })),
  }
}

/// Add Account: the person's accounts in the Vault this app wants and doesn't use yet, to switch it on for one; with
/// none, its sign-in.
async function addAccount(a, p, at, stale, reload) {
  const signIn = home.settings(a, p, true)
  // Only its accounts: not what it needs besides (Google's apps' sign-in client).
  const types = Object.values(a.vault || {}).filter((need) => need.accounts).map((need) => need.type)
  const items = types.length ? await one.api("GET", "/v1/vault").catch(() => []) : []
  const there = items.filter((i) => types.includes(i.type) && (i.wanted_by || []).includes(a.name) && !(i.apps || []).includes(a.name))
  if (!there.length) return signIn()
  const pick = there.map((i) => ({
    title: i.title,
    run: async () => {
      try {
        await one.api("POST", "/v1/vault", { scope: i.scope, id: i.id, apps: [...i.apps, a.name] })
        stale.add(a.name)
        await reload()
      } catch (err) {
        home.failed(err)
      }
    },
  }))
  home.menu([...pick, { title: "Sign In to Another Account", run: signIn }], at)
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
      : <p className="px-1 text-sm text-muted-foreground">No accounts yet</p>
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
      home.call(a.name, "accounts").then((list) => (Array.isArray(list) ? list : [])).catch((err) => err).then((out) => setAccounts((all) => ({ ...all, [a.name]: out })))
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
    one.largeTitle("Accounts")
    const wait = setTimeout(() => setSlow(true), 100)
    reload()
    // An app's own change (a sign-in finished on its settings screen) lists its accounts again; a change to the apps or
    // the pages draws the screen again a moment later, so a burst of them draws it once.
    let soon
    one.on("changed", (e) => {
      ;(e.type === "changed" ? (e.apps?.length ? e.apps : names.current) : []).forEach((n) => stale.current.add(n))
      if (!["changed", "devices", "install"].includes(e.type)) return
      clearTimeout(soon)
      soon = setTimeout(() => show().catch(() => {}), 250)
    })
    return () => (clearTimeout(wait), clearTimeout(soon))
  }, [])

  if (phase === "error") return <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: () => (setPhase("loading"), reload()) }} />
  if (phase !== "ready") {
    return slow ? (
      <div role="status" aria-label="Loading" className="flex max-w-2xl flex-col gap-3">
        <Skeleton className="h-9 w-40" />
        <Skeleton className="h-14 w-full rounded-xl" />
        <Skeleton className="h-14 w-full rounded-xl" />
      </div>
    ) : null
  }
  if (view.missing) return <Empty>This device can't open this page. Sign out and in again to choose it.</Empty>
  const { p, withAccounts, may } = view
  return (
    <div className="flex flex-col gap-8">
      <h1 className="large-title text-3xl font-bold tracking-tight">Accounts</h1>
      {withAccounts.map((item) => {
        const { a, signIn } = item
        return (
          <section key={a.name} className="flex max-w-2xl flex-col gap-3">
            <div className="flex items-center gap-3">
              <AppIcon app={a} size={32} />
              <h2 className="min-w-0 flex-1 truncate text-lg font-semibold">{home.label(a)}</h2>
              <Button variant="ghost" size="icon-sm" aria-label={`More for ${home.label(a)}`} onClick={(e) => home.menu(home.actions(a, p), e.currentTarget)}>
                <Icon name="dots-three-outline-fill" className="size-[18px]" />
              </Button>
            </div>
            <AppAccounts item={item} list={accounts[a.name]} />
            {signIn && accounts[a.name] !== undefined && !(accounts[a.name] instanceof Error) && (
              <Button variant="secondary" className="self-start" onClick={(e) => addAccount(a, p, e.currentTarget, stale.current, async () => (await show()))}>Add Account</Button>
            )}
          </section>
        )
      })}
      {!withAccounts.length && <Empty className="mr-auto max-w-md px-0 py-0 text-left items-start" action={may ? { label: "Add Account", run: () => home.addAccountApp()?.() } : undefined}>The accounts your apps sign in to, like your email and chats, show here.</Empty>}
      {may && withAccounts.length > 0 && <Button variant="outline" className="self-start" onClick={() => home.addAccountApp()?.()}>Add Account for Another App</Button>}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Accounts />)
