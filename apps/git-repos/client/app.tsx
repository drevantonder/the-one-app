// Git Repos: the repos someone may read, and each one opening beside them as GitHub shows a repo: its files at a branch
// with the newest commit and its README, its history, and its pull requests, which whoever pushes merges or
// closes. A file and a commit open in the repo's place, with Back.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { marked } from "marked"
import DOMPurify from "dompurify"
import "./app.css"
import { Button, call, changed, confirm, createListCollection, Empty, Field, go, Icon, Input, List, Menu, Problem, Row, Screen, Section, Select, share, Sheet, Skeleton, Split, Tabs, toast } from "ui"

/** A commit, as the tools answer it. */
interface Commit {
  sha: string
  subject: string
  author: string
  at: number
}

/** A pull request. */
interface PullRequest {
  name: string
  title: string
  state: "open" | "merged" | "closed"
  by: string
  into: string
  commits: unknown[]
  at: number
}

/** Where in the open repo: a view, and a folder, a file or a commit. */
interface At {
  tab: string
  path?: string
  file?: string
  commit?: string
}

const json = <T,>(tool: string, inputs?: object, options?: { subject?: string }): Promise<T> => call(tool, inputs, options).then(JSON.parse)
const repos = createMemo(() => json<{ repos: { repo: string; whose: string; pushed?: number }[] }>("git-repos_list").then(({ repos }) => repos))
changed(() => refresh(repos))

/// The repo open beside the list, by its name, and where in it: a view, and a folder, a file or a commit.
const [open, setOpen] = createSignal<string>()
const [at, setAt] = createSignal<At>({ tab: "code", path: "" })
const [ref, setRef] = createSignal<string>()
const [adding, setAdding] = createSignal(false)

