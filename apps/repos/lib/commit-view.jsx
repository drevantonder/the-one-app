// A commit: who made it and what it says, the files it changed, and each file's diff, loaded as it comes near the
// screen. It's the Commit screen, and what History shows beside its list where there's room.
import { useEffect, useRef, useState } from "react"
import { Avatar, Badge, Button, Card, DiffView, Empty, Icon, List, ListRow, Problem, Spinner, Tabs, TabsList, TabsTrigger, useWidth } from "@one/ui"
import { Counts, SmallCopy } from "./parts.jsx"
import { Pending, ago, go, plural, said, short, useCall } from "./use.jsx"

const BADGE = { added: "Added", deleted: "Deleted", renamed: "Renamed" }

function Diff({ f, split, load, loading }) {
  if (f.failed) return <div className="p-4"><Problem title={said(f.failed).title} action={{ label: "Try Again", run: () => load(true) }} /></div>
  if (f.binary) return <p className="px-4 py-3 text-sm text-muted-foreground">Binary file not shown.</p>
  if (f.too_big) return <p className="px-4 py-3 text-sm text-muted-foreground">This file is too big to diff here.</p>
  if (f.large && !f.hunks) {
    return (
      <div className="flex items-center justify-between gap-3 px-4 py-3 text-sm">
        <span className="text-muted-foreground">Large diff, {(f.additions + f.deletions).toLocaleString()} lines.</span>
        <Button variant="outline" size="sm" disabled={loading} onClick={() => load(true)}>{loading && <Spinner />}Load diff</Button>
      </div>
    )
  }
  if (!f.hunks) {
    // Room for about what's coming, so the page doesn't jump as diffs come in above where someone's reading.
    return (
      <div className="flex items-start px-4 py-3 text-sm text-muted-foreground" style={{ minHeight: Math.min(f.additions + f.deletions + 2, 30) * 20 }}>
        {loading && <span className="flex items-center gap-2"><Spinner />Loading diff</span>}
      </div>
    )
  }
  if (!f.hunks.length) {
    return <p className="px-4 py-3 text-sm text-muted-foreground">{f.status === "renamed" ? "File renamed without changes." : f.old_mode !== f.new_mode ? "Only the file's mode changed." : "No changes to show."}</p>
  }
  return <DiffView hunks={f.hunks} path={f.path} split={split} />
}

function DiffFile({ d, f, k, split, load, loading }) {
  const [folded, setFolded] = useState(false)
  const box = useRef(null)
  useEffect(() => {
    if (!f.later || f.large || f.hunks || f.failed || loading) return
    if (!window.IntersectionObserver) return void load(false)
    const near = new IntersectionObserver(([e]) => e.isIntersecting && (near.disconnect(), load(false)), { rootMargin: "800px 0px" })
    near.observe(box.current)
    return () => near.disconnect()
  }, [f.later, f.large, !!f.hunks])
  const at = f.status === "deleted" ? d.parents[0] : d.sha
  const mode = f.old_mode && f.new_mode && f.old_mode !== f.new_mode ? `${f.old_mode} → ${f.new_mode}` : ""
  return (
    <Card ref={box} id={`f${k}`} className="scroll-mt-4 gap-0 overflow-hidden py-0">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1 py-1.5 pr-3 pl-1.5 text-sm">
        <Button variant="ghost" size="icon-sm" aria-label={folded ? "Expand" : "Collapse"} onClick={() => setFolded(!folded)}>
          <Icon name={folded ? "caret-right" : "caret-down"} />
        </Button>
        <span className="min-w-0 flex-1 font-mono text-xs break-all">{f.old_path ? `${f.old_path} → ` : ""}{f.path}</span>
        {BADGE[f.status] && <Badge variant="secondary">{BADGE[f.status]}</Badge>}
        {mode && <span className="text-xs text-muted-foreground">{mode}</span>}
        <Counts add={f.additions} del={f.deletions} className="text-xs" />
        {at && <Button variant="ghost" size="sm" onClick={() => go("file", { repo: d.repo, ref: at, path: f.path })}>View file</Button>}
      </div>
      {!folded && <div className="border-t"><Diff f={f} split={split} load={load} loading={loading} /></div>}
    </Card>
  )
}

