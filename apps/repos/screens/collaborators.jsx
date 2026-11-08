// Who pushes to a repo: its owner, a group's members, and the collaborators its owners added, whom they add and remove.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, Button, Icon, Input, List, ListRow, Problem, ResponsiveDialog } from "@one/ui"
import { RepoTabs } from "../lib/frame.jsx"
import { Frame, Pending, Section, said, useCall } from "../lib/use.jsx"

const P = one.params

const User = ({ p, trailing }) => (
  <ListRow leading={<Avatar name={p.name} size={32} />} title={p.name} sub={p.handle ? `@${p.handle}` : undefined} trailing={trailing} />
)

function Add({ d, done }) {
  const [query, setQuery] = useState("")
  const [error, setError] = useState("")
  const q = query.trim().toLowerCase()
  const found = d.users.filter((p) => !q || p.name.toLowerCase().includes(q) || p.handle.includes(q))
  const add = async (p) => {
    setError("")
    try {
      await one.call("add_collaborator", { repo: P.repo, user: p.handle, confirm: true })
      one.tap("success")
      done(true)
    } catch (err) {
      setError(said(err).title)
    }
  }
  return (
    <ResponsiveDialog open onOpenChange={(o) => !o && done(false)} title="Add Collaborator" tall action={<Button variant="secondary" size="sm" onClick={() => done(false)}>Cancel</Button>}>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">
        <div className="relative">
          <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground" />
          <Input className="pl-8" placeholder="Find users" aria-label="Find users" value={query} onChange={(e) => setQuery(e.target.value)} autoFocus />
        </div>
        {!d.users.length ? (
          <p className="text-sm text-muted-foreground">Everyone pushes to it already.</p>
        ) : found.length ? (
          <List>
            {found.map((p) => <ListRow key={p.handle} leading={<Avatar name={p.name} size={32} />} title={p.name} sub={`@${p.handle}`} trailing={<Icon name="plus-circle" className="size-5 text-muted-foreground" />} onClick={() => add(p)} />)}
          </List>
        ) : (
          <p className="text-sm text-muted-foreground">No one called “{query.trim()}”.</p>
        )}
        <p className="text-xs text-muted-foreground">They push to {d.repo} once you add them.</p>
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

function Collaborators() {
  const call = useCall("collaborators", { repo: P.repo })
  const issues = useCall("issues", { repo: P.repo, page: 0 })
  const [trouble, setTrouble] = useState("")
  const [adding, setAdding] = useState(false)
  useEffect(() => {
    one.title("Collaborators")
  }, [])
  const d = call.data
  if (!d) return <><RepoTabs repo={P.repo} on="settings" info={issues.data || undefined} /><Pending call={call} rows={3} /></>

  const remove = async (c) => {
    const ok = await one.alert({ title: `Remove ${c.name}?`, text: `They stop pushing to ${d.repo}.`, action: "Remove", danger: true })
    if (!ok) return
    try {
      await one.call("remove_collaborator", { repo: P.repo, user: c.handle, confirm: true })
      call.reload()
    } catch (err) {
      setTrouble(said(err).title)
    }
  }
  return (
    <>
      <RepoTabs repo={P.repo} on="settings" info={issues.data || undefined} />
      <div className="flex max-w-2xl flex-col gap-6">
        {trouble && <Problem title={trouble} />}
        <Section title="Owner"><List><User p={d.owner} /></List></Section>
        {d.group && (
          <Section title={`Members of ${d.owner.name}`}>
            <List>{d.members.map((m) => <User key={m.handle} p={m} trailing={<span className="text-xs text-muted-foreground">{m.role === "admin" ? "Admin" : "Member"}</span>} />)}</List>
            <p className="px-1 text-xs text-muted-foreground">Its admins own it, and its members push to it.</p>
          </Section>
        )}
        <Section title="Collaborators">
          {d.collaborators.length ? (
            <List>
              {d.collaborators.map((c) => (
                <User key={c.handle} p={c} trailing={d.yours ? <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" aria-label={`Remove ${c.name}`} onClick={() => remove(c)}>Remove</Button> : undefined} />
              ))}
            </List>
          ) : (
            <p className="px-1 text-sm text-muted-foreground">No one else pushes to it.</p>
          )}
          {d.yours && <Button variant="outline" className="self-start" onClick={() => setAdding(true)}><Icon name="plus" />Add Collaborator…</Button>}
        </Section>
      </div>
      {adding && <Add d={d} done={(did) => (setAdding(false), did && (setTrouble(""), call.reload()))} />}
    </>
  )
}

createRoot(document.getElementById("root")).render(<Frame><Collaborators /></Frame>)
