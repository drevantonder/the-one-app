// React and @one/ui come from the page; Tailwind's classes, with shadcn's theme, work in className.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, List, ListRow, Problem } from "@one/ui"

// Today where the user is. The app's own `ctx.now()` is UTC, so a screen says which day it means.
const day = () => new Date().toLocaleDateString("en-CA")
const time = (at) => new Date(at).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })

function Main() {
  const [marks, setMarks] = useState(null)
  const list = marks ?? []
  const [problem, setProblem] = useState(null)

  const load = () => one.call("today", { day: day() }).then((d) => { setMarks(d.marks); setProblem(null) }, (e) => setProblem(one.plain(e)))
  useEffect(() => { load(); one.on("changed", load) }, [])

  // Show the result at once, and put it back if the call fails.
  const act = (next, call) => {
    const before = marks
    setMarks(next)
    call.then(load, (e) => { setMarks(before); setProblem(one.plain(e)) })
  }
  const add = () => act([{ id: -Date.now(), at: Date.now() }, ...marks], one.call("add", { day: day() }))
  const remove = (m) => act(marks.filter((x) => x !== m), one.call("remove", { id: m.id }))

  return (
    <div className="@container flex flex-col gap-6 p-5">
      <div className="flex items-end justify-between gap-4">
        <div>
          <div className="text-5xl font-semibold tabular-nums">{marks ? list.length : "–"}</div>
          <div className="text-sm text-muted-foreground">today</div>
        </div>
        <Button disabled={!marks} onClick={add}>Add One</Button>
      </div>
      {problem && <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />}
      {marks && list.length === 0 && <Empty>Nothing yet today</Empty>}
      {list.length > 0 && (
        <List>
          {list.map((m) => (
            <ListRow key={m.id} title={time(m.at)} trailing={<Button size="sm" variant="ghost" onClick={() => remove(m)}>Remove</Button>} />
          ))}
        </List>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
