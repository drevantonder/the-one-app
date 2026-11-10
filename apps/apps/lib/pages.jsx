// An app's pages: the one control wherever an app can be added, Add to while it's on none of the person's pages and
// Manage once it's on one, and the sheet it opens, a switch for each page they may change.
import { useRef, useState } from "react"
import { Button, Hint, List, ListRow, Problem, ResponsiveDialog, Switch, useDialog } from "@one/ui"
import { onAPage, said } from "./store.jsx"

function Pages({ name, store }) {
  const [error, setError] = useState("")
  const [note, setNote] = useState("")
  const { apps, places } = store.data
  const a = apps.find((x) => x.name === name)
  const flip = (place, on) => {
    setError("")
    setNote("")
    store.put(a, place, on).then(setNote).catch((err) => setError(said(err).title))
  }
  return (
    <div className="flex min-h-0 flex-col gap-4 overflow-y-auto p-4">
      <p className="text-sm text-muted-foreground">Turning one off keeps its data.</p>
      <List>
        {places.filter((p) => p.may).map((p) => (
          <ListRow key={p.id} title={p.name} trailing={<Switch checked={a.added.includes(p.id)} aria-label={p.name} onCheckedChange={(on) => flip(p, on)} />} />
        ))}
      </List>
      {error && <Problem title={error} />}
      {note && <Hint>{note}</Hint>}
    </div>
  )
}

/// Add to… or Manage for `a`, and its sheet. `store` is `useApps`. `then(a)` is told, as the sheet closes, of an app that
/// has got onto a page and has no main screen to open, since its page says what comes next (an account, a device).
export function PagesButton({ a, store, variant = "secondary", then }) {
  const [dialog, show, setOpen] = useDialog()
  const was = useRef(false)
  const close = () => {
    setOpen(false)
    if (!was.current && onAPage(a, store.data.places) && !a.opens) then?.(a)
  }
  return (
    <>
      <Button size="sm" variant={variant} onClick={() => ((was.current = onAPage(a, store.data.places)), show())}>
        {onAPage(a, store.data.places) ? "Manage" : "Add to"}
      </Button>
      {dialog.n > 0 && (
        <ResponsiveDialog key={dialog.n} open={dialog.open} onOpenChange={(o) => (o ? setOpen(true) : close())} title={a.title} action={<Button size="sm" onClick={close}>Done</Button>}>
          <Pages name={a.name} store={store} />
        </ResponsiveDialog>
      )}
    </>
  )
}
