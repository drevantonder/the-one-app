// The Add to… sheet, from an app's row or its page: a switch for each of the person's pages, dimmed with a few words of
// why where they can't use it.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, List, ListRow, Problem, Switch } from "@one/ui"
import { Failed, Waiting, said, useList } from "../lib/store.jsx"

function Add() {
  const list = useList()
  // The switch stays where it was put, and goes back only if the change fails.
  const [put, setPut] = useState({})
  const [error, setError] = useState("")
  const name = one.params.app
  const a = list.data?.apps.find((x) => x.name === name)

  useEffect(() => {
    one.title(a?.title || name)
  }, [a?.title, name])

  if (!list.data) return <div className="p-4">{list.error ? <Failed error={list.error} retry={list.reload} /> : <Waiting rows={3} />}</div>
  if (!a) return <Empty>This app isn't here. It may have been erased, or the link is old.</Empty>
  const { places, admin } = list.data
  const first = !a.added && !admin ? "Ask an admin to add it first" : null

  const flip = async (p, on) => {
    one.tap()
    setError("")
    setPut((s) => ({ ...s, [p.id]: on }))
    try {
      await one.call(on ? "add" : "remove", { app: name, scope: p.scope, confirm: true })
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
      {error && <Problem title={error} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Add />)
