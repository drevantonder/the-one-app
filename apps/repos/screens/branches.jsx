// A repo's branches, the default first and then the newest: each opens the repo on it.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { Badge, Empty, Grid, GridRow, Icon, Pending, useCall } from "@one/ui"
import { Frame, ago, go, short, useAddress, where } from "../lib/use.jsx"

const P = one.params

function Branches() {
  const { address } = useAddress()
  const call = useCall("branches", { repo: P.repo })
  useEffect(() => {
    one.title("Branches")
  }, [])
  if (!call.data) return <Pending call={call} rows={3} />
  if (!call.data.length) {
    return (
      <Empty>
        <span className="block">No branches yet. {where}, push one:</span>
        <code className="mt-2 block rounded-md bg-muted px-2 py-1 font-mono text-xs">git push {address(P.repo)} main</code>
      </Empty>
    )
  }
  const sorted = [...call.data].sort((a, b) => (b.branch === "main") - (a.branch === "main") || Date.parse(b.time) - Date.parse(a.time))
  return (
    <Grid min="22rem" max={4} gap="0.25rem">
        {sorted.map((b) => (
          <GridRow
            key={b.branch}
            leading={<Icon name="git-branch" className="size-5 text-muted-foreground" />}
            title={<span className="flex items-center gap-2"><span className="truncate">{b.branch}</span>{b.branch === "main" && <Badge variant="secondary">Default</Badge>}</span>}
            trailing={<span className="text-xs text-muted-foreground">{ago(b.time)}</span>}
            sub={b.subject ? `${b.subject} · ${b.author}` : `${short(b.head)} · ${b.pushed_by}`}
            onClick={() => go("repo", { repo: P.repo, ref: b.branch, path: P.path || "" })}
          />
        ))}
    </Grid>
  )
}

createRoot(document.getElementById("root")).render(<Frame><Branches /></Frame>)
