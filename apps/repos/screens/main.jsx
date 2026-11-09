// Repos: this network's repos, GitHub's beside them (connected, not copied), and Recently Deleted; searchable.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Grid, Icon, Input, LargeTitle, List, ListRow, ListSkeleton, Pending, Problem, Section, useCall } from "@one/ui"
import { RepoIcon } from "../lib/parts.jsx"
import { Frame, ago, go, plural, said, useAddress, where } from "../lib/use.jsx"

function Gone({ r, restore }) {
  return (
    <ListRow
      leading={<RepoIcon repo={r.repo} github={!!r.github} className="size-5 text-muted-foreground" />}
      title={<><span className="font-normal text-muted-foreground">{r.repo.slice(0, -r.name.length)}</span>{r.name}</>}
      sub={"Deleted " + ago(r.deleted.time)}
      trailing={r.yours ? <Button variant="secondary" size="sm" onClick={(e) => (e.stopPropagation(), restore(r.name))}>Restore</Button> : undefined}
      onClick={() => go("repo", { repo: r.repo })}
    />
  )
}

// A repo that's here: no chevron, since the grid's rows sit side by side where there's room.
function Tile({ r }) {
  const owner = r.repo.slice(0, -r.name.length)
  const sub = [r.github?.private && "Private", r.branch, ago(r.updated || r.created), r.from && "fork of " + r.from, r.open_issues && plural(r.open_issues, "open issue", "open issues")].filter(Boolean).join(" · ")
  return (
    <button type="button" className="min-w-0 cursor-pointer rounded-xl text-left outline-none hover:bg-accent/50 focus-visible:bg-accent/50" onClick={() => go("repo", { repo: r.repo })}>
      <ListRow leading={<RepoIcon repo={r.repo} github={!!r.github} className="size-5 text-muted-foreground" />} title={<><span className="font-normal text-muted-foreground">{owner}</span>{r.name}</>} sub={sub} />
    </button>
  )
}

const Tiles = ({ of }) => <Grid min="22rem" max={4} gap="0.25rem">{of.map((r) => <Tile key={r.repo} r={r} />)}</Grid>

function Main() {
  const { address, mine } = useAddress()
  const own = useCall("list", { source: "one" })
  // GitHub's come as GitHub answers, after the network's own: its problem is its own, not the whole list's.
  const github = useCall("list", { source: "github" })
  const [query, setQuery] = useState("")
  // Restored here and maybe not yet on the device, so a redraw meanwhile doesn't put them back under Recently Deleted.
  const [restoring, setRestoring] = useState(new Set())
  const [trouble, setTrouble] = useState("")

  const restore = (name) => {
    setTrouble("")
    setRestoring((s) => new Set(s).add(name))
    one.call("restore", { repo: name })
      .then(() => own.reload())
      .catch((err) => setTrouble(said(err).title))
      .finally(() => setRestoring((s) => (s.delete(name), new Set(s))))
  }

  const find = (
    <div className="relative max-w-md">
      <Icon name="magnifying-glass" className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-muted-foreground" />
      <Input className="pl-8" placeholder="Search" aria-label="Search" value={query} onChange={(e) => setQuery(e.target.value)} />
    </div>
  )
  const title = <LargeTitle>Repos</LargeTitle>
  if (!own.data) return <Frame><div className="flex flex-col gap-6">{title}{find}<Pending call={own} rows={6} tiles="22rem" icon={20} /></div></Frame>

  const q = query.trim().toLowerCase()
  const githubs = github.data
  const githubTrouble = github.error ? said(github.error).title : ""
  const mineShown = own.data.filter((r) => !q || r.repo.includes(q) || (r.from || "").includes(q))
  const githubShown = (githubs || []).filter((r) => !q || r.repo.toLowerCase().includes(q))
  const gone = mineShown.filter((r) => r.deleted && !restoring.has(r.repo))
  const here = mineShown.filter((r) => !gone.includes(r))
  const asked = githubs || github.error

  if (!own.data.length && !githubs?.length && asked && !githubTrouble) {
    return (
      <Frame>
        {title}
        <Empty>
          <span className="block">No repos yet. {where}, push one:</span>
          <code className="mt-2 block rounded-md bg-muted px-2 py-1 font-mono text-xs">git push {address(`${mine}/<name>`)} main</code>
        </Empty>
      </Frame>
    )
  }

  return (
    <Frame>
      <div className="flex flex-col gap-6">
        {title}
        {find}
        {trouble && <Problem title={trouble} />}
        {here.length > 0 ? <Tiles of={here} /> : !asked && !q && <ListSkeleton rows={6} tiles="22rem" icon={20} />}
        {githubTrouble ? (
          <Section title="GitHub"><Problem title={githubTrouble} action={{ label: "Settings", run: () => one.open("settings") }} /></Section>
        ) : (
          githubShown.length > 0 && <Section title="GitHub"><Tiles of={githubShown} /></Section>
        )}
        {gone.length > 0 && <Section title="Recently Deleted"><List>{gone.map((r) => <Gone key={r.repo} r={r} restore={restore} />)}</List></Section>}
        {!here.length && !githubShown.length && !gone.length && asked && !githubTrouble && <Empty>No repos match.</Empty>}
      </div>
    </Frame>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
