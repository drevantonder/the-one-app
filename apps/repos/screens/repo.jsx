// A repo, as GitHub shows one: its tabs, its files (a folder of them, with each one's last commit where there's room),
// its README, and its branches and commits. Deleted, it says so, with Restore for its owners, and Erase.
import { useCallback, useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Card, CardContent, CopyButton, Empty, Icon, List, ListRow, Markdown, Problem, Spinner } from "@one/ui"
import { CommitCard, PathBar, RepoTabs } from "../lib/frame.jsx"
import { GitHubMark, RepoIcon } from "../lib/parts.jsx"
import { Frame, Pending, ago, go, refOf, said, short, useAddress, useBeside, useCall, useWide, where } from "../lib/use.jsx"

const P = one.params
const at = P.path || ""

const ICONS = { tree: "folder", blob: "file", commit: "folder-dashed" }

function Entry({ d, e, last }) {
  const wide = useWide()
  const path = (at ? at + "/" : "") + e.name
  const open = e.type === "tree" ? () => go("repo", { repo: d.repo, ref: refOf(d), path }) : e.type === "blob" ? () => go("file", { repo: d.repo, ref: refOf(d), path }) : undefined
  const Row = open ? "button" : "div"
  return (
    <Row
      type={open ? "button" : undefined}
      onClick={open}
      className={`grid w-full min-w-0 items-center gap-3 px-3 py-2 text-left text-sm ${open ? "cursor-pointer outline-none hover:bg-accent/50 focus-visible:bg-accent/50" : ""}`}
      style={{ gridTemplateColumns: wide ? "minmax(0, 16rem) minmax(0, 1fr) 6rem" : "minmax(0, 1fr)" }}
    >
      <span className="flex min-w-0 items-center gap-2.5">
        <Icon name={ICONS[e.type]} className="size-4 text-muted-foreground" />
        <span className="truncate">{e.name}{e.type === "commit" && <span className="text-muted-foreground"> @ {short(e.sha)}</span>}</span>
      </span>
      {wide && (
        <span className="min-w-0 truncate text-muted-foreground">
          {last && (
            <span role="link" tabIndex={0} className="cursor-pointer hover:text-foreground hover:underline" onClick={(ev) => (ev.stopPropagation(), go("commit", { repo: d.repo, sha: last.sha }))}>
              {last.subject}
            </span>
          )}
        </span>
      )}
      {wide && <span className="truncate text-right text-xs text-muted-foreground">{last ? ago(last.time) : ""}</span>}
    </Row>
  )
}

function Readme({ d }) {
  let text = d.readme.text
  if (/\.mdx$/i.test(d.readme.name)) text = text.split("\n").filter((l) => !/^\s*(import|export)\s/.test(l) && !/^\s*<\/?[A-Z][\w.]*[^>]*\/?>\s*$/.test(l)).join("\n")
  const ref = refOf(d)
  const link = useCallback(
    (href) => {
      const base = at ? at + "/" : ""
      const p = new URL(href, "file:///" + base).pathname.slice(1).replace(/\/$/, "")
      const path = decodeURIComponent(p)
      go(/\.[\w]+$/.test(path) ? "file" : "repo", { repo: d.repo, ref, path })
    },
    [d.repo, ref],
  )
  const markdown = /\.(md|mdx|markdown)$/i.test(d.readme.name)
  return (
    <Card className="min-w-0 gap-0 py-0">
      <div className="flex items-center gap-2 border-b px-4 py-3 text-sm font-medium"><Icon name="book" className="text-muted-foreground" />{d.readme.name}</div>
      <CardContent className="px-6 py-5">
        {markdown ? <Markdown text={text} onLink={link} /> : <pre className="font-mono text-sm break-words whitespace-pre-wrap">{text}</pre>}
      </CardContent>
    </Card>
  )
}

function Deleted({ d }) {
  return (
    <div className="flex items-center gap-3">
      <RepoIcon repo={d.repo} github={!!d.github} className="size-7 text-muted-foreground" />
      <div className="min-w-0">
        <div className="truncate text-sm text-muted-foreground">{d.owner.name}</div>
        <h1 className="truncate text-xl font-semibold">{d.name}</h1>
      </div>
    </div>
  )
}

function Clone({ d }) {
  const { address } = useAddress()
  return <CopyButton variant="outline" text={`git clone ${d.clone || address(d.repo)}`} label="Clone" />
}

// On a phone there are no tabs, so a repo's other pages are rows above its files.
function Pages({ d }) {
  return (
    <List className="max-w-none">
      <ListRow leading={<Icon name="record" className="size-5 text-muted-foreground" />} title="Issues" trailing={d.open_issues ? <span className="text-muted-foreground">{d.open_issues.toLocaleString()}</span> : undefined} onClick={() => go("issues", { repo: d.repo })} />
      {d.yours && !d.github && <ListRow leading={<Icon name="gear" className="size-5 text-muted-foreground" />} title="Settings" onClick={() => go("repo_settings", { repo: d.repo })} />}
    </List>
  )
}

