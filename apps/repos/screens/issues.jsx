// A repo's issues, as GitHub lists them: Open and Closed, filters, and New Issue; an issue opens beside the list where
// there's room.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, Button, Empty, Icon, Input, List, ListRow, Problem, ResponsiveDialog, Spinner, Tabs, TabsList, TabsTrigger, Pending, Swap, useCall } from "@one/ui"
import { RepoTabs } from "../lib/frame.jsx"
import { IssueView } from "../lib/issue-view.jsx"
import { LabelChip, MarkdownField, StateIcon } from "../lib/parts.jsx"
import { Beside, Frame, ago, go, plural, said, useBeside } from "../lib/use.jsx"

const P = one.params

function NewIssue({ done }) {
  const [title, setTitle] = useState("")
  const [body, setBody] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const create = async () => {
    if (!title.trim()) return
    setBusy(true)
    setError("")
    try {
      const i = await one.call("open_issue", { repo: P.repo, title, body })
      one.tap("success")
      done(i.number)
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog
      open
      onOpenChange={(o) => !o && done()}
      title="New Issue"
      action={<><Button variant="ghost" size="sm" onClick={() => done()}>Cancel</Button><Button size="sm" disabled={busy || !title.trim()} onClick={create}>{busy && <Spinner />}Create</Button></>}
    >
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">
        <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" aria-label="Title" maxLength={256} autoFocus />
        <MarkdownField value={body} onChange={setBody} placeholder="Add a description" rows={8} label="Description" />
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

function Row({ i, selected, open }) {
  const by = `#${i.number} · ${i.author.name} · ${i.state === "open" ? ago(i.created) : `closed ${ago(i.closed)}`}`
  return (
    <ListRow
      leading={<StateIcon issue={i} className="size-5" />}
      wrap
      title={<span className="flex flex-wrap items-center gap-x-2 gap-y-1"><span className="min-w-0">{i.title}</span>{i.labels.map((l) => <LabelChip key={l.name} l={l} />)}</span>}
      sub={by}
      trailing={
        <span className="flex shrink-0 items-center gap-2 text-muted-foreground">
          {i.assignees.length > 0 && <span className="flex -space-x-1.5" title={i.assignees.map((a) => a.name).join(", ")}>{i.assignees.map((a) => <Avatar key={a.handle} name={a.name} size={20} className="ring-2 ring-card" />)}</span>}
          {i.comments > 0 && <span className="flex items-center gap-1 text-xs" aria-label={plural(i.comments, "comment", "comments")}><Icon name="chat-circle" />{i.comments}</span>}
        </span>
      }
      selected={selected}
      onClick={() => open(i)}
    />
  )
}

function Filter({ title, value, choices, set, also }) {
  const now = choices.find((c) => c.value === value)
  const pick = async (e) => {
    const items = [{ title: `Any ${title.toLowerCase()}`, checked: !value }, ...choices.map((c) => ({ title: c.title, checked: c.value === value })), ...(also ? [{ title: also.title, section: true }] : [])]
    const i = await one.menu(items, e.currentTarget)
    if (i == null) return
    if (also && i === items.length - 1) return also.run()
    set(i === 0 ? "" : choices[i - 1].value)
  }
  return (
    <Button variant="outline" size="sm" onClick={pick} aria-label={title}>
      <span className="max-w-32 truncate">{value ? now?.title || value : title}</span>
      <Icon name="caret-down" className="size-3 text-muted-foreground" />
    </Button>
  )
}

function Issues() {
  const beside = useBeside()
  const [f, setF] = useState({ state: P.state || "open", label: P.label || "", assignee: P.assignee || "", author: P.author || "", q: "" })
  const [page, setPage] = useState(0)
  const [typed, setTyped] = useState("")
  const [shown, setShown] = useState(null)
  const [creating, setCreating] = useState(false)
  const input = { repo: P.repo, state: f.state, page }
  for (const k of ["label", "assignee", "author", "q"]) if (f[k]) input[k] = f[k]
  const list = useCall("issues", input)
  const labels = useCall("labels", { repo: P.repo })
  const users = useCall("_users", { repo: P.repo })
  const d = list.data
  const filtered = !!(f.label || f.assignee || f.author || f.q)
  // The tab's count is the repo's open issues, whatever the filters.
  const tab = useRef(null)
  if (d && !filtered) tab.current = { open_issues: d.open_issues, yours: d.yours }

  useEffect(() => {
    one.title("Issues")
  }, [])
  useEffect(() => {
    const t = setTimeout(() => {
      if (typed.trim() === f.q) return
      setF((x) => ({ ...x, q: typed.trim() }))
      setPage(0)
    }, 250)
    return () => clearTimeout(t)
  }, [typed])

  const set = (key, value) => (setF((x) => ({ ...x, [key]: value })), setPage(0))
  const open = (i) => (beside ? setShown(i.number) : go("issue", { repo: P.repo, number: i.number }))
  const made = (number) => {
    setCreating(false)
    if (!number) return
    list.reload()
    if (beside) setShown(number)
    else go("issue", { repo: P.repo, number })
  }

  if (!d) return <><RepoTabs repo={P.repo} on="issues" /><Pending call={list} rows={4} /></>

  const tabs = <RepoTabs repo={P.repo} on="issues" info={tab.current || d} />
  if (!d.open_issues && !d.closed_issues && !filtered) {
    return (
      <>
        {tabs}
        <Empty action={{ label: "New Issue", run: () => setCreating(true) }}>No issues yet.</Empty>
        {creating && <NewIssue done={made} />}
      </>
    )
  }
  const names = (users.data || []).map((p) => ({ value: p.handle, title: p.name }))
  // Where there's room the newest is open from the start, so the page isn't half empty; the one that's open stays
  // while the list it's in does.
  const current = beside ? (d.issues.some((i) => i.number === shown) ? shown : d.issues[0]?.number) : null
  const none = filtered ? "No issues match." : f.state === "open" ? "No open issues." : "No closed issues."
  const left = (
    <div className="flex max-w-2xl flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <Tabs value={f.state} onValueChange={(v) => set("state", v)}>
          <TabsList>
            <TabsTrigger value="open"><Icon name="record" />{d.open_issues.toLocaleString()} Open</TabsTrigger>
            <TabsTrigger value="closed"><Icon name="check" />{d.closed_issues.toLocaleString()} Closed</TabsTrigger>
          </TabsList>
        </Tabs>
        <Button variant={beside ? "secondary" : "default"} onClick={() => setCreating(true)}>New Issue</Button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-48 flex-1">
          <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground" />
          <Input className="pl-8" placeholder="Find an issue" aria-label="Find an issue" value={typed} onChange={(e) => setTyped(e.target.value)} />
        </div>
        <Filter title="Author" value={f.author} choices={names} set={(v) => set("author", v)} />
        <Filter title="Label" value={f.label} choices={(labels.data || []).map((l) => ({ value: l.name, title: l.name }))} set={(v) => set("label", v)} also={{ title: "Edit Labels…", run: () => go("labels", { repo: P.repo }) }} />
        <Filter title="Assignee" value={f.assignee} choices={[{ value: "none", title: "Assigned to no one" }, ...names]} set={(v) => set("assignee", v)} />
      </div>
      {list.error && !list.moving && <Problem title={said(list.error).title} />}
      <Swap call={list} rows={4}>
        {(now) => (now.issues.length ? <List>{now.issues.map((i) => <Row key={i.number} i={i} selected={current === i.number} open={open} />)}</List> : <Empty>{none}</Empty>)}
      </Swap>
      {(page > 0 || d.more) && (
        <div className="flex gap-2">
          <Button variant="outline" disabled={!page} onClick={() => setPage(page - 1)}>Previous</Button>
          <Button variant="outline" disabled={!d.more} onClick={() => setPage(page + 1)}>Next</Button>
        </div>
      )}
    </div>
  )
  return (
    <>
      {tabs}
      <Beside size="34rem" list={left} detail={current && <IssueView repo={P.repo} number={current} onGone={() => (setShown(null), list.reload())} />} />
      {creating && <NewIssue done={made} />}
    </>
  )
}

createRoot(document.getElementById("root")).render(<Frame><Issues /></Frame>)
