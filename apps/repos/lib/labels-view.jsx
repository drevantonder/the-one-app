// A repo's labels: each with its colour, description and open issues; those who push to the repo make, change and
// delete them. A part of a repo's Settings.
import { useState } from "react"
import { Button, Grid, GridRow, Hint, Input, Pending, Problem, ResponsiveDialog, Section, Spinner, useCall } from "@one/ui"
import { LabelChip } from "./parts.jsx"
import { plural, said } from "./use.jsx"

const COLOURS = ["d73a4a", "0075ca", "a2eeef", "7057ff", "008672", "e4e669", "d876e3", "fbca04"]

function LabelDialog({ repo, l, done }) {
  const [name, setName] = useState(l?.name || "")
  const [description, setDescription] = useState(l?.description || "")
  const [color, setColor] = useState(l?.color || COLOURS[Math.floor(Math.random() * 8)])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const run = async (go) => {
    setBusy(true)
    setError("")
    try {
      await go()
      done(true)
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  const save = () =>
    name.trim() && run(() => one.call("set_label", { repo, name: l ? l.name : name.trim(), ...(l && name.trim() !== l.name ? { new_name: name.trim() } : {}), color, description: description.trim() }))
  const remove = async () => {
    if (!l) return
    const ok = await one.alert({
      title: `Delete ${l.name}?`,
      text: l.open_issues ? `It comes off the ${plural(l.open_issues, "open issue", "open issues")} that have it, and closed ones.` : "It comes off every issue that has it.",
      action: "Delete",
      danger: true,
    })
    if (ok) run(() => one.call("delete_label", { repo, name: l.name, confirm: true }))
  }
  return (
    <ResponsiveDialog
      open
      onOpenChange={(o) => !o && done(false)}
      title={l ? "Edit Label" : "New Label"}
      action={<Button size="sm" disabled={busy || !name.trim()} onClick={save}>{busy && <Spinner />}{l ? "Save" : "Create Label"}</Button>}
    >
      <div className="flex min-h-0 flex-col gap-3 overflow-y-auto p-4">
        <Input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Name" maxLength={50} autoFocus />
        <Input value={description} onChange={(e) => setDescription(e.target.value)} placeholder="Description (optional)" aria-label="Description" maxLength={100} />
        <div className="flex items-center gap-3">
          <input type="color" aria-label="Colour" value={`#${color}`} onChange={(e) => setColor(e.target.value.slice(1))} className="h-9 w-12 cursor-pointer rounded-md border border-input bg-transparent p-1" />
          <LabelChip l={{ name: name || "Label preview", color }} />
        </div>
        {error && <Problem title={error} />}
        {l && <Button variant="destructive" className="self-start" disabled={busy} onClick={remove}>Delete</Button>}
      </div>
    </ResponsiveDialog>
  )
}

export function Labels({ repo, push }) {
  const labels = useCall("labels", { repo })
  const [editing, setEditing] = useState(null)
  if (!labels.data) return <Section title="Labels"><Pending call={labels} rows={3} /></Section>
  const edit = (l) => setEditing({ l })
  return (
    <Section title="Labels" action={push && <Button variant="outline" size="sm" onClick={() => edit()}>New Label</Button>}>
      {labels.data.length ? (
        <Grid min="22rem" max={4} gap="0.25rem">
          {labels.data.map((l) => (
            <GridRow
              key={l.name}
              title={<LabelChip l={l} />}
              sub={l.description || undefined}
              trailing={l.open_issues ? <span className="text-xs text-muted-foreground">{l.open_issues.toLocaleString()}</span> : undefined}
              onClick={push ? () => edit(l) : undefined}
            />
          ))}
        </Grid>
      ) : (
        <Hint>No labels yet.</Hint>
      )}
      {editing && <LabelDialog repo={repo} l={editing.l} done={(did) => (setEditing(null), did && labels.reload())} />}
    </Section>
  )
}
