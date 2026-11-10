// The inbox and the board. What opens from either shows beside it where there's room, and in a sheet where there isn't.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Icon, Input, List, ListRow, Pending, Problem, ResponsiveDialog, SearchInput, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner, Split, Tabs, TabsList, TabsTrigger, Textarea, useCall, useWidth } from "@one/ui"
import { ItemView, STATUSES, ago, quiet, said, shown, times, useChanged } from "../lib/item.jsx"

const P = one.params || {}
const BOARD = STATUSES.filter((s) => s.value !== "dropped")
// Past this the column is history, not something to scan.
const DONE_SHOWN = 30

const line = (i) => [
  i.times != null ? times(i.times) : null,
  ago(i.last_seen ?? i.updated),
  ...i.tags.filter((t) => !quiet(t) && t !== "regression").map(shown),
].filter(Boolean).join(" · ")

const CameBack = ({ i }) => i.tags.includes("regression") && i.status === "inbox" && <span className="shrink-0 rounded-md bg-amber-500/15 px-1.5 py-0.5 text-xs font-medium text-amber-700 dark:text-amber-400">Came back</span>

function Filter({ title, icon, value, choices, set, wide }) {
  const now = choices.find((c) => c.value === value)
  const pick = async (e) => {
    const items = [{ title: `Any ${title.toLowerCase()}`, checked: !value }, ...choices.map((c) => ({ title: c.title, checked: c.value === value }))]
    const i = await one.menu(items, e.currentTarget)
    if (i != null) set(i === 0 ? "" : choices[i - 1].value)
  }
  if (!wide) return <Button variant={value ? "secondary" : "outline"} size="icon" onClick={pick} aria-label={value ? `${title}: ${now?.title || value}` : title}><Icon name={icon} /></Button>
  return (
    <Button variant="outline" onClick={pick} aria-label={title}>
      <span className="max-w-32 truncate">{value ? now?.title || value : title}</span>
      <Icon name="caret-down" className="size-3 text-muted-foreground" />
    </Button>
  )
}