export function CommitView({ repo, sha, files = true }) {
  const commit = useCall("commit", { repo, sha })
  const d = commit.data
  const [room, width] = useWidth()
  const [asked, setAsked] = useState(null)
  const split = asked ?? width >= 720
  const [more, setMore] = useState({})
  const [asking, setAsking] = useState({})

  useEffect(() => {
    setMore({})
    setAsking({})
  }, [repo, sha])

  if (!d) return <div ref={room}><Pending call={commit} rows={3} /></div>

  const load = async (k, loud) => {
    const patch = (p) => setMore((m) => ({ ...m, [k]: { ...m[k], ...p } }))
    patch({ failed: null })
    const slow = setTimeout(() => setAsking((a) => ({ ...a, [k]: true })), loud ? 0 : 400)
    try {
      patch(await one.call("diff", { repo, sha, path: d.files[k].path }))
    } catch (err) {
      patch({ failed: err })
    }
    clearTimeout(slow)
    setAsking((a) => ({ ...a, [k]: false }))
  }
  const shown = d.files.map((f, k) => ({ ...f, ...more[k] }))
  const add = d.files.reduce((n, f) => n + f.additions, 0)
  const del = d.files.reduce((n, f) => n + f.deletions, 0)
  const scroll = (k) => document.getElementById(`f${k}`)?.scrollIntoView({ behavior: "smooth" })

  return (
    <div ref={room} className="flex min-w-0 flex-col gap-4">
      <div className="flex max-w-3xl flex-col gap-3">
        <h1 className="text-xl font-semibold break-words">{d.subject}</h1>
        {d.body && <pre className="font-sans text-sm break-words whitespace-pre-wrap text-muted-foreground">{d.body}</pre>}
        <div className="flex items-center gap-2 text-sm">
          <Avatar name={d.author} size={20} />
          <span><strong className="font-semibold">{d.author}</strong> committed {ago(d.time)}</span>
        </div>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 text-sm text-muted-foreground">
          <span className="flex items-center gap-1">commit <code>{short(d.sha)}</code><SmallCopy text={d.sha} /></span>
          <span>
            {d.parents.length ? <>{plural(d.parents.length, "parent", "parents")}{" "}</> : "No parents"}
            {d.parents.map((p, i) => (
              <span key={p}>{i > 0 && " + "}<button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={() => go("commit", { repo: d.repo, sha: p })}><code>{short(p)}</code></button></span>
            ))}
          </span>
          <Button variant="outline" size="sm" onClick={() => go("repo", { repo: d.repo, ref: d.sha })}><Icon name="code" />Browse files</Button>
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 text-sm">
        <span>{plural(d.files.length, "file", "files")} changed, <Counts add={add} del={del} /></span>
        <Tabs value={split ? "split" : "unified"} onValueChange={(v) => setAsked(v === "split")}>
          <TabsList aria-label="Diff layout">
            <TabsTrigger value="unified">Unified</TabsTrigger>
            <TabsTrigger value="split">Split</TabsTrigger>
          </TabsList>
        </Tabs>
      </div>

      {d.files.length === 0 ? (
        <Empty>This commit changes no files.</Empty>
      ) : (
        <>
          {files && (
            <List className="max-w-3xl">
              {d.files.map((f, k) => (
                <ListRow
                  key={k}
                  leading={<Icon name={f.status === "added" ? "file" : f.status === "deleted" ? "x" : "git-diff"} className="size-4 text-muted-foreground" />}
                  title={<span className="font-mono text-xs">{f.path}</span>}
                  trailing={<Counts add={f.additions} del={f.deletions} className="text-xs" />}
                  onClick={() => scroll(k)}
                />
              ))}
            </List>
          )}
          {shown.map((f, k) => <DiffFile key={k} d={d} f={f} k={k} split={split} load={(loud) => load(k, loud)} loading={!!asking[k]} />)}
        </>
      )}
    </div>
  )
}
