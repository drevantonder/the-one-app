// A repo's commits, newest first, under the day each was made; a commit opens beside the list where there's room.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Avatar, Button, Empty, List, Pending, Section, Split, useCall } from "@one/ui"
import { CommitView } from "../lib/commit-view.jsx"
import { PathBar, placed } from "../lib/frame.jsx"
import { SmallCopy } from "../lib/parts.jsx"
import { WIDE_BESIDE, Frame, ago, go, short, useBeside, useWide } from "../lib/use.jsx"

const P = one.params
const at = { repo: P.repo, ref: P.ref, path: P.path || "" }
const day = (ms) => new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" })

function Row({ c, selected, open }) {
  const wide = useWide()
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
        <div className="line-clamp-2 font-medium break-words">{c.subject}</div>
        <div className="flex items-center gap-1.5 truncate text-muted-foreground">
          <Avatar name={c.author} size={16} />
          <span className="truncate">{c.author} · {ago(c.time)}</span>
        </div>
      </div>
      {wide && (
        <span className="flex shrink-0 items-center gap-1 text-muted-foreground">
          <code className="text-xs">{short(c.sha)}</code>
          <span onClick={(e) => e.stopPropagation()}><SmallCopy text={c.sha} /></span>
        </span>
      )}
    </div>
  )
}

function History() {
  const beside = useBeside()
  const page = Number(P.page) || 0
  const log = useCall("log", { ...at, page })
  const [shown, setShown] = useState(null)
  const d = log.data

  useEffect(() => {
    one.title(P.path ? `History of ${P.path.split("/").pop()}` : "History")
  }, [])

  if (!d) {
    return (
      <div className="flex max-w-2xl flex-col gap-4">
        <PathBar d={placed(at)} />
        <Pending call={log} rows={5} />
      </div>
    )
  }
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
  const open = (c) => (beside ? setShown(c.sha) : go("commit", { repo: d.repo, sha: c.sha }))
  // Where there's room the newest is open from the start, so the page isn't half empty.
  const current = beside ? shown ?? d.commits[0].sha : null
  const list = (
    <div className="flex max-w-2xl flex-col gap-4">
      <PathBar d={d} />
      {groups.map((g) => (
        <Section key={g.day} title={g.day}>
          <List>{g.commits.map((c) => <Row key={c.sha} c={c} selected={current === c.sha} open={open} />)}</List>
        </Section>
      ))}
      <div className="flex gap-2">
        <Button variant="outline" disabled={!page} onClick={() => go("history", { ...at, page: page - 1 })}>Newer</Button>
        <Button variant="outline" disabled={!d.more} onClick={() => go("history", { ...at, page: page + 1 })}>Older</Button>
      </div>
    </div>
  )
  return <Split wide={WIDE_BESIDE} size="28rem" list={list} detail={beside && <CommitView repo={d.repo} sha={current} files={false} />} />
}

createRoot(document.getElementById("root")).render(<Frame><History /></Frame>)
