// What waits for this user: items apps and agents asked them for, and group invites.
//
// A Settings screen written in React: @one/ui's build (ui/kit, `bun run build`) makes it into
// dist/requests.html, which the client builds in (ui/src/screen.rs). React and @one/ui come from the page; `one` is the
// bridge.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Avatar, Button, Empty, LargeTitle, List, ListRow, ListSkeleton, LoadFailure, Problem } from "@one/ui"
import { appNamed, asking } from "./agents.js"

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
      const [state, apps, asked] = await Promise.all([api("GET", "/api/state"), api("GET", "/api/apps"), api("GET", "/api/vault/requests")])
      setData({ state, apps, asked })
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    load()
    one.on("changed", load)
  }, [])

  // Each does its one thing, and its row goes at once: Join as it's tapped, put back with why if it fails; a request once
  // it's answered in its sheet.
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
    body = problem ? <LoadFailure problem={problem} retry={load} />
      : <ListSkeleton />
  } else {
    const { state, apps, asked } = data
    const i = state.network || {}, me = state.user?.id
    const app = (name) => appNamed(apps, name) || { name }
    const userName = (id) => (i.users || []).find((u) => u.id === id)?.name || "Someone"
    const vault = asked.filter((r) => r.state === "waiting" && !gone.includes(`request:${r.id}`))
    const invites = (i.groups || []).filter((g) => g.invited.some((x) => x.user === me) && !gone.includes(`join:${g.id}`))
    body = !vault.length && !invites.length ? <Empty>No requests.</Empty> : (
      <>
        <List>
          {vault.map((r) => (
            <ListRow
              key={r.id}
              leading={<AppIcon app={app(r.app)} size={32} />}
              title={`${asking(r, apps)} wants ${r.title || "a key"}`}
              sub={r.why || undefined}
              onClick={() => answer(`request:${r.id}`, () => one.request(r.scope, r.id), false)}
            />
          ))}
          {invites.map((g) => (
            <ListRow
              key={g.id}
              wrap
              leading={<Avatar id={g.id} name={g.name} />}
              title={g.name}
              sub={names(g.members.map((m) => userName(m.user)))}
              trailing={
                <div className="flex gap-1">
                  <Button variant="ghost" size="sm" disabled={!!busy} onClick={() => answer(`join:${g.id}`, async () => (await api("POST", "/api/member", { group: g.id, user: me, role: null }), false), true)}>Decline</Button>
                  <Button variant="secondary" size="sm" disabled={!!busy} onClick={() => answer(`join:${g.id}`, async () => (one.tap(), await api("POST", "/api/accept", { group: g.id }), false), true)}>Join</Button>
                </div>
              }
            />
          ))}
        </List>
        {error && <Problem title={error} />}
      </>
    )
  }

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <LargeTitle>Requests</LargeTitle>
      {body}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Requests />)
