// The Add to… sheet, from an app's row or its page: a switch for each of the person's pages, dimmed with a few words of
// why where they can't use it; and for an app with a service, the node it runs on, chosen before adding it, and picked
// again to move it there.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, Icon, List, ListRow, Problem, Switch, ListSkeleton } from "@one/ui"
import { Failed, Section, said, useCall, useList } from "../lib/store.jsx"

function Add() {
  const list = useList()
  const [put, setPut] = useState({})
  const [error, setError] = useState("")
  const name = one.params.app
  const a = list.data?.apps.find((x) => x.name === name)
  const where = useCall("devices", { app: name })
  const [picked, setPicked] = useState(null)

  useEffect(() => {
    one.title(a?.title || name)
  }, [a?.title, name])

  if (!list.data) return <div className="p-4">{list.error ? <Failed error={list.error} retry={list.reload} /> : <ListSkeleton rows={3} />}</div>
  if (!a) return <Empty>This app isn't here. It may have been erased, or the link is old.</Empty>
  const { places, admin } = list.data
  const first = !a.added && !admin ? "Ask an admin to add it first" : null

  const w = where.data
  const node = picked || w?.runs_on.find((r) => r.name)?.name || w?.nodes[0]
  const pick = async (n) => {
    one.tap()
    setError("")
    setPicked(n)
    if (!a.installed.length) return
    try {
      await one.call("move", { app: name, node: n })
    } catch (err) {
      setError(said(err).title)
    }
    list.reload()
    where.reload()
  }

  const flip = async (p, on) => {
    one.tap()
    setError("")
    setPut((s) => ({ ...s, [p.id]: on }))
    try {
      await one.call(on ? "add" : "remove", { app: name, scope: p.scope, confirm: true, ...(on && node ? { node } : {}) })
    } catch (err) {
      setError(said(err).title)
    }
    list.reload()
    setPut((s) => ({ ...s, [p.id]: undefined }))
  }

  return (
    <div className="flex flex-col items-center gap-4 p-4">
      <AppIcon app={a} size={64} />
      <List>
        {places.map((p) => {
          const why = first || p.why
          return (
            <ListRow
              key={p.id}
              title={p.name}
              sub={why}
              className={why ? "opacity-60" : undefined}
              trailing={<Switch checked={put[p.id] ?? a.installed.includes(p.id)} disabled={!!why} aria-label={p.name} onCheckedChange={(on) => flip(p, on)} />}
            />
          )
        })}
      </List>
      {w && (w.nodes.length > 1 || (w.problem && w.nodes.length > 0)) && (
        <div className="w-full"><Section title="Runs On">
          <List>
            {w.nodes.map((n) => (
              <ListRow key={n} title={n === "cloudflare" ? "Cloudflare" : n} onClick={() => pick(n)} trailing={n === node && <Icon name="check" />} />
            ))}
          </List>
        </Section></div>
      )}
      {error && <Problem title={error} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Add />)
