// Labs: features still being built, which an admin switches on for everyone (src/labs.rs): the core's first, then each
// app's and each service's under its name.
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/labs.html.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, List, ListSkeleton, Problem, Switch } from "@one/ui"

// Labs under each app's (or service's) name, in the order they come.
function groups(labs, kind) {
  const by = new Map()
  for (const l of labs.filter((l) => l[kind])) by.set(l[kind].name, [...(by.get(l[kind].name) || []), l])
  return [...by.values()]
}

function Labs() {
  const [labs, setLabs] = useState(null)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  // What a switch was just set to, until the node says it has it.
  const [asked, setAsked] = useState({})
  const latest = useRef(0)

  async function load() {
    const n = ++latest.current
    try {
      const list = await one.api("GET", "/v1/labs")
      if (n !== latest.current) return
      setLabs(list)
      setProblem(null)
    } catch (e) {
      if (n === latest.current) setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.largeTitle("Labs")
    one.on("changed", load)
    load()
  }, [])

  async function set(l, on) {
    setError("")
    setAsked((all) => ({ ...all, [l.setting]: on }))
    try {
      await one.api("POST", "/v1/settings", { name: l.setting, value: on ? "on" : null })
    } catch (e) {
      setError(one.plain(e).title)
    }
    await load()
    setAsked((all) => {
      const { [l.setting]: _, ...rest } = all
      return rest
    })
  }

  const row = (l) => <Lab key={l.setting} lab={l} on={asked[l.setting] ?? l.on} busy={l.setting in asked} set={(on) => set(l, on)} />
  const section = (title, list) => !!list.length && (
    <section className="flex flex-col gap-2">
      <h2 className="text-xl font-semibold tracking-tight">{title}</h2>
      {list.map((ls) => (
        <div key={ls[0].app?.name ?? ls[0].service?.name} className="flex flex-col gap-2">
          <h3 className="flex items-center gap-2 text-sm font-medium text-muted-foreground">
            <AppIcon app={ls[0].app || ls[0].service} size={18} />
            {(ls[0].app || ls[0].service).title}
          </h3>
          <List>{ls.map(row)}</List>
        </div>
      ))}
    </section>
  )

  let body
  if (!labs) {
    body = problem ? <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      : <ListSkeleton rows={3} />
  } else if (!labs.length) {
    body = <Empty>Nothing to try right now.</Empty>
  } else {
    const core = labs.filter((l) => !l.app && !l.service)
    body = (
      <>
        {!!core.length && <List>{core.map(row)}</List>}
        {section("Apps", groups(labs, "app"))}
        {section("Services", groups(labs, "service"))}
        {error && <Problem title={error} />}
      </>
    )
  }

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <h1 className="large-title text-3xl font-bold tracking-tight">Labs</h1>
      {body}
    </div>
  )
}

// A feature: its title, what it is, and a switch. It moves at once; one that couldn't change goes back when the
// node's list comes.
function Lab({ lab, on, busy, set }) {
  return (
    <label className="flex cursor-pointer items-center gap-3 px-3 py-3 text-sm">
      <span className="min-w-0 flex-1">
        <span className="block font-medium">{lab.title}</span>
        {lab.description && <span className="block text-muted-foreground">{lab.description}</span>}
      </span>
      <Switch checked={on} aria-busy={busy || undefined} onCheckedChange={set} />
    </label>
  )
}

createRoot(document.getElementById("root")).render(<Labs />)
