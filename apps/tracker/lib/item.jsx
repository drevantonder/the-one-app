// An item, in the screen's side pane or its sheet: its status and tags, then for a failure what broke and how to look
// into it, then what people said.
import { useEffect, useRef, useState } from "react"
import { Badge, Button, CopyButton, Hint, Icon, Input, Markdown, Pending, Problem, Section, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Spinner, Textarea, fileSize, useCall } from "@one/ui"

export const STATUSES = [
  { value: "inbox", title: "Inbox" },
  { value: "next", title: "Next" },
  { value: "doing", title: "Doing" },
  { value: "done", title: "Done" },
  { value: "dropped", title: "Dropped" },
]

export const said = (error) => one.plain(error)
export const ago = (ms) => one.ago(ms, { long: true })
export const times = (n) => (n === 1 ? "once" : `${n.toLocaleString()} times`)
/// A tag as a person reads it: an app's is just its name, a build's its first characters.
export const shown = (tag) => (tag.startsWith("app:") ? tag.slice(4) : tag.startsWith("build:") ? `build ${tag.slice(6, 13)}` : tag)
export const quiet = (tag) => tag.startsWith("fingerprint:") || tag.startsWith("build:") || tag === "error"
export const shortBuild = (b) => String(b || "").slice(0, 7)

/// Runs `fn` each time something changes, while this is on screen: one.on has no way to stop listening.
export function useChanged(fn) {
  const latest = useRef(fn)
  latest.current = fn
  useEffect(() => {
    let live = true
    one.on("changed", () => live && latest.current())
    return () => { live = false }
  }, [])
}

const toBase64 = (file) =>
  new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result).split(",")[1] || "")
    reader.onerror = () => reject(reader.error)
    reader.readAsDataURL(file)
  })

function Failure({ e }) {
  return (
    <Section title="What broke">
      <div>
        <div className="text-3xl font-semibold tabular-nums">{times(e.times)}</div>
        <Hint>
          Last {ago(e.last_seen)}, first {ago(e.first_seen)}. Since build {shortBuild(e.first_build)}{e.last_build !== e.first_build ? `, now ${shortBuild(e.last_build)}` : ""}.
        </Hint>
      </div>
      {e.message && <pre className="overflow-x-auto rounded-lg bg-muted p-3 text-xs break-words whitespace-pre-wrap">{e.message}</pre>}
      <Hint>{e.type} at {e.frame}, in {e.app} {e.tool}.</Hint>
      {e.dig.map((line) => (
        <div key={line} className="flex min-w-0 items-center gap-1">
          <code className="min-w-0 flex-1 truncate font-mono text-xs">{line}</code>
          <CopyButton text={line} />
        </div>
      ))}
    </Section>
  )
}

function Comment({ c }) {
  return (
    <div className="flex flex-col gap-1">
      <div className="text-xs text-muted-foreground">{c.by}, {ago(c.at)}</div>
      <Markdown className="text-sm">{c.body}</Markdown>
    </div>
  )
}

function AddTag({ item, tags, act }) {
  const [adding, setAdding] = useState(false)
  const [text, setText] = useState("")
  const add = () => {
    const tag = text.trim().toLowerCase().replace(/\s+/g, "-")
    setAdding(false)
    setText("")
    if (tag && !item.tags.includes(tag)) act("tag", { id: item.id, add: [tag] })
  }
  if (!adding) return <Button variant="ghost" size="sm" onClick={() => setAdding(true)}><Icon name="plus" />Tag</Button>
  return (
    <>
      <Input
        autoFocus
        aria-label="Tag"
        className="h-8 w-40"
        list="tracker-tags"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => (e.key === "Enter" ? add() : e.key === "Escape" && setAdding(false))}
        onBlur={add}
      />
      <datalist id="tracker-tags">{tags.map((t) => <option key={t.tag} value={t.tag} />)}</datalist>
    </>
  )
}

