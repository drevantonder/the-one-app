// Repos: this network's repos, GitHub's beside them (connected, not copied), and Recently Deleted; searchable.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Icon, Input, List, ListRow, Problem, Spinner } from "@one/ui"
import { RepoIcon } from "../lib/parts.jsx"
import { Frame, Pending, Section, ago, go, plural, said, useAddress, useCall, where } from "../lib/use.jsx"

function Row({ r, restoring, restore }) {
  const deleted = r.deleted && !restoring.has(r.repo)
  const sub = deleted
    ? "Deleted " + ago(r.deleted.time)
    : [r.github?.private && "Private", r.branch, ago(r.updated || r.created), r.from && "fork of " + r.from, r.open_issues && plural(r.open_issues, "open issue", "open issues")].filter(Boolean).join(" · ")
  const owner = r.repo.slice(0, -r.name.length)
  const action = deleted && r.yours && (
    <Button variant="secondary" size="sm" onClick={(e) => (e.stopPropagation(), restore(r.name))}>Restore</Button>
  )
  return (
    <ListRow
      leading={<RepoIcon repo={r.repo} github={!!r.github} className="size-5 text-muted-foreground" />}
      title={<><span className="font-normal text-muted-foreground">{owner}</span>{r.name}</>}
      sub={sub}
      trailing={action || undefined}
      onClick={() => go("repo", { repo: r.repo })}
    />
  )
}

function Main() {
  const { address, mine } = useAddress()
  const own = useCall("list", { source: "one" })
  // GitHub's come as GitHub answers, after the network's own: its problem is its own, not the whole list's.
  const github = useCall("list", { source: "github" })
  const [query, setQuery] = useState("")
  // Restored here and maybe not yet on the device, so a redraw meanwhile doesn't put them back under Recently Deleted.
  const [restoring, setRestoring] = useState(new Set())
  const [trouble, setTrouble] = useState("")

  useEffect(() => {
    one.title("Repos")
  }, [])

  const restore = (name) => {
    setTrouble("")
    setRestoring((s) => new Set(s).add(name))
    one.call("restore", { repo: name })
      .then(() => own.reload())
      .catch((err) => setTrouble(said(err).title))
      .finally(() => setRestoring((s) => (s.delete(name), new Set(s))))
  }

  if (!own.data) return <Frame><Pending call={own} /></Frame>

  const q = query.trim().toLowerCase()
  const githubs = github.data
  const githubTrouble = github.error ? said(github.error).title : ""
  const mineShown = own.data.filter((r) => !q || r.repo.includes(q) || (r.from || "").includes(q))
  const githubShown = (githubs || []).filter((r) => !q || r.repo.toLowerCase().includes(q))
  const gone = mineShown.filter((r) => r.deleted && !restoring.has(r.repo))
  const here = mineShown.filter((r) => !gone.includes(r))
  const row = (r) => <Row key={r.repo} r={r} restoring={restoring} restore={restore} />
  const asked = githubs || github.error

  if (!own.data.length && !githubs?.length && asked && !githubTrouble) {
    return (
      <Frame>
        <Empty>
          <span className="block">No repos yet. {where}, push one:</span>
          <code className="mt-2 block rounded-md bg-muted px-2 py-1 font-mono text-xs">git push {address(`${mine}/<name>`)} main</code>
        </Empty>
      </Frame>
    )
  }

  return (
    <Frame>
      <div className="flex max-w-2xl flex-col gap-6">
        <div className="relative">
          <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground" />
          <Input className="pl-8" placeholder="Find a repo" aria-label="Find a repo" value={query} onChange={(e) => setQuery(e.target.value)} />
        </div>
        {trouble && <Problem title={trouble} />}
        {here.length > 0 && <List>{here.map(row)}</List>}
        {githubTrouble ? (
          <Section title="GitHub"><Problem title={githubTrouble} action={{ label: "Settings", run: () => one.open("settings") }} /></Section>
        ) : !githubs ? (
          <Section title="GitHub">
            <p className="flex items-center gap-2 text-sm text-muted-foreground"><Spinner />Loading GitHub's repos…</p>
          </Section>
        ) : (
          githubShown.length > 0 && <Section title="GitHub"><List>{githubShown.map(row)}</List></Section>
        )}
        {gone.length > 0 && <Section title="Recently Deleted"><List>{gone.map(row)}</List></Section>}
        {!here.length && !githubShown.length && !gone.length && asked && !githubTrouble && <Empty>No repos match.</Empty>}
      </div>
    </Frame>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