function NewItem({ scopes, known, done }) {
  const [title, setTitle] = useState("")
  const [body, setBody] = useState("")
  const [tags, setTags] = useState("")
  const [scope, setScope] = useState(one.scope?.startsWith("group:") ? one.scope : "user")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const create = async () => {
    setBusy(true)
    setError("")
    try {
      const { id } = await one.call("add", { title, body, scope, tags: tags.split(",").map((t) => t.trim()).filter(Boolean) })
      one.tap("success")
      done(id)
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open onOpenChange={(o) => !o && done()} title="New Item" action={<Button size="sm" disabled={busy || !title.trim()} onClick={create}>{busy && <Spinner />}Add</Button>}>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">
        <Input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" aria-label="Title" autoFocus />
        <Textarea value={body} onChange={(e) => setBody(e.target.value)} placeholder="What's needed to start on it" aria-label="Description" rows={6} />
        <Input value={tags} onChange={(e) => setTags(e.target.value)} placeholder="Tags, with commas" aria-label="Tags" list="new-item-tags" />
        <datalist id="new-item-tags">{known.map((t) => <option key={t.tag} value={t.tag} />)}</datalist>
        <Select value={scope} onValueChange={setScope}>
          <SelectTrigger aria-label="Whose"><SelectValue /></SelectTrigger>
          <SelectContent>{scopes.map((s) => <SelectItem key={s.scope} value={s.scope}>{s.name}</SelectItem>)}</SelectContent>
        </Select>
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

function Row({ i, selected, open, status }) {
  return (
    <ListRow
      wrap
      title={i.title}
      sub={line(i)}
      trailing={status === "inbox" ? <CameBack i={i} /> : undefined}
      selected={selected}
      onClick={() => open(i)}
    />
  )
}

function Card({ i, selected, open, drag }) {
  return (
    <button
      type="button"
      draggable
      onDragStart={(e) => drag(e, i)}
      onClick={() => open(i)}
      data-selected={selected || undefined}
      className="flex w-full cursor-pointer flex-col gap-1 rounded-lg border bg-card p-3 text-left outline-none hover:bg-accent/50 focus-visible:bg-accent/50 data-[selected]:bg-accent"
    >
      <span className="line-clamp-3 text-sm font-medium [overflow-wrap:anywhere]">{i.title}</span>
      <span className="truncate text-xs text-muted-foreground">{line(i)}</span>
      <CameBack i={i} />
    </button>
  )
}

function Tracker() {
  const [ref, width] = useWidth()
  const wide = width >= 640
  const [view, setView] = useState("list")
  const [status, setStatus] = useState(P.status || "inbox")
  const [tag, setTag] = useState(P.tag || "")
  const [scope, setScope] = useState("")
  const [typed, setTyped] = useState("")
  const [q, setQ] = useState("")
  const [open, setOpen] = useState(P.item || null)
  const [adding, setAdding] = useState(false)
  const [moved, setMoved] = useState({})
  const [over, setOver] = useState(null)
  const [problem, setProblem] = useState(null)

  const input = { limit: view === "board" ? 500 : 200, ...(view === "list" ? { status } : {}), ...(tag ? { tags: [tag] } : {}), ...(q ? { q } : {}), ...(scope ? { scope } : {}) }
  const list = useCall("list", input)
  const known = useCall("tags", {})
  const d = list.data
  useChanged(() => { list.reload(); known.reload() })
  useEffect(() => {
    const t = setTimeout(() => setQ(typed.trim()), 250)
    return () => clearTimeout(t)
  }, [typed])

  const tags = known.data || []
  const statusOf = (i) => moved[i.id] ?? i.status
  const items = (d?.items || []).filter((i) => view === "board" || statusOf(i) === status)
  const move = (i, to) => {
    if (statusOf(i) === to) return
    setMoved((m) => ({ ...m, [i.id]: to }))
    one.call("set_status", { id: i.id, status: to }).then(
      () => list.reload().then(() => setMoved(({ [i.id]: _, ...rest }) => rest)),
      (err) => { setMoved(({ [i.id]: _, ...rest }) => rest); setProblem(said(err)) },
    )
  }
  const drag = (e, i) => e.dataTransfer.setData("text/plain", i.id)
  const drop = (e, to) => {
    e.preventDefault()
    setOver(null)
    const i = d?.items.find((x) => x.id === e.dataTransfer.getData("text/plain"))
    if (i) move(i, to)
  }
  const made = (id) => {
    setAdding(false)
    if (!id) return
    list.reload()
    known.reload()
    setOpen(id)
  }

  const beside = width >= 1000
  // Where there's room the newest is open from the start, so the page isn't half empty.
  const shownId = open ?? (beside && view === "list" ? items[0]?.id ?? null : null)
  const filtered = !!(tag || q || scope)
  const count = (s) => {
    const base = d?.counts?.[s] ?? 0
    const delta = (d?.items || []).reduce((n, i) => n + (statusOf(i) === s) - (i.status === s), 0)
    return Math.max(0, base + delta)
  }
  const none = filtered ? "Nothing matches" : status === "inbox" ? "Nothing in the Inbox" : `Nothing in ${STATUSES.find((s) => s.value === status).title}`

  const bar = (
    <div className="flex items-center gap-2">
      <SearchInput className="min-w-0 flex-1" placeholder="Search" value={typed} onChange={(e) => setTyped(e.target.value)} />
      <Filter title="Tag" icon="tag" value={tag} wide={wide} set={setTag} choices={tags.filter((t) => !quiet(t.tag)).map((t) => ({ value: t.tag, title: shown(t.tag) }))} />
      {(d?.scopes.length ?? 0) > 2 && <Filter title="Whose" icon="users" value={scope} wide={wide} set={setScope} choices={(d?.scopes || []).map((s) => ({ value: s.scope, title: s.name }))} />}
      <Button variant="ghost" size="icon" aria-label={view === "list" ? "Board" : "List"} onClick={() => setView(view === "list" ? "board" : "list")}><Icon name={view === "list" ? "kanban" : "list-bullets"} /></Button>
      <Button variant={shownId && beside ? "secondary" : "default"} onClick={() => setAdding(true)}>New</Button>
    </div>
  )

  const listing = (
    <div className="flex max-w-2xl flex-col gap-3">
      {bar}
      <div className="overflow-x-auto">
        <Tabs value={status} onValueChange={setStatus}>
          <TabsList>{STATUSES.map((s) => <TabsTrigger key={s.value} value={s.value}>{s.title}{count(s.value) > 0 && <span className="ml-1 text-muted-foreground tabular-nums">{count(s.value)}</span>}</TabsTrigger>)}</TabsList>
        </Tabs>
      </div>
      {problem && <Problem title={problem.title} text={problem.next} />}
      {!d ? <Pending call={list} rows={5} /> : items.length ? <List>{items.map((i) => <Row key={i.id} i={i} status={status} selected={shownId === i.id} open={(x) => setOpen(x.id)} />)}</List> : <Empty>{none}</Empty>}
    </div>
  )

  const board = (
    <div className="flex flex-col gap-3">
      {bar}
      {problem && <Problem title={problem.title} text={problem.next} />}
      {!d ? <Pending call={list} rows={5} /> : (
        <div className="-mx-5 flex snap-x gap-3 overflow-x-auto px-5 pb-2 @3xl:mx-0 @3xl:grid @3xl:grid-cols-4 @3xl:overflow-visible @3xl:px-0">
          {BOARD.map((c) => {
            const here = items.filter((i) => statusOf(i) === c.value)
            const cards = c.value === "done" ? here.slice(0, DONE_SHOWN) : here
            return (
              <div
                key={c.value}
                onDragOver={(e) => (e.preventDefault(), setOver(c.value))}
                onDragLeave={() => setOver(null)}
                onDrop={(e) => drop(e, c.value)}
                data-over={over === c.value || undefined}
                className="flex w-[78vw] max-w-72 shrink-0 snap-start flex-col gap-2 rounded-xl data-[over]:bg-accent/50 @3xl:w-auto @3xl:max-w-none"
              >
                <div className="px-1 text-sm font-medium text-muted-foreground">{c.title}{here.length > 0 && <span className="ml-1 tabular-nums">{here.length}</span>}</div>
                {cards.map((i) => <Card key={i.id} i={i} selected={shownId === i.id} open={(x) => setOpen(x.id)} drag={drag} />)}
              </div>
            )
          })}
        </div>
      )}
    </div>
  )

  const detail = shownId && <ItemView key={shownId} id={shownId} tags={tags} beside={beside} moved={moved[shownId]} onMove={move} />
  const openItem = (d?.items || []).find((i) => i.id === shownId)

  return (
    <div ref={ref} className="@container p-5">
      <Split wide={view === "board" ? 1500 : 1000} size={view === "board" ? "minmax(0, 60rem)" : "34rem"} list={view === "board" ? board : listing} detail={detail} title={openItem?.title || "Item"} onClose={() => setOpen(null)} />
      {adding && <NewItem scopes={d?.scopes || [{ scope: "user", name: "You" }]} known={tags} done={made} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Tracker />)