function Forks({ d }) {
  if (!d.forks?.length) return null
  return (
    <List className="max-w-none">
      {d.forks.map((f) => (
        <ListRow key={f} leading={<Icon name="git-branch" className="size-5 text-muted-foreground" />} title="Fork" trailing={<span className="truncate text-muted-foreground">{f}</span>} onClick={() => go("repo", { repo: f })} />
      ))}
    </List>
  )
}

function Erase() {
  const [error, setError] = useState("")
  const retire = async () => {
    const ok = await one.alert({ title: `Erase ${P.repo} for good?`, text: "Its branches and history are gone, and its name is free again.", action: "Erase", danger: true })
    if (!ok) return
    try {
      await one.call("erase", { repo: P.repo, confirm: true })
      one.close()
    } catch (err) {
      setError(said(err).title)
    }
  }
  return (
    <div className="flex flex-col items-start gap-3">
      {error && <Problem title={error} />}
      <Button variant="destructive" onClick={retire}>Erase Repository</Button>
    </div>
  )
}

function Restore() {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const restore = async () => {
    setBusy(true)
    setError("")
    try {
      await one.call("restore", { repo: P.repo })
    } catch (err) {
      setError(said(err).title)
      setBusy(false)
    }
  }
  return (
    <div className="flex flex-col items-start gap-3">
      {error && <Problem title={error} />}
      <Button variant="secondary" disabled={busy} onClick={restore}>{busy && <Spinner />}Restore</Button>
    </div>
  )
}

function Files({ d }) {
  const wide = useWide()
  const latest = useCall("latest", wide && !d.empty ? { repo: d.repo, ref: refOf(d), path: at } : null)
  const last = latest.data || {}
  return d.entries.length ? (
    <List className="max-w-none">
      {at && (
        <button type="button" onClick={() => go("repo", { repo: d.repo, ref: refOf(d), path: at.split("/").slice(0, -1).join("/") })} className="flex w-full cursor-pointer items-center gap-2.5 px-3 py-2 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50">
          <Icon name="folder" className="text-muted-foreground" />..
        </button>
      )}
      {d.entries.map((e) => <Entry key={e.name} d={d} e={e} last={last[e.name]} />)}
    </List>
  ) : (
    <Empty>This folder is empty.</Empty>
  )
}

function Code({ d }) {
  const wide = useWide()
  const beside = useBeside() && d.readme
  const files = (
    <div className="flex min-w-0 flex-col gap-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <PathBar d={d} />
        {!at && (
          <div className="flex items-center gap-2">
            {d.github && <Button variant="outline" size="sm" onClick={() => one.link(d.github.url)}><GitHubMark />GitHub{d.github.private ? " · Private" : ""}</Button>}
            <Clone d={d} />
          </div>
        )}
      </div>
      {!at && d.from && (
        <p className="text-sm text-muted-foreground">
          Forked from <button type="button" className="underline underline-offset-4 hover:text-foreground" onClick={() => go("repo", { repo: d.from })}>{d.from}</button>
        </p>
      )}
      {!at && !wide && <Pages d={d} />}
      <CommitCard c={d.latest} d={d} />
      <Files d={d} />
      {!at && <Forks d={d} />}
    </div>
  )
  return (
    <div className={beside ? "grid grid-cols-2 items-start gap-6" : "flex flex-col gap-6"}>
      {files}
      {d.readme && <Readme d={d} />}
    </div>
  )
}

function Repo() {
  const { address } = useAddress()
  const tree = useCall("tree", { repo: P.repo, ref: P.ref, path: at })
  const d = tree.data

  useEffect(() => {
    one.title(at ? at.split("/").pop() : P.repo.split("/").pop())
  }, [])
  useEffect(() => {
    if (d?.file) one.open("file", { repo: d.repo, ref: refOf(d), path: at })
  }, [d?.file])

  if (!d) return <Frame><Pending call={tree} rows={3} /></Frame>
  if (d.file) return null
  const info = { open_issues: d.open_issues, yours: d.yours }

  if (d.deleted) {
    return (
      <Frame>
        <div className="flex max-w-2xl flex-col gap-6">
          <Deleted d={d} />
          <p className="text-sm text-muted-foreground">Deleted {ago(d.deleted.time)}</p>
          {d.yours && <Restore />}
          {d.yours && <Erase />}
        </div>
      </Frame>
    )
  }

  if (d.empty) {
    const url = d.clone || address(d.repo)
    return (
      <Frame>
        <RepoTabs repo={d.repo} on="code" info={info} />
        <Empty>
          <span className="block">No code yet.{d.push && ` ${d.clone ? "Push some" : `${where}, push some`}:`}</span>
          {d.push && <code className="mt-2 block rounded-md bg-muted px-2 py-1 font-mono text-xs">git push {url} main</code>}
        </Empty>
      </Frame>
    )
  }

  return (
    <Frame>
      <RepoTabs repo={d.repo} on="code" info={info} />
      <Code d={d} />
    </Frame>
  )
}

createRoot(document.getElementById("root")).render(<Repo />)
