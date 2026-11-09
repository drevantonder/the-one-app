// Who pushes to a repo: its owner, a group's members, and the collaborators its owners add and remove. A part of a
// repo's Settings.
import { useState } from "react"
import { Avatar, Button, Grid, GridRow, Hint, Icon, List, ListRow, Pending, Problem, ResponsiveDialog, SearchInput, Section, useCall } from "@one/ui"
import { said } from "./use.jsx"

function Add({ d, repo, done }) {
  const [query, setQuery] = useState("")
  const [error, setError] = useState("")
  const q = query.trim().toLowerCase()
  const found = d.users.filter((p) => !q || p.name.toLowerCase().includes(q) || p.handle.includes(q))
  const add = async (p) => {
    setError("")
    try {
      await one.call("add_collaborator", { repo, user: p.handle, confirm: true })
      one.tap("success")
      done(true)
    } catch (err) {
      setError(said(err).title)
    }
  }
  return (
    <ResponsiveDialog open onOpenChange={(o) => !o && done(false)} title="Add Collaborator" action={<Button variant="ghost" size="sm" onClick={() => done(false)}>Cancel</Button>}>
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">
        <SearchInput placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)} autoFocus />
        {!d.users.length ? (
          <Hint>Everyone pushes to it already.</Hint>
        ) : found.length ? (
          <List>
            {found.map((p) => <ListRow key={p.handle} leading={<Avatar name={p.name} size={32} />} title={p.name} sub={`@${p.handle}`} trailing={<Icon name="plus-circle" className="size-5 text-muted-foreground" />} onClick={() => add(p)} />)}
          </List>
        ) : (
          <Hint>No one called “{query.trim()}”.</Hint>
        )}
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

const Person = ({ p, role, trailing }) => (
  <GridRow leading={<Avatar name={p.name} size={32} />} title={p.name} sub={p.handle ? `@${p.handle}` : undefined} trailing={trailing || (role && <span className="text-xs text-muted-foreground">{role}</span>)} />
)

export function People({ repo }) {
  const call = useCall("collaborators", { repo })
  const [trouble, setTrouble] = useState("")
  const [adding, setAdding] = useState(false)
  const d = call.data
  if (!d) return <Section title="Collaborators"><Pending call={call} rows={2} /></Section>
  const remove = async (c) => {
    const ok = await one.alert({ title: `Remove ${c.name}?`, text: `They stop pushing to ${d.repo}.`, action: "Remove", danger: true })
    if (!ok) return
    try {
      await one.call("remove_collaborator", { repo, user: c.handle, confirm: true })
      call.reload()
    } catch (err) {
      setTrouble(said(err).title)
    }
  }
  return (
    <Section title="Collaborators" action={d.yours && <Button variant="outline" size="sm" onClick={() => setAdding(true)}>Add Collaborator</Button>}>
      {trouble && <Problem title={trouble} />}
      <Grid min="22rem" max={4} gap="0.25rem">
        <Person p={d.owner} role="Owner" />
        {d.group && d.members.map((m) => <Person key={m.handle} p={m} role={m.role === "admin" ? "Admin" : "Member"} />)}
        {d.collaborators.map((c) => <Person key={c.handle} p={c} trailing={d.yours ? <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" aria-label={`Remove ${c.name}`} onClick={() => remove(c)}>Remove</Button> : undefined} />)}
      </Grid>
      {adding && <Add d={d} repo={repo} done={(did) => (setAdding(false), did && (setTrouble(""), call.reload()))} />}
    </Section>
  )
}
