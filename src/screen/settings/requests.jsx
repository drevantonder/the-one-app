// What waits for this person: items apps and agents asked them for, and group invites.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into
// dist/requests.html, which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the
// bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, List, Problem, Skeleton } from "@one/ui"

const api = one.api
const COLORS = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"]
const names = (list) => new Intl.ListFormat(undefined, { type: "conjunction" }).format(list)

// A steady colour and initials for a group, from its id and name.
function Avatar({ id, name, size = 32 }) {
  let h = 0
  for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0
  const initials = String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase()
  return (
    <span
      aria-hidden="true"
      className="inline-grid shrink-0 place-items-center rounded-full font-semibold text-white"
      style={{ width: size, height: size, fontSize: size * 0.4, background: COLORS[h % COLORS.length] }}
    >
      {initials}
    </span>
  )
}

// Shows after `wait` ms: grey shapes only for a slow load, so a fast one shows nothing in between.
function useLater(wait) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const timer = setTimeout(() => setLate(true), wait)
    return () => clearTimeout(timer)
  }, [wait])
  return late
}

function Requests() {
  const [data, setData] = useState(null)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  // Rows gone at once, as they're answered: put back, with why, if that fails.
  const [gone, setGone] = useState([])
  const [busy, setBusy] = useState(null)
  const slow = useLater(100)

  async function load() {
    try {
      const [state, apps, asked] = await Promise.all([api("GET", "/v1/state"), api("GET", "/v1/apps"), api("GET", "/v1/vault/requests")])
      setData({ state, apps, asked })
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.largeTitle("Requests")
    load()
    one.on("changed", load)
  }, [])

  // Each button does its one thing, and its row goes at once: Join as it's tapped, put back with why if it fails;
  // Choose once the item's chosen.
  async function answer(key, run, early) {
    if (busy) return
    setBusy(key)
    setError("")
    try {
      if (early) setGone((g) => [...g, key])
      if (await run()) setGone((g) => [...g, key])
    } catch (e) {
      setGone((g) => g.filter((k) => k !== key))
      setError(one.plain(e).title)
    }
    setBusy(null)
    await load()
    setGone((g) => g.filter((k) => k !== key))
  }

  let body
  if (!data) {
    body = problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      : slow ? <List aria-busy="true" aria-label="Loading">{[0, 1].map((n) => <Placeholder key={n} />)}</List> : null
  } else {
    const { state, apps, asked } = data
    const i = state.install || {}, me = state.user?.id
    const app = (name) => apps.find((a) => a.name === name) || { name }
    const userName = (id) => (i.users || []).find((u) => u.id === id)?.name || "Someone"
    const vault = asked.filter((r) => r.state === "waiting" && !gone.includes(`request:${r.id}`))
    const invites = (i.groups || []).filter((g) => g.invited.some((x) => x.user === me) && !gone.includes(`join:${g.id}`))
    body = !vault.length && !invites.length ? <Empty>No requests.</Empty> : (
      <>
        {!!vault.length && (
          <Section title="Vault">
            {vault.map((r) => (
              <Row
                key={r.id}
                leading={<AppIcon app={app(r.app)} size={32} />}
                title={`${app(r.app).title || r.app} wants ${r.title || ""}`}
                sub={[r.agent || r.by, r.why].filter(Boolean).join(" · ")}
                action="Choose"
                disabled={!!busy}
                run={() => answer(`request:${r.id}`, () => one.request(r.scope, r.id), false)}
              />
            ))}
          </Section>
        )}
        {!!invites.length && (
          <Section title="Invites">
            {invites.map((g) => (
              <Row
                key={g.id}
                leading={<Avatar id={g.id} name={g.name} />}
                title={g.name}
                sub={names(g.members.map((m) => userName(m.user)))}
                action="Join"
                disabled={!!busy}
                run={() => answer(`join:${g.id}`, async () => (one.tap(), await api("POST", "/v1/accept", { group: g.id }), false), true)}
              />
            ))}
          </Section>
        )}
        {error && <Problem title={error} />}
      </>
    )
  }

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <h1 className="large-title text-3xl font-bold tracking-tight">Requests</h1>
      {body}
    </div>
  )
}

// A heading over a group of rows.
const Section = ({ title, children }) => (
  <section className="flex flex-col gap-2">
    <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
    <List>{children}</List>
  </section>
)

// A request: what asks, who and why under it, wrapping rather than cut, and its one button.
function Row({ leading, title, sub, action, run, disabled }) {
  return (
    <div className="flex items-center gap-3 px-3 py-2.5 text-sm">
      {leading}
      <div className="min-w-0 flex-1">
        <div className="font-medium break-words">{title}</div>
        {sub && <div className="break-words text-muted-foreground">{sub}</div>}
      </div>
      <Button variant="secondary" size="sm" disabled={disabled} onClick={run}>{action}</Button>
    </div>
  )
}

// A row's grey shape, while the list is coming.
function Placeholder() {
  return (
    <div className="flex items-center gap-3 px-3 py-2.5">
      <Skeleton className="size-8 rounded-lg" />
      <div className="flex-1 space-y-1.5">
        <Skeleton className="h-3.5 w-1/2" />
        <Skeleton className="h-3 w-1/3" />
      </div>
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Requests />)
