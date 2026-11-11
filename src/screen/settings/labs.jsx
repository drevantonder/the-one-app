// Labs: features still being built, which an admin switches on for everyone (labs/): the core's first, then each
// app's and each service's under its name.
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/labs.html.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, Hint, LargeTitle, List, ListSkeleton, LoadFailure, Problem, Section, Switch } from "@one/ui"

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
      const list = await one.api("GET", "/api/labs")
      if (n !== latest.current) return
      setLabs(list)
      setProblem(null)
    } catch (e) {
      if (n === latest.current) setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.on("changed", load)
    load()
  }, [])

  async function set(l, on) {
    setError("")
    setAsked((all) => ({ ...all, [l.setting]: on }))
    try {
      await one.api("POST", "/api/settings", { name: l.setting, value: on ? "on" : null })
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
  const group = (ls) => (
    <Section key={ls[0].app?.name ?? ls[0].service?.name} title={<><AppIcon app={ls[0].app || ls[0].service} size={18} />{(ls[0].app || ls[0].service).title}</>}>
      <List>{ls.map(row)}</List>
    </Section>
  )

  let body
  if (!labs) {
    body = problem ? <LoadFailure problem={problem} retry={load} />
      : <ListSkeleton rows={3} />
  } else if (!labs.length) {
    body = <Empty>Nothing to try right now.</Empty>
  } else {
    const core = labs.filter((l) => !l.app && !l.service)
    body = (
      <>
        {!!core.length && <List>{core.map(row)}</List>}
        {[...groups(labs, "app"), ...groups(labs, "service")].map(group)}
        {error && <Problem title={error} />}
      </>
    )
  }

  return (
    <div className="flex max-w-2xl flex-col gap-6">
      <div className="flex flex-col gap-2">
        <LargeTitle>Labs</LargeTitle>
        <Hint>What you turn on here is on for everyone.</Hint>
      </div>
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
