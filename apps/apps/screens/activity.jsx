// How an app is doing, for its owners: its errors, its schedules' recent runs, who used it, and its AI cost.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { Empty, List, ListRow, Pending, useCall } from "@one/ui"
import { Section } from "../lib/store.jsx"

const what = { cli: "CLI", mcp: "Agent", phone: "iPhone", mac: "Mac", browser: "Browser", schedule: "Schedule", app: "Another app", node: "The device", api: "A program on the device" }
const usd = (n) => "$" + (n < 1 ? n.toFixed(4) : n.toFixed(2))
const grey = (text) => <span className="text-muted-foreground">{text}</span>

function Activity() {
  const app = one.params.app
  const health = useCall("_health", { app })
  useEffect(() => {
    one.title("Activity")
  }, [])
  if (!app) return <Empty>This page isn’t here.</Empty>
  if (!health.data) return <div className="p-4"><Pending call={health} rows={4} /></div>

  const { insights, activity } = health.data
  const errors = insights?.errors || []
  const runs = activity || []
  const usage = insights?.usage || []
  const cost = insights?.cost || []
  if (!errors.length && !runs.length && !usage.length && !cost.length) return <Empty>Nothing yet.</Empty>

  return (
    <div className="flex flex-col gap-6 p-4">
      {errors.length > 0 && (
        <Section title="Errors">
          <List>
            {errors.slice(0, 20).map((e, i) => (
              <ListRow
                key={i}
                title={<span className="text-destructive">{e.error || "Failed"}</span>}
                sub={`${e.tool} · ${one.ago(e.last)}${e.user ? " · " + e.user : ""}`}
                trailing={grey(`${e.count}×`)}
              />
            ))}
          </List>
        </Section>
      )}
      {runs.length > 0 && (
        <Section title="Runs">
          <List>
            {runs.slice(0, 12).map((r, i) => (
              <ListRow
                key={i}
                title={<>{!r.ok && <span className="text-destructive">✕ </span>}{r.schedule}</>}
                sub={r.ok ? `${r.device || ""} · ${r.ms < 1000 ? r.ms + " ms" : (r.ms / 1000).toFixed(1) + " s"}` : <span className="text-destructive">{r.error || "Failed"}</span>}
                trailing={grey(one.ago(r.time))}
              />
            ))}
          </List>
        </Section>
      )}
      {usage.length > 0 && (
        <Section title="Used By">
          <List>
            {usage.slice(0, 20).map((u, i) => <ListRow key={i} title={u.user} sub={u.agent || what[u.client] || u.client} trailing={grey(one.ago(u.last))} />)}
          </List>
        </Section>
      )}
      {cost.length > 0 && (
        <Section title="AI Cost">
          <List>{cost.map((c) => <ListRow key={c.model} title={c.model} trailing={grey(usd(c.usd))} />)}</List>
        </Section>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Activity />)