export function ItemView({ id, tags, moved, onMove, beside }) {
  const call = useCall("read", { id })
  const [problem, setProblem] = useState(null)
  const [note, setNote] = useState("")
  const [sending, setSending] = useState(false)
  const [attaching, setAttaching] = useState(false)
  const picker = useRef(null)
  useChanged(call.reload)

  const i = call.data
  if (!i) return <Pending call={call} rows={5} />

  const fail = (err) => setProblem(said(err))
  const act = (tool, input) => {
    setProblem(null)
    return one.call(tool, input).then(call.reload, fail)
  }
  const send = async () => {
    if (!note.trim()) return
    setSending(true)
    await one.call("comment", { id, body: note }).then(() => (setNote(""), call.reload()), fail)
    setSending(false)
  }
  const attach = async (file) => {
    if (!file) return
    setAttaching(true)
    await toBase64(file).then((base64) => one.call("attach", { id, name: file.name, type: file.type || undefined, base64 })).then(call.reload, fail)
    setAttaching(false)
  }
  const open = (a) => one.call("attachment", { id: a.id }).then(one.preview, fail)
  const status = moved ?? i.status

  return (
    <div className="flex min-w-0 flex-col gap-5">
      <div className="flex min-w-0 flex-col gap-2">
        <h2 className="text-lg leading-snug font-semibold [overflow-wrap:anywhere]">{i.title}</h2>
        <Hint>{i.by}, {ago(i.at)}, in {i.scope_name}</Hint>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Select value={status} onValueChange={(v) => onMove(i, v)}>
          <SelectTrigger aria-label="Status" size="sm" className="w-28"><SelectValue /></SelectTrigger>
          <SelectContent>{STATUSES.map((s) => <SelectItem key={s.value} value={s.value}>{s.title}</SelectItem>)}</SelectContent>
        </Select>
        {i.tags.filter((t) => !t.startsWith("fingerprint:")).map((t) => (
          <Badge key={t} variant="secondary" className="gap-1">
            {shown(t)}
            <button type="button" aria-label={`Remove ${t}`} className="cursor-pointer text-muted-foreground" onClick={() => act("tag", { id, remove: [t] })}><Icon name="x" className="size-3" /></button>
          </Badge>
        ))}
        <AddTag item={i} tags={tags} act={act} />
      </div>
      {problem && <Problem title={problem.title} text={problem.next} />}
      {i.error && <Failure e={i.error} />}
      {i.body && <Markdown className="text-sm">{i.body}</Markdown>}
      {i.attachments.length > 0 && (
        <Section title="Attachments">
          {i.attachments.map((a) => (
            <button key={a.id} type="button" className="flex min-w-0 cursor-pointer items-center gap-2 text-left text-sm hover:underline" onClick={() => open(a)}>
              <Icon name="paperclip" className="size-4 shrink-0 text-muted-foreground" />
              <span className="min-w-0 flex-1 truncate">{a.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">{fileSize(a.size)}</span>
            </button>
          ))}
        </Section>
      )}
      {i.comments.length > 0 && <div className="flex flex-col gap-4">{i.comments.map((c) => <Comment key={c.id} c={c} />)}</div>}
      <div className="flex flex-col gap-2">
        <Textarea aria-label="Comment" placeholder="Comment" rows={2} value={note} onChange={(e) => setNote(e.target.value)} />
        <div className="flex items-center justify-between gap-2">
          <Button variant="ghost" size="sm" disabled={attaching} onClick={() => picker.current.click()}>{attaching ? <Spinner /> : <Icon name="paperclip" />}Attach</Button>
          <input ref={picker} type="file" className="hidden" onChange={(e) => (attach(e.target.files[0]), (e.target.value = ""))} />
          <Button size="sm" variant={beside ? "default" : "secondary"} disabled={sending || !note.trim()} onClick={send}>{sending && <Spinner />}Comment</Button>
        </div>
      </div>
    </div>
  )
}