/// Who something is, as its name's last part: users/alice is alice.
const short = (name: unknown) => String(name ?? "").split("/").pop()
const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was: "2 hours ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -1) return "just now"
  if (minutes > -60) return ago.format(minutes, "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  if (minutes > -60 * 24 * 60) return ago.format(Math.round(minutes / (24 * 60)), "day")
  return new Date(at).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })
}
const units = ["bytes", "KB", "MB", "GB"]
function size(bytes: number) {
  let n = bytes, unit = 0
  while (n >= 1000 && unit < units.length - 1) (n /= 1000), unit++
  return `${n >= 100 || unit === 0 ? Math.round(n) : n.toFixed(1)} ${units[unit]}`
}
const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`

/// Phosphor's folder, which the UI library doesn't have.
const Folder = () => (
  <svg class="ui-icon folder" viewBox="0 0 256 256" aria-hidden="true">
    <path d="M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72Z" />
  </svg>
)

/// Phosphor's git branch, which the UI library doesn't have either.
const Branchy = () => (
  <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true">
    <path d="M232,64a32,32,0,1,0-40,31v17a8,8,0,0,1-8,8H96a23.84,23.84,0,0,0-8,1.38V95a32,32,0,1,0-16,0v66a32,32,0,1,0,16,0V144a8,8,0,0,1,8-8h88a24,24,0,0,0,24-24V95A32.06,32.06,0,0,0,232,64ZM64,64A16,16,0,1,1,80,80,16,16,0,0,1,64,64ZM96,192a16,16,0,1,1-16-16A16,16,0,0,1,96,192Z" />
  </svg>
)

/// Does something that changes a repo, and says why if it couldn't. → whether it did.
async function does(tool: string, inputs: object, options?: { subject?: string }) {
  try {
    const result = await call(tool, inputs, options)
    refresh(repos)
    return result || true
  } catch (error) {
    toast(error as Error)
    return false
  }
}

/// Goes somewhere in the open repo, as a move.
const to = (where: At, back?: boolean) => go(() => setAt(where), { back })

function NewRepo() {
  const groups = createMemo(() => (adding() ? json<{ groups: { group: string; called?: string }[] }>("permissions_groups").then(({ groups }) => groups) : []))
  const whose = createMemo(() => createListCollection({ items: [{ value: "", label: "You" }, ...groups().map((group) => ({ value: group.group, label: group.called ?? short(group.group) }))] }))
  const [forWhom, setFor] = createSignal("")
  const make = async (event: SubmitEvent) => {
    event.preventDefault()
    const { name } = Object.fromEntries(new FormData(event.target as HTMLFormElement)) as Record<string, string>
    if (!(await does("git-repos_create", { name }, forWhom() ? { subject: forWhom() } : undefined))) return
    setAdding(false)
    setOpen(`${forWhom() || (await mine())}/made/git-repos/${name}`)
  }
  return (
    <Sheet heading="New Repo" open={adding()} onClose={() => setAdding(false)}>
      <form onSubmit={make}>
        <Field label="Name" hint="Lowercase letters, digits, - and _.">
          <Input name="name" pattern="[a-z0-9][a-z0-9_\-]*" autocapitalize="none" spellcheck={false} required autofocus />
        </Field>
        <Loading>
          <Show when={groups().length}>
            <Field label="Owner">
              <Select.Root collection={whose()} value={[forWhom()]} onValueChange={({ value }) => setFor(value[0]!)} />
            </Field>
          </Show>
        </Loading>
        <Button type="submit">Create</Button>
      </form>
    </Sheet>
  )
}

/// Who's asking, for the name of a repo they just made.
const mine = async () => (await json<{ people: { who: string }[] }>("sign-in_people")).people[0].who

function GitRepos() {
  return (
    <Screen heading="Git Repos" action={<Button onClick={() => setAdding(true)}>New</Button>}>
      <Show when={repos().length} fallback={<Empty>No repos yet.</Empty>}>
        <List>
          <For each={repos()}>
            {(repo) => (
              <Row
                heading={short(repo.repo)}
                text={`${short(repo.whose)} · ${repo.pushed ? `Pushed ${since(repo.pushed)}` : "Empty"}`}
                current={repo.repo === open()}
                onClick={() => go(() => (setAt({ tab: "code", path: "" }), setRef(), setOpen(repo.repo)))}
              />
            )}
          </For>
        </List>
      </Show>
    </Screen>
  )
}

/// A repo, as GitHub's page has it: what it is and how to clone it, then its code, history and pull_requests.
/** A repo, as `git-repos_branches` answers it. */
interface About {
  head: string
  clone: string
  whose: string
  owns: boolean
  pushes: boolean
  branches: { branch: string }[]
}

function Repo(props: { repo: string }) {
  const about = createMemo(() => json<About>("git-repos_branches", { repo: props.repo }))
  const branch = () => ref() ?? about().head
  const clone = async () => (await share(`git clone ${about().clone}`)) === "copied" && toast("Copied the clone command.")
  const remove = async () => {
    if (!(await confirm({ heading: `Remove ${short(props.repo)}?`, text: "Nobody can read, clone or push to it until you restore it.", action: "Remove", destructive: true }))) return
    if (await does("git-repos_remove", { repo: props.repo })) setOpen()
  }
  return (
    <Screen
      heading={short(props.repo)}
      back
      action={
        <div class="actions">
          <Button onClick={clone}>Clone</Button>
          <Show when={about().owns}>
            <Menu.Root onSelect={({ value }) => value === "remove" && remove()}>
              <Menu.Trigger />
              <Menu.Positioner>
                <Menu.Content>
                  <Menu.Item value="remove" destructive>
                    Remove Repo
                  </Menu.Item>
                </Menu.Content>
              </Menu.Positioner>
            </Menu.Root>
          </Show>
        </div>
      }
    >
      <p class="whose">
        {short(about().whose)} · {about().pushes ? "You push to it" : "You read it"}
      </p>
      <Tabs.Root value={at().tab} onValueChange={({ value }) => setAt({ tab: value!, path: "" })}>
        <Tabs.List>
          <Tabs.Trigger value="code">Code</Tabs.Trigger>
          <Tabs.Trigger value="history">History</Tabs.Trigger>
          <Tabs.Trigger value="pull_requests">Pull Requests</Tabs.Trigger>
          <Tabs.Indicator />
        </Tabs.List>
        <Tabs.Content value="code">
          <Loading fallback={<Skeleton rows={5} />}>
            <Code repo={props.repo} about={about()} branch={branch()} />
          </Loading>
        </Tabs.Content>
        <Tabs.Content value="history">
          <Loading fallback={<Skeleton rows={6} />}>
            <History repo={props.repo} branch={branch()} />
          </Loading>
        </Tabs.Content>
        <Tabs.Content value="pull_requests">
          <Loading fallback={<Skeleton rows={3} />}>
            <PullRequests repo={props.repo} pushes={about().pushes} />
          </Loading>
        </Tabs.Content>
      </Tabs.Root>
    </Screen>
  )
}

/// Where a folder or file is, each part of its path going back up to it.
function Crumbs(props: { repo: string; path: string; folder?: boolean }) {
  const parts = () => props.path.split("/").filter(Boolean)
  return (
    <nav class="crumbs" aria-label="Path">
      <button class="crumb" onClick={() => to({ tab: "code", path: "" }, true)}>
        {short(props.repo)}
      </button>
      <For each={parts()}>
        {(part, index) => (
          <>
            <span class="slash">/</span>
            <Show when={index() < parts().length - 1 || props.folder} fallback={<span class="here">{part}</span>}>
              <button class="crumb" onClick={() => to({ tab: "code", path: parts().slice(0, index() + 1).join("/") }, true)}>
                {part}
              </button>
            </Show>
          </>
        )}
      </For>
    </nav>
  )
}

function Branch(props: { about: About; branch: string }) {
  const branches = () => createListCollection({ items: props.about.branches.map((branch) => ({ value: branch.branch, label: branch.branch })) })
  return (
    <Show when={props.about.branches.length > 1} fallback={<span class="branch"><Branchy />{props.branch}</span>}>
      <Select.Root collection={branches()} value={[props.branch]} onValueChange={({ value }) => setRef(value[0])} aria-label="Branch" />
    </Show>
  )
}

/** A folder at a branch, as `git-repos_tree` answers it. */
interface Tree {
  empty?: boolean
  commit?: Commit
  commits: number
  entries: { name: string; type: "tree" | "blob"; size?: number }[]
  readme?: { name: string; text: string }
}

function Code(props: { repo: string; about: About; branch: string }) {
  const tree = createMemo(() => (at().file ? null : json<Tree>("git-repos_tree", { repo: props.repo, ref: props.branch, path: at().path })))
  return (
    <Show when={!at().file} fallback={<File repo={props.repo} branch={props.branch} path={at().file!} />}>
      <Show when={!tree()!.empty} fallback={<Empty>Nothing's been pushed yet. Clone it, commit and push.</Empty>}>
        <div class="bar">
          <Branch about={props.about} branch={props.branch} />
          <Show when={at().path}>
            <Crumbs repo={props.repo} path={at().path!} folder />
          </Show>
        </div>
        <div class="box">
          <Show when={tree()!.commit}>
            {(commit) => (
              <button class="latest" onClick={() => to({ tab: "history", commit: commit().sha })}>
                <span class="who">{commit().author}</span>
                <span class="subject">{commit().subject}</span>
                <span class="when">
                  {commit().sha.slice(0, 7)} · {since(commit().at)}
                </span>
              </button>
            )}
          </Show>
          <ul class="files">
            <For each={tree()!.entries}>
              {(entry) => (
                <li>
                  <button class="file" onClick={() => to(entry.type === "tree" ? { tab: "code", path: [at().path, entry.name].filter(Boolean).join("/") } : { tab: "code", path: at().path, file: [at().path, entry.name].filter(Boolean).join("/") })}>
                    {entry.type === "tree" ? <Folder /> : <Icon name="file" />}
                    <span class="name">{entry.name}</span>
                    <span class="size">{entry.size != null ? size(entry.size) : ""}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
        <Show when={!at().path}>
          <p class="count">{plural(tree()!.commits, "commit")} on {props.branch}</p>
        </Show>
        <Show when={tree()!.readme}>{(readme) => <Readme readme={readme()} />}</Show>
      </Show>
    </Show>
  )
}

/// A README as GitHub shows it: its Markdown drawn, with nothing in it that runs.
function Readme(props: { readme: { name: string; text: string } }) {
  const html = () => (/\.(md|markdown)$/i.test(props.readme.name) ? DOMPurify.sanitize(marked.parse(props.readme.text) as string) : null)
  return (
    <article class="box readme">
      <h2 class="readme-name">{props.readme.name}</h2>
      <Show when={html()} fallback={<pre class="plain">{props.readme.text}</pre>}>
        <div class="markdown" innerHTML={html()!} />
      </Show>
    </article>
  )
}

function File(props: { repo: string; branch: string; path: string }) {
  const file = createMemo(() => json<{ text?: string | null; size: number; binary?: boolean }>("git-repos_file", { repo: props.repo, ref: props.branch, path: props.path }))
  const lines = () => file().text!.replace(/\n$/, "").split("\n")
  return (
    <>
      <div class="bar">
        <span class="branch">
          <Branchy />
          {props.branch}
        </span>
        <Crumbs repo={props.repo} path={props.path} />
      </div>
      <div class="box">
        <p class="head">
          {file().text != null ? `${plural(lines().length, "line")} · ` : ""}
          {size(file().size)}
        </p>
        <Show when={file().text != null} fallback={<Empty>{file().binary ? "It isn't text. Clone the repo to open it." : "It's too large to show. Clone the repo to open it."}</Empty>}>
          <div class="code">
            <ol>
              <For each={lines()}>{(line) => <li>{line || " "}</li>}</For>
            </ol>
          </div>
        </Show>
      </div>
    </>
  )
}

function History(props: { repo: string; branch: string }) {
  const [pages, setPages] = createSignal(1)
  const log = createMemo(() => Promise.all(Array.from({ length: pages() }, (_, page) => json<{ commits: Commit[]; more?: boolean }>("git-repos_log", { repo: props.repo, ref: props.branch, page }))))
  const commits = () => log().flatMap((page) => page.commits)
  return (
    <Show when={!at().commit} fallback={<CommitScreen repo={props.repo} sha={at().commit!} />}>
      <Show when={commits().length} fallback={<Empty>No commits yet.</Empty>}>
        <List>
          <For each={commits()}>{(commit) => <Row heading={commit.subject} text={`${commit.author} · ${since(commit.at)} · ${commit.sha.slice(0, 7)}`} onClick={() => to({ tab: "history", commit: commit.sha })} />}</For>
        </List>
        <Show when={log().at(-1)!.more}>
          <Button variant="outline" onClick={() => setPages(pages() + 1)}>
            Older
          </Button>
        </Show>
      </Show>
    </Show>
  )
}

/// A commit: its message and what it changed, with each line of the diff coloured as added or removed.
function CommitScreen(props: { repo: string; sha: string }) {
  const commit = createMemo(() => json<Commit & { message: string; diff: string }>("git-repos_commit", { repo: props.repo, sha: props.sha }))
  const kind = (line: string) => (/^(\+\+\+|---|diff |index )/.test(line) ? "meta" : line[0] === "+" ? "add" : line[0] === "-" ? "del" : line.startsWith("@@") ? "hunk" : "")
  return (
    <>
      <div class="bar">
        <Button variant="ghost" onClick={() => to({ tab: "history" }, true)}>
          <Icon name="caret-left" along /> History
        </Button>
      </div>
      <div class="box">
        <h2 class="commit-subject">{commit().message.split("\n")[0]}</h2>
        <Show when={commit().message.split("\n").slice(1).join("\n").trim()}>{(more) => <pre class="plain">{more()}</pre>}</Show>
        <p class="head">
          {commit().author} committed {since(commit().at)} · {commit().sha.slice(0, 7)}
        </p>
      </div>
      <div class="box">
        <div class="code">
          <div class="diff">
            <For each={commit().diff.replace(/^---\n/, "").split("\n")}>{(line) => <div class={kind(line)}>{line || " "}</div>}</For>
          </div>
        </div>
      </div>
    </>
  )
}

function PullRequests(props: { repo: string; pushes: boolean }) {
  const pullRequests = createMemo(() => json<{ pull_requests: PullRequest[] }>("git-repos_pull_requests", { repo: props.repo }).then(({ pull_requests }) => pull_requests))
  const open = () => pullRequests().filter((one) => one.state === "open")
  const closed = () => pullRequests().filter((one) => one.state !== "open")
  const merge = (one: PullRequest) => does("git-repos_merge", { repo: props.repo, pull_request: one.name }).then((said): unknown => said && (toast(`Merged into ${one.into}.`), refresh(pullRequests)))
  const close = async (one: PullRequest) => (await confirm({ heading: `Close ${one.title}?`, text: "It stays listed as closed.", action: "Close" })) && (await does("git-repos_close", { repo: props.repo, pull_request: one.name })) && refresh(pullRequests)
  const line = (one: PullRequest) => `${short(one.by)} · into ${one.into} · ${plural(one.commits.length, "commit")} · ${since(one.at)}`
  return (
    <Show when={pullRequests().length} fallback={<Empty>No pull requests.</Empty>}>
      <Show when={open().length}>
        <Section heading="Open">
          <List>
            <For each={open()}>
              {(one) => (
                <Row
                  heading={one.title}
                  text={line(one)}
                  end={
                    <Show when={props.pushes}>
                      <div class="actions">
                        <Button variant="secondary" onClick={() => merge(one)}>
                          Merge
                        </Button>
                        <Button variant="ghost" onClick={() => close(one)}>
                          Close
                        </Button>
                      </div>
                    </Show>
                  }
                />
              )}
            </For>
          </List>
        </Section>
      </Show>
      <Show when={closed().length}>
        <Section heading="Closed">
          <List>
            <For each={closed()}>{(one) => <Row heading={one.title} text={`${one.state === "merged" ? "Merged" : "Closed"} · ${line(one)}`} />}</For>
          </List>
        </Section>
      </Show>
    </Show>
  )
}

function App() {
  return (
    <>
      <Split
        open={!!open()}
        onClose={() => setOpen()}
        detail={
          <Show when={open()} fallback={<Show when={repos().length}><Empty>Pick a repo to see its code.</Empty></Show>}>
            {(repo) => (
              <Errored fallback={(error) => <Problem heading="Can't show this repo" text={error() as Error} />}>
                <Loading fallback={<Skeleton rows={6} />}>
                  <Repo repo={repo()} />
                </Loading>
              </Errored>
            )}
          </Show>
        }
      >
        <GitRepos />
      </Split>
      <NewRepo />
    </>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your repos" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={4} />}>
        <App />
      </Loading>
    </Errored>
  ),
  document.body,
)
