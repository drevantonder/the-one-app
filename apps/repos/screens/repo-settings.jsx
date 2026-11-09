// A repo's Settings, for its owners, as GitHub's: who pushes to it, its labels, and deleting it.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Icon, List, ListRow, Problem, Pending, useCall } from "@one/ui"
import { RepoTabs } from "../lib/frame.jsx"
import { Frame, Section, go, said } from "../lib/use.jsx"

const P = one.params

function Settings() {
  const call = useCall("issues", { repo: P.repo, page: 0 })
  const [error, setError] = useState("")
  useEffect(() => {
    one.title("Settings")
  }, [])
  if (!call.data) return <><RepoTabs repo={P.repo} on="settings" /><Pending call={call} rows={2} /></>
  const d = call.data
  const tabs = <RepoTabs repo={P.repo} on="settings" info={d} />
  if (!d.yours) return <>{tabs}<Empty>Only the owners of {P.repo} change its settings.</Empty></>
  const retire = async () => {
    const ok = await one.alert({ title: `Delete ${P.repo}?`, text: "No one can clone or push to it. You can restore it from Recently Deleted.", action: "Delete", danger: true })
    if (!ok) return
    try {
      await one.call("delete", { repo: P.repo, confirm: true })
      one.open("repo", { repo: P.repo })
    } catch (err) {
      setError(said(err).title)
    }
  }
  return (
    <>
      {tabs}
      <div className="flex max-w-2xl flex-col gap-6">
        <List>
          <ListRow leading={<Icon name="users" className="size-5 text-muted-foreground" />} title="Collaborators" onClick={() => go("collaborators", { repo: P.repo })} />
          <ListRow leading={<Icon name="tag" className="size-5 text-muted-foreground" />} title="Labels" onClick={() => go("labels", { repo: P.repo })} />
        </List>
        <Section title="Danger Zone">
          {error && <Problem title={error} />}
          <Button variant="destructive" className="self-start" onClick={retire}>Delete Repository</Button>
        </Section>
      </div>
    </>
  )
}

createRoot(document.getElementById("root")).render(<Frame><Settings /></Frame>)
