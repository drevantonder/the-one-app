// An issue, as GitHub shows one: its title and state, its body and comments in order, a comment box with Close or
// Reopen, its assignees and labels (beside it where there's room, above it where there isn't), and for its repo's
// owners, Delete. It's the Issue screen, and what Issues shows beside its list where there's room.
import { useEffect, useState } from "react"
import { Avatar, Button, Card, Icon, Input, Markdown, ResponsiveDialog, Spinner, Problem, cn, useWidth } from "@one/ui"
import { LabelChip, MarkdownField, StateIcon } from "./parts.jsx"
import { Pending, ago, go, plural, said, useCall } from "./use.jsx"

const reason = (i) => (i.state_reason === "not_planned" ? "not planned" : "completed")

const Form = ({ children }) => <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">{children}</div>

function Comment({ d, c, body = false, editing, edit, done, menu, reload, fail }) {
  const [text, setText] = useState(c.body)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const update = async () => {
    setBusy(true)
    setError("")
    try {
      await one.call("edit_comment", { repo: d.repo, id: c.id, body: text })
      await reload()
      done()
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  const shown = !body && (c.yours || d.can.delete)
  return (
    <Card id={body ? undefined : `c-${c.id}`} className="gap-0 overflow-hidden py-0">
      <div className="flex items-center gap-2 border-b bg-muted/40 py-1.5 pr-1.5 pl-3 text-sm">
        <Avatar name={c.author.name} size={20} />
        <span className="min-w-0 flex-1 truncate">
          <strong className="font-semibold">{c.author.name}</strong> <span className="text-muted-foreground">{body ? "opened" : "commented"} {ago(c.created)}{!body && c.updated !== c.created ? " · edited" : ""}</span>
        </span>
        {shown && <Button variant="ghost" size="icon-sm" aria-label="More" onClick={(e) => menu(c, e.currentTarget)}><Icon name="dots-three" /></Button>}
      </div>
      <div className="px-4 py-3">
        {editing ? (
          <div className="flex flex-col gap-3">
            <MarkdownField value={text} onChange={setText} rows={5} label="Comment" />
            {error && <Problem title={error} />}
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={done}>Cancel</Button>
              <Button disabled={busy} onClick={update}>{busy && <Spinner />}Update Comment</Button>
            </div>
          </div>
        ) : c.body.trim() ? (
          <Markdown text={c.body} />
        ) : (
          <p className="text-sm text-muted-foreground italic">No description provided.</p>
        )}
      </div>
    </Card>
  )
}

const ClosedEvent = ({ d }) => (
  <div className="flex items-center gap-2 px-1 text-sm text-muted-foreground">
    <StateIcon issue={d} />
    <span><strong className="font-semibold text-foreground">{d.closed_by?.name || "Someone"}</strong> closed this as {reason(d)} {ago(d.closed)}</span>
  </div>
)

function Ticks({ title, items, chosen, save, empty, extra, done }) {
  const [on, setOn] = useState(new Set(chosen))
  const [error, setError] = useState("")
  const flip = async (key) => {
    const next = new Set(on)
    next.has(key) ? next.delete(key) : next.add(key)
    setOn(next)
    setError("")
    try {
      await save([...next])
    } catch (err) {
      setOn(on)
      setError(said(err).title)
    }
  }
  return (
    <ResponsiveDialog open onOpenChange={(o) => !o && done()} title={title} action={<Button variant="secondary" size="sm" onClick={done}>Done</Button>}>
      <Form>
        {items.length ? (
          <div className="divide-y overflow-hidden rounded-xl border">
            {items.map((x) => (
              <button key={x.key} type="button" onClick={() => flip(x.key)} className="flex w-full cursor-pointer items-center gap-3 px-3 py-2.5 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50">
                <Icon name="check" className={cn("shrink-0", !on.has(x.key) && "invisible")} />
                {x.node}
              </button>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">{empty}</p>
        )}
        {extra}
        {error && <Problem title={error} />}
      </Form>
    </ResponsiveDialog>
  )
}

function EditDialog({ d, act, done }) {
  const [title, setTitle] = useState(d.title)
  const [body, setBody] = useState(d.body)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const save = async () => {
    if (!title.trim()) return
    setBusy(true)
    setError("")
    try {
      await act("edit_issue", { title, body })
      done()
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog
      open
      onOpenChange={(o) => !o && done()}
      title={`Edit #${d.number}`}
      action={<><Button variant="ghost" size="sm" onClick={done}>Cancel</Button><Button size="sm" disabled={busy || !title.trim()} onClick={save}>{busy && <Spinner />}Save</Button></>}
    >
      <Form>
        <Input value={title} onChange={(e) => setTitle(e.target.value)} aria-label="Title" maxLength={256} autoFocus />
        <MarkdownField value={body} onChange={setBody} placeholder="Add a description" rows={10} label="Description" />
        {error && <Problem title={error} />}
      </Form>
    </ResponsiveDialog>
  )
}

export function IssueView({ repo, number, onGone }) {
  const issue = useCall("issue", { repo, number: Number(number) })
  const [room, width] = useWidth()
  const d = issue.data
  const [people, setPeople] = useState(null)
  const [draft, setDraft] = useState("")
  const [error, setError] = useState("")
  const [busy, setBusy] = useState("")
  const [dialog, setDialog] = useState(null)
  const [editing, setEditing] = useState(null)

  useEffect(() => {
    setPeople(null)
    setDraft("")
    setError("")
    setDialog(null)
    setEditing(null)
  }, [repo, number])

  if (!d) return <div ref={room}><Pending call={issue} rows={3} /></div>

  const act = async (tool, input) => {
    await one.call(tool, { repo, number: d.number, ...input })
    one.tap("success")
    await issue.reload()
  }
  const sending = async (key, tool, build) => {
    const text = draft
    setBusy(key)
    setError("")
    setDraft("")
    try {
      await act(tool, build(text))
    } catch (err) {
      setDraft(text)
      setError(said(err).title)
    }
    setBusy("")
  }
  const comment = () => draft.trim() && sending("comment", "comment", (text) => ({ body: text }))
  const close = async (e) => {
    const i = await one.menu([{ title: "Close as completed", icon: "check-circle" }, { title: "Close as not planned", icon: "prohibit" }], e.currentTarget)
    if (i != null) await sending("close", "close_issue", (text) => ({ state_reason: i ? "not_planned" : "completed", ...(text.trim() ? { comment: text } : {}) }))
  }
  const reopen = () => sending("reopen", "reopen_issue", (text) => (text.trim() ? { comment: text } : {}))

  const assign = async () => {
    try {
      setPeople(people || (await one.call("_people", { repo })))
      setDialog("assign")
    } catch (err) {
      setError(said(err).title)
    }
  }
  const label = async () => {
    try {
      setDialog({ labels: await one.call("labels", { repo }) })
    } catch (err) {
      setError(said(err).title)
    }
  }

  const more = async (c, button) => {
    const items = [...(c.yours ? [{ title: "Edit", icon: "pencil-simple" }] : []), { title: "Delete", icon: "trash", danger: true }]
    const i = await one.menu(items, button)
    if (i == null) return
    if (items[i].title === "Edit") return setEditing(c.id)
    if (!(await one.alert({ title: "Delete this comment?", text: "It can't be brought back.", action: "Delete", danger: true }))) return
    try {
      await one.call("delete_comment", { repo, id: c.id, confirm: true })
      await issue.reload()
    } catch (err) {
      setError(said(err).title)
    }
  }
  const remove = async () => {
    if (!(await one.alert({ title: `Delete #${d.number}?`, text: "It and its comments are gone for good.", action: "Delete", danger: true }))) return
    try {
      await one.call("delete_issue", { repo, number: d.number, confirm: true })
      onGone()
    } catch (err) {
      setError(said(err).title)
    }
  }

  const state = d.state === "open" ? "Open" : reason(d) === "not planned" ? "Closed as not planned" : "Closed"
  const timeline = []
  let shown = d.state === "open"
  for (const c of d.comments) {
    if (!shown && c.created > d.closed) (timeline.push(<ClosedEvent key="closed" d={d} />), (shown = true))
    timeline.push(<Comment key={c.id} d={d} c={c} editing={editing === c.id} edit={() => setEditing(c.id)} done={() => setEditing(null)} menu={more} reload={issue.reload} />)
  }
  if (!shown) timeline.push(<ClosedEvent key="closed" d={d} />)

  const triage = d.can.triage
  const side = (what, title, value, open) => {
    const Row = triage ? "button" : "div"
    return (
      <Row type={triage ? "button" : undefined} onClick={triage ? open : undefined} className={cn("flex w-full flex-col gap-1.5 px-3 py-2.5 text-left text-sm", triage && "cursor-pointer outline-none hover:bg-accent/50 focus-visible:bg-accent/50")}>
        <span className="flex items-center justify-between font-medium text-muted-foreground">{title}{triage && <Icon name="gear-six" />}</span>
        <span className="flex flex-wrap gap-1.5">{value}</span>
      </Row>
    )
  }
  const sidebar = (
    <div className="divide-y overflow-hidden rounded-xl border bg-card text-card-foreground">
      {side("assign", "Assignees", d.assignees.length ? d.assignees.map((a) => <span key={a.handle} className="flex items-center gap-1.5"><Avatar name={a.name} size={18} />{a.name}</span>) : <span className="text-muted-foreground">No one</span>, assign)}
      {side("label", "Labels", d.labels.length ? d.labels.map((l) => <LabelChip key={l.name} l={l} />) : <span className="text-muted-foreground">None yet</span>, label)}
    </div>
  )

  const withSide = width >= 640
  return (
    <div ref={room} className="flex min-w-0 flex-col gap-4">
      <div className="flex flex-col gap-2">
        <div className="flex items-start justify-between gap-3">
          <h1 className="text-xl font-semibold break-words">{d.title} <span className="font-normal text-muted-foreground">#{d.number}</span></h1>
          {d.can.edit && <Button variant="outline" size="sm" onClick={() => setDialog("edit")}>Edit</Button>}
        </div>
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm text-muted-foreground">
          <span className="inline-flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 font-medium text-foreground"><StateIcon issue={d} />{state}</span>
          <span><strong className="font-semibold text-foreground">{d.author.name}</strong> opened this {ago(d.created)} · {plural(d.comments.length, "comment", "comments")}</span>
        </div>
      </div>

      <div className={cn("grid items-start gap-4", withSide ? "grid-cols-[minmax(0,1fr)_15rem]" : "grid-cols-1")}>
        {!withSide && sidebar}
        <div className="flex min-w-0 flex-col gap-4">
          <Comment d={d} c={{ ...d, author: d.author }} body editing={false} />
          {timeline}
          <div className="flex flex-col gap-3">
            <MarkdownField value={draft} onChange={setDraft} placeholder="Add a comment" rows={4} />
            {error && <Problem title={error} />}
            <div className="flex flex-wrap justify-end gap-2">
              {d.can.close && (d.state === "open"
                ? <Button variant="outline" disabled={!!busy} onClick={close}><Icon name="check-circle" />{draft.trim() ? "Close with Comment" : "Close Issue"}</Button>
                : <Button variant="outline" disabled={!!busy} onClick={reopen}><Icon name="record" />Reopen Issue</Button>)}
              <Button disabled={!!busy || !draft.trim()} onClick={comment}>{busy === "comment" && <Spinner />}Comment</Button>
            </div>
          </div>
          {d.can.delete && <Button variant="destructive" className="self-start" onClick={remove}>Delete Issue</Button>}
        </div>
        {withSide && <div className="sticky top-4">{sidebar}</div>}
      </div>

      {dialog === "edit" && <EditDialog d={d} act={act} done={() => setDialog(null)} />}
      {dialog === "assign" && people && (
        <Ticks
          title="Assignees"
          items={people.map((p) => ({ key: p.handle, node: <><Avatar name={p.name} size={28} /><span className="min-w-0"><span className="block truncate font-medium">{p.name}</span><span className="block truncate text-muted-foreground">@{p.handle}</span></span></> }))}
          chosen={d.assignees.map((a) => a.handle)}
          save={(keys) => one.call("edit_issue", { repo, number: d.number, assignees: keys })}
          empty="No one to assign."
          done={() => (setDialog(null), issue.reload())}
        />
      )}
      {dialog?.labels && (
        <Ticks
          title="Labels"
          items={dialog.labels.map((l) => ({ key: l.name, node: <><span className="size-3.5 shrink-0 rounded-full" style={{ background: `#${l.color}` }} /><span className="min-w-0"><span className="block truncate font-medium">{l.name}</span>{l.description && <span className="block truncate text-muted-foreground">{l.description}</span>}</span></> }))}
          chosen={d.labels.map((l) => l.name)}
          save={(keys) => one.call("edit_issue", { repo, number: d.number, labels: keys })}
          empty="This repo has no labels."
          extra={<Button variant="outline" className="self-start" onClick={() => go("labels", { repo })}>Edit Labels…</Button>}
          done={() => (setDialog(null), issue.reload())}
        />
      )}
    </div>
  )
}
