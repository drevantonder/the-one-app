// A repo's commits, newest first, under the day each was made; a commit opens beside the list where there's room.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, Button, Empty, Icon, List } from "@one/ui"
import { CommitView } from "../lib/commit-view.jsx"
import { PathBar } from "../lib/frame.jsx"
import { SmallCopy } from "../lib/parts.jsx"
import { Beside, Frame, Pending, ago, go, short, useCall, useWide } from "../lib/use.jsx"

const P = one.params
const at = { repo: P.repo, ref: P.ref, path: P.path || "" }
const day = (ms) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })

function Row({ c, selected, open }) {
  return (
    <div
      role="button"
      tabIndex={0}
      data-selected={selected || undefined}
      onClick={() => open(c)}
      onKeyDown={(e) => e.target === e.currentTarget && (e.key === "Enter" || e.key === " ") && (e.preventDefault(), open(c))}
      className="flex w-full min-w-0 cursor-pointer items-center gap-3 px-3 py-2.5 text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50 data-[selected]:bg-accent"
    >
      <div className="min-w-0 flex-1">
        <div className="truncate font-medium">{c.subject}</div>
        <div className="flex items-center gap-1.5 truncate text-muted-foreground">
          <Avatar name={c.author} size={16} />
          <span className="truncate"><strong className="font-medium">{c.author}</strong> committed {ago(c.time)}</span>
        </div>
      </div>
      <span className="flex shrink-0 items-center gap-1 text-muted-foreground">
        <code className="text-xs">{short(c.sha)}</code>
        <span onClick={(e) => e.stopPropagation()}><SmallCopy text={c.sha} /></span>
      </span>
    </div>
  )
}

function History() {
  const wide = useWide()
  const page = Number(P.page) || 0
  const log = useCall("log", { ...at, page })
  const [shown, setShown] = useState(null)
  const d = log.data

  useEffect(() => {
    one.title(P.path ? `History of ${P.path.split("/").pop()}` : "History")
  }, [])

  if (!d) return <Pending call={log} rows={5} />
  if (!d.commits.length) {
    return (
      <div className="flex max-w-2xl flex-col gap-4">
        <PathBar d={d} />
        <Empty>{page ? "No older commits." : "No commits yet."}</Empty>
      </div>
    )
  }
  const groups = []
  for (const c of d.commits) {
    const k = day(c.time)
    if (groups[groups.length - 1]?.day !== k) groups.push({ day: k, commits: [] })
    groups[groups.length - 1].commits.push(c)
  }
  const open = (c) => (wide ? setShown(c.sha) : go("commit", { repo: d.repo, sha: c.sha }))
  const list = (
    <div className="flex max-w-2xl flex-col gap-6">
      <PathBar d={d} />
      {groups.map((g) => (
        <section key={g.day} className="flex flex-col gap-2">
          <h2 className="flex items-center gap-2 text-sm font-medium text-muted-foreground"><Icon name="git-commit" />Commits on {g.day}</h2>
          <List>{g.commits.map((c) => <Row key={c.sha} c={c} selected={wide && shown === c.sha} open={open} />)}</List>
        </section>
      ))}
      <div className="flex gap-2">
        <Button variant="outline" disabled={!page} onClick={() => go("history", { ...at, page: page - 1 })}>Newer</Button>
        <Button variant="outline" disabled={!d.more} onClick={() => go("history", { ...at, page: page + 1 })}>Older</Button>
      </div>
    </div>
  )
  return <Beside size="28rem" list={list} detail={shown && <CommitView key={shown} repo={d.repo} sha={shown} files={false} />} />
}

createRoot(document.getElementById("root")).render(<Frame><History /></Frame>)
