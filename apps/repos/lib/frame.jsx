// What a repo's pages share: its name and tabs, the path bar, and the newest commit's card.
import { Avatar, Badge, Button, Icon, Tabs, TabsList, TabsTrigger, useCall } from "@one/ui"
import { RepoIcon, SmallCopy } from "./parts.jsx"
import { ago, go, isGithub, refOf, short, useWide } from "./use.jsx"

const SCREENS = { code: "repo", issues: "issues", settings: "repo_settings" }

/// `info` is `{ open_issues, yours }`, from an issues call the page has made; else it's asked for.
export function RepoTabs({ repo, on, info }) {
  const wide = useWide()
  const asked = useCall("issues", info || !wide ? null : { repo, page: 0 })
  if (!wide) return null
  const d = info || asked.data
  const parts = repo.split("/")
  const name = parts.pop()
  const owner = parts.join("/")
  const settings = d?.yours && !isGithub(repo)
  return (
    <div className="mb-4 flex flex-col gap-3">
      <div className="flex min-w-0 items-center gap-2 text-lg">
        <RepoIcon repo={repo} className="size-5 text-muted-foreground" />
        <span className="truncate text-muted-foreground">{owner}</span>
        <span className="text-muted-foreground">/</span>
        <span className="truncate font-semibold">{name}</span>
      </div>
      <Tabs value={on} onValueChange={(v) => v !== on && go(SCREENS[v], { repo })}>
        <TabsList variant="line" className="w-full justify-start border-b">
          <TabsTrigger value="code" className="flex-none px-3"><Icon name="code" />Code</TabsTrigger>
          <TabsTrigger value="issues" className="flex-none px-3">
            <Icon name="record" />Issues
            {!!d?.open_issues && <Badge variant="secondary">{d.open_issues.toLocaleString()}</Badge>}
          </TabsTrigger>
          {settings && <TabsTrigger value="settings" className="flex-none px-3"><Icon name="gear" />Settings</TabsTrigger>}
        </TabsList>
      </Tabs>
    </div>
  )
}

export function PathBar({ d, file = false }) {
  const parts = (d.path || "").split("/").filter(Boolean)
  const ref = refOf(d)
  const branches = () => go("branches", { repo: d.repo, path: d.path || "" })
  if (!parts.length && !file && d.branch) return <BranchButton d={d} open={branches} />
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-x-3 gap-y-2">
      <BranchButton d={d} open={branches} />
      {!d.branch && <Button variant="outline" size="sm" onClick={() => go("repo", { repo: d.repo })}>Back to Main</Button>}
      <nav aria-label="Path" className="flex min-w-0 flex-wrap items-center gap-x-1.5 text-sm">
        <Crumb onClick={() => go("repo", { repo: d.repo, ref })}>{d.repo}</Crumb>
        {parts.map((p, k) => {
          const last = k === parts.length - 1
          return (
            <span key={k} className="flex items-center gap-1.5">
              <span className="text-muted-foreground">/</span>
              {last ? <strong className="font-semibold">{p}</strong> : <Crumb onClick={() => go("repo", { repo: d.repo, ref, path: parts.slice(0, k + 1).join("/") })}>{p}</Crumb>}
            </span>
          )
        })}
        {file && <SmallCopy text={d.path} className="ml-1" />}
      </nav>
    </div>
  )
}

const Crumb = ({ onClick, children }) => (
  <button type="button" onClick={onClick} className="cursor-pointer rounded-sm text-muted-foreground underline-offset-4 outline-none hover:text-foreground hover:underline focus-visible:text-foreground focus-visible:underline">
    {children}
  </button>
)

function BranchButton({ d, open }) {
  return (
    <Button variant="outline" size="sm" className="self-start" onClick={open}>
      <Icon name={d.branch ? "git-branch" : "git-commit"} />
      <span className="max-w-48 truncate">{d.branch || short(d.sha)}</span>
      <Icon name="caret-down" className="size-3 text-muted-foreground" />
    </Button>
  )
}

export function CommitCard({ c, d }) {
  const wide = useWide()
  const history = () => go("history", { repo: d.repo, ref: refOf(d), path: d.path || "" })
  const when = <><code>{short(c.sha)}</code> · {ago(c.time)}</>
  const row = "flex cursor-pointer items-center gap-3 px-3 py-2.5 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50"
  return (
    <div className={`overflow-hidden rounded-xl border bg-card text-card-foreground ${wide ? "flex items-stretch divide-x" : "divide-y"}`}>
      <button type="button" onClick={() => go("commit", { repo: d.repo, sha: c.sha })} className={`${row} min-w-0 ${wide ? "flex-1" : "w-full"}`}>
        <Avatar name={c.author} size={20} />
        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span className="shrink-0 font-medium">{c.author}</span>
            <span className="min-w-0 flex-1 truncate text-muted-foreground">{c.subject}</span>
          </span>
          {!wide && <span className="block text-xs text-muted-foreground">{when}</span>}
        </span>
        {wide && <span className="shrink-0 whitespace-nowrap text-xs text-muted-foreground">{when}</span>}
      </button>
      <button type="button" onClick={history} className={`${row} gap-2 font-medium ${wide ? "shrink-0" : "w-full"}`}>
        <Icon name="clock-counter-clockwise" />History
        {!wide && <Icon name="caret-right" className="ml-auto text-muted-foreground" />}
      </button>
    </div>
  )
}
