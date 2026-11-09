// A repo's labels, as GitHub's Labels page: each with its colour, description and open issues; those who push to the
// repo make, change and delete them.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Input, List, ListRow, Problem, ResponsiveDialog, Spinner, Pending, useCall } from "@one/ui"
import { RepoTabs } from "../lib/frame.jsx"
import { LabelChip } from "../lib/parts.jsx"
import { Frame, plural, said } from "../lib/use.jsx"

const P = one.params
const COLOURS = ["d73a4a", "0075ca", "a2eeef", "7057ff", "008672", "e4e669", "d876e3", "fbca04"]

function LabelDialog({ l, done }) {
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
    name.trim() && run(() => one.call("set_label", { repo: P.repo, name: l ? l.name : name.trim(), ...(l && name.trim() !== l.name ? { new_name: name.trim() } : {}), color, description: description.trim() }))
  const remove = async () => {
    if (!l) return
    const ok = await one.alert({
      title: `Delete ${l.name}?`,
      text: l.open_issues ? `It comes off the ${plural(l.open_issues, "open issue", "open issues")} that have it, and closed ones.` : "It comes off every issue that has it.",
      action: "Delete",
      danger: true,
    })
    if (ok) run(() => one.call("delete_label", { repo: P.repo, name: l.name, confirm: true }))
  }
  return (
    <ResponsiveDialog
      open
      onOpenChange={(o) => !o && done(false)}
      title={l ? "Edit Label" : "New Label"}
      action={<><Button variant="ghost" size="sm" onClick={() => done(false)}>Cancel</Button><Button size="sm" disabled={busy || !name.trim()} onClick={save}>{busy && <Spinner />}{l ? "Save" : "Create Label"}</Button></>}
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

function Labels() {
  const labels = useCall("labels", { repo: P.repo })
  // Whether the caller pushes to the repo, which decides who manages its labels, comes with its issues.
  const issues = useCall("issues", { repo: P.repo, page: 0 })
  const [editing, setEditing] = useState(null)
  useEffect(() => {
    one.title("Labels")
  }, [])

  if (!labels.data || !issues.data) return <><RepoTabs repo={P.repo} on="issues" /><Pending call={labels.error ? labels : issues.error ? issues : labels} rows={3} /></>
  const push = issues.data.push
  const edit = (l) => setEditing({ l })
  const changed = (did) => (setEditing(null), did && labels.reload())
  return (
    <>
      <RepoTabs repo={P.repo} on="issues" info={issues.data} />
      <div className="flex max-w-2xl flex-col gap-4">
        {labels.data.length ? (
          <>
            {push && <div className="flex justify-end"><Button onClick={() => edit()}>New Label</Button></div>}
            <List>
              {labels.data.map((l) => (
                <ListRow
                  key={l.name}
                  title={<LabelChip l={l} />}
                  sub={l.description || undefined}
                  trailing={l.open_issues ? <span className="text-xs text-muted-foreground">{plural(l.open_issues, "open issue", "open issues")}</span> : undefined}
                  onClick={push ? () => edit(l) : undefined}
                />
              ))}
            </List>
          </>
        ) : (
          <Empty action={push ? { label: "New Label", run: () => edit() } : undefined}>No labels yet.</Empty>
        )}
      </div>
      {editing && <LabelDialog l={editing.l} done={changed} />}
    </>
  )
}

createRoot(document.getElementById("root")).render(<Frame><Labels /></Frame>)
