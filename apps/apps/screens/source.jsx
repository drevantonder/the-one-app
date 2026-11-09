// The sheet that sets an app's source link: Save, or Remove Link (or an empty field) to clear it.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Input, Pending, Problem } from "@one/ui"
import { said, useList } from "../lib/store.jsx"

function Source() {
  const list = useList()
  const name = one.params.app
  const a = list.data?.apps.find((x) => x.name === name)
  const [link, setLink] = useState(null)
  const [error, setError] = useState("")
  const [saving, setSaving] = useState(false)
  const field = useRef(null)

  useEffect(() => {
    if (a && link === null) {
      setLink(a.source || "")
      setTimeout(() => field.current?.focus(), 300)
    }
  }, [a, link])

  if (!list.data) return <div className="p-4"><Pending call={list} rows={1} /></div>
  if (!a) return <Empty>This app isn't here. It may have been erased, or the link is old.</Empty>

  const save = async (to) => {
    setSaving(true)
    setError("")
    try {
      await one.call("source", { app: name, link: to })
      one.close()
    } catch (err) {
      setError(said(err).title)
      setSaving(false)
    }
  }

  return (
    <form className="flex flex-col items-start gap-4 p-4" onSubmit={(e) => (e.preventDefault(), save((link || "").trim()))}>
      <Input
        ref={field}
        type="url"
        placeholder="https://github.com/…"
        aria-label="Source link"
        autoCapitalize="none"
        autoCorrect="off"
        spellCheck={false}
        value={link ?? ""}
        onChange={(e) => setLink(e.target.value)}
      />
      {error && <Problem title={error} />}
      <Button type="submit" disabled={saving}>Save</Button>
      {a.source && <Button type="button" variant="destructive" size="sm" disabled={saving} onClick={() => save("")}>Remove Link</Button>}
    </form>
  )
}

createRoot(document.getElementById("root")).render(<Source />)
