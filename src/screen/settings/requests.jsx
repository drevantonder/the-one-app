// What waits for this person: items apps and agents asked them for, and group invites.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into
// dist/requests.html, which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the
// bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, AppIcon, Button, Empty, List, ListRow, ListSkeleton, Problem } from "@one/ui"

const api = one.api
const names = (list) => new Intl.ListFormat(undefined, { type: "conjunction" }).format(list)

function Requests() {
  const [data, setData] = useState(null)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  // Rows gone at once, as they're answered: put back, with why, if that fails.
  const [gone, setGone] = useState([])
  const [busy, setBusy] = useState(null)

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
      : <ListSkeleton />
  } else {
    const { state, apps, asked } = data
    const i = state.network || {}, me = state.user?.id
    const app = (name) => apps.find((a) => a.name === name) || { name }
    const userName = (id) => (i.users || []).find((u) => u.id === id)?.name || "Someone"
    const vault = asked.filter((r) => r.state === "waiting" && !gone.includes(`request:${r.id}`))
    const invites = (i.groups || []).filter((g) => g.invited.some((x) => x.user === me) && !gone.includes(`join:${g.id}`))
    body = !vault.length && !invites.length ? <Empty>No requests.</Empty> : (
      <>
        {!!vault.length && (
          <Section title="Vault">
            {vault.map((r) => (
              <ListRow
                key={r.id}
                wrap
                leading={<AppIcon app={app(r.app)} size={32} />}
                title={`${app(r.app).title || r.app} wants ${r.title || ""}`}
                sub={[r.agent || r.by, r.why].filter(Boolean).join(" · ")}
                trailing={<Button variant="secondary" size="sm" disabled={!!busy} onClick={() => answer(`request:${r.id}`, () => one.request(r.scope, r.id), false)}>Choose</Button>}
              />
            ))}
          </Section>
        )}
        {!!invites.length && (
          <Section title="Invites">
            {invites.map((g) => (
              <ListRow
                key={g.id}
                wrap
                leading={<Avatar id={g.id} name={g.name} />}
                title={g.name}
                sub={names(g.members.map((m) => userName(m.user)))}
                trailing={<Button variant="secondary" size="sm" disabled={!!busy} onClick={() => answer(`join:${g.id}`, async () => (one.tap(), await api("POST", "/v1/accept", { group: g.id }), false), true)}>Join</Button>}
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

createRoot(document.getElementById("root")).render(<Requests />)
