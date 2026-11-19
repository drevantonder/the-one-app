// Git Repos: the repos someone may read, and each one opening beside them as GitHub shows a repo: its files at a branch
// with the newest commit and its README, its history, and its pull requests, which whoever pushes merges or
// closes. A file and a commit open in the repo's place, with Back.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { marked, type Token, type Tokens } from "marked"
import { Box, Button, call, changed, confirm, createListCollection, Empty, Field, go, Icon, Image, Input, List, Menu, Problem, render, Row, Screen, Section, Select, share, Sheet, Skeleton, Split, Tabs, Text, toast } from "ui"
import type { Style } from "ui"

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
const short = (name: unknown) => String(name ?? "").split("/").pop()!
/// How long ago `at` was, as people say it: "2 hours ago", or its date once it's long ago. (Not Intl's: a
/// strict page's JS has none on a native host.)
const months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]
function since(at: number) {
  const minutes = Math.round((Date.now() - at) / 60_000)
  const ago = (count: number, unit: string) => `${plural(count, unit)} ago`
  if (minutes < 1) return "just now"
  if (minutes < 60) return ago(minutes, "minute")
  if (minutes < 48 * 60) return ago(Math.round(minutes / 60), "hour")
  if (minutes < 60 * 24 * 60) return ago(Math.round(minutes / (24 * 60)), "day")
  const date = new Date(at)
  return `${date.getDate()} ${months[date.getMonth()]} ${date.getFullYear()}`
}
const units = ["bytes", "KB", "MB", "GB"]
function size(bytes: number) {
  let n = bytes, unit = 0
  while (n >= 1000 && unit < units.length - 1) (n /= 1000), unit++
  return `${n >= 100 || unit === 0 ? Math.round(n) : n.toFixed(1)} ${units[unit]}`
}
const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`

// TODO: the UI library's Icon has no folder or git-branch (Phosphor's, drawn here from their SVGs in a mid grey, which
// reads on light and dark), and an image can't take a colour of the tokens: restore them as Icon names in the strict contract.
const svg = (markup: string) => `data:image/svg+xml,${encodeURIComponent(markup)}`
const FOLDER = svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="#8a8a8a"><path d="M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72Z"/></svg>')
const BRANCH = svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="#8a8a8a"><path d="M232,64a32,32,0,1,0-40,31v17a8,8,0,0,1-8,8H96a23.84,23.84,0,0,0-8,1.38V95a32,32,0,1,0-16,0v66a32,32,0,1,0,16,0V144a8,8,0,0,1,8-8h88a24,24,0,0,0,24-24V95A32.06,32.06,0,0,0,232,64ZM64,64A16,16,0,1,1,80,80,16,16,0,0,1,64,64ZM96,192a16,16,0,1,1-16-16A16,16,0,0,1,96,192Z"/></svg>')
const Folder = () => <Image src={FOLDER} alt="" aria-hidden="true" style={{ width: "icon", height: "icon" }} />
const Branchy = () => <Image src={BRANCH} alt="" aria-hidden="true" style={{ width: "icon", height: "icon" }} />

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

// What a repo's page is laid out with: a bordered card the readable width, and a bar of what's where.
const card: Style = { maxWidth: "readable", borderWidth: 1, borderRadius: "md", overflow: "hidden", backgroundColor: "card" }
const bar: Style = { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: "gutter", maxWidth: "readable" }
const head: Style = { paddingBlock: 10, paddingInline: 16, borderBottomWidth: 1, backgroundColor: "muted" }

/// The name a new repo may have.
const NAME = /^[a-z0-9][a-z0-9_-]*$/

function NewRepo() {
  const groups = createMemo(() => (adding() ? json<{ groups: { group: string; called?: string }[] }>("permissions_groups").then(({ groups }) => groups) : []))
  const whose = createMemo(() => createListCollection({ items: [{ value: "", label: "You" }, ...groups().map((group) => ({ value: group.group, label: group.called ?? short(group.group) }))] }))
  const [forWhom, setFor] = createSignal("")
  const [name, setName] = createSignal("")
  const [sent, setSent] = createSignal(false)
  const make = async () => {
    setSent(true)
    if (!NAME.test(name())) return
    if (!(await does("git-repos_create", { name: name() }, forWhom() ? { subject: forWhom() } : undefined))) return
    setAdding(false)
    setOpen(`${forWhom() || (await mine())}/made/git-repos/${name()}`)
  }
  return (
    <Sheet heading="New Repo" open={adding()} onClose={() => setAdding(false)}>
      <Field label="Name" hint="Lowercase letters, digits, - and _." error={sent() && !NAME.test(name()) ? "Use lowercase letters, digits, - and _, starting with a letter or digit." : undefined}>
        <Input required autofocus autocapitalize="none" autocorrect={false} spellcheck={false} value={name()} onInput={(event) => setName(event.currentTarget.value)} onAction={make} />
      </Field>
      <Loading>
        <Show when={groups().length}>
          <Field label="Owner">
            <Select.Root collection={whose()} value={[forWhom()]} onValueChange={({ value }) => setFor(value[0]!)} />
          </Field>
        </Show>
      </Loading>
      <Button onClick={make}>Create</Button>
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
        <Box style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
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
        </Box>
      }
    >
      <Text style={{ font: "footnote", color: "muted-foreground" }}>
        {short(about().whose)} · {about().pushes ? "You push to it" : "You read it"}
      </Text>
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
  const crumb = (name: string, where: At) => (
    <Box role="button" onClick={() => to(where, true)}>
      <Text style={{ font: "label" }}>{name}</Text>
    </Box>
  )
  return (
    <Box role="navigation" aria-label="Path" style={{ flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: 4 }}>
      {crumb(short(props.repo), { tab: "code", path: "" })}
      <For each={parts()}>
        {(part, index) => (
          <>
            <Text aria-hidden="true" style={{ color: "muted-foreground" }}>/</Text>
            <Show when={index() < parts().length - 1 || props.folder} fallback={<Text style={{ font: "label" }}>{part}</Text>}>
              {crumb(part, { tab: "code", path: parts().slice(0, index() + 1).join("/") })}
            </Show>
          </>
        )}
      </For>
    </Box>
  )
}

function Branch(props: { about: About; branch: string }) {
  const name = () => <Text style={{ font: "label" }}>{props.branch}</Text>
  return (
    <Show
      when={props.about.branches.length > 1}
      fallback={
        <Box style={{ flexDirection: "row", alignItems: "center", gap: 6, paddingBlock: 6, paddingInline: 12, borderWidth: 1, borderRadius: "md" }}>
          <Branchy />
          {name()}
        </Box>
      }
    >
      <Menu.Root onSelect={({ value }) => setRef(value)}>
        <Menu.Trigger variant="outline" aria-label={`Branch: ${props.branch}`}>
          <Branchy /> {props.branch} <Icon name="caret-up-down" />
        </Menu.Trigger>
        <Menu.Positioner>
          <Menu.Content>
            <For each={props.about.branches}>{(one) => <Menu.Item value={one.branch}>{one.branch}</Menu.Item>}</For>
          </Menu.Content>
        </Menu.Positioner>
      </Menu.Root>
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
        <Box style={{ gap: "gutter", paddingTop: "gutter" }}>
          <Box style={bar}>
            <Branch about={props.about} branch={props.branch} />
            <Show when={at().path}>
              <Crumbs repo={props.repo} path={at().path!} folder />
            </Show>
          </Box>
          <Box style={card}>
            <Show when={tree()!.commit}>
              {(commit) => (
                <Box role="button" onClick={() => to({ tab: "history", commit: commit().sha })} style={{ ...head, flexDirection: "row", alignItems: "baseline", gap: 8 }}>
                  <Text style={{ font: "label", lineClamp: 1 }}>{commit().author}</Text>
                  <Text style={{ font: "footnote", flexGrow: 1, flexShrink: 1, color: "muted-foreground", lineClamp: 1 }}>{commit().subject}</Text>
                  <Text style={{ font: "footnote", color: "muted-foreground", lineClamp: 1 }}>
                    {commit().sha.slice(0, 7)} · {since(commit().at)}
                  </Text>
                </Box>
              )}
            </Show>
            <Box role="list">
              <For each={tree()!.entries}>
                {(entry, index) => (
                  <Box role="listitem" style={{ borderTopWidth: index() ? 1 : 0 }}>
                    <Box
                      role="button"
                      onClick={() => to(entry.type === "tree" ? { tab: "code", path: [at().path, entry.name].filter(Boolean).join("/") } : { tab: "code", path: at().path, file: [at().path, entry.name].filter(Boolean).join("/") })}
                      style={{ flexDirection: "row", alignItems: "center", gap: 12, paddingBlock: 10, paddingInline: 16 }}
                    >
                      <Box style={{ color: "muted-foreground" }}>
                        {entry.type === "tree" ? <Folder /> : <Icon name="file" />}
                      </Box>
                      <Text style={{ flexGrow: 1, flexShrink: 1, lineClamp: 1 }}>{entry.name}</Text>
                      <Text style={{ font: "footnote", color: "muted-foreground" }}>{entry.size != null ? size(entry.size) : ""}</Text>
                    </Box>
                  </Box>
                )}
              </For>
            </Box>
          </Box>
          <Show when={!at().path}>
            <Text style={{ font: "footnote", color: "muted-foreground" }}>
              {plural(tree()!.commits, "commit")} on {props.branch}
            </Text>
          </Show>
          <Show when={tree()!.readme}>{(readme) => <Readme readme={readme()} />}</Show>
        </Box>
      </Show>
    </Show>
  )
}

/// A README as GitHub shows it: its Markdown drawn from what marked makes of it, so nothing in it runs.
function Readme(props: { readme: { name: string; text: string } }) {
  const markdown = () => /\.(md|markdown)$/i.test(props.readme.name)
  return (
    <Box style={card}>
      <Text role="heading" aria-level={2} style={{ ...head, font: "label", backgroundColor: "card" }}>
        {props.readme.name}
      </Text>
      <Show when={markdown()} fallback={<Plain text={props.readme.text} />}>
        <Box style={{ gap: 12, paddingBlock: 12, paddingInline: 24 }}>
          <Blocks tokens={marked.lexer(props.readme.text)} />
        </Box>
      </Show>
    </Box>
  )
}

/// Text as it was typed, its lines kept.
const Plain = (props: { text: string }) => <Lines lines={props.text.split("\n").map((text) => ({ text }))} />

// Marked leaves what's typed as it was: `&amp;` is still `&amp;`.
const entities: Record<string, string> = { "&amp;": "&", "&lt;": "<", "&gt;": ">", "&quot;": '"', "&#39;": "'" }
const plain = (text: string) => text.replace(/&(amp|lt|gt|quot|#39);/g, (entity) => entities[entity])

function Blocks(props: { tokens: Token[] }) {
  return <For each={props.tokens}>{(token) => <Block token={token} />}</For>
}

/// One block of Markdown. A host draws a few type styles and no weights or italics, so a heading is title or label,
/// a strong is label, and a link is words like any other.
function Block(props: { token: Token }) {
  const token = props.token
  switch (token.type) {
    case "heading": {
      const { depth, tokens } = token as Tokens.Heading
      return (
        <Box style={{ paddingTop: 8, paddingBottom: depth <= 2 ? 4 : 0, borderBottomWidth: depth <= 2 ? 1 : 0 }}>
          <Text role="heading" aria-level={depth} style={{ font: depth <= 2 ? "title" : "label" }}>
            <Inline tokens={tokens} />
          </Text>
        </Box>
      )
    }
    case "paragraph":
    case "text":
      return (
        <Text>
          <Inline tokens={(token as Tokens.Paragraph).tokens ?? [token]} />
        </Text>
      )
    case "list": {
      const { ordered, start, items } = token as Tokens.List
      return (
        <Box role="list" style={{ gap: 4 }}>
          <For each={items}>
            {(item, index) => (
              <Box role="listitem" style={{ flexDirection: "row", gap: 8 }}>
                <Text aria-hidden="true" style={{ width: 24, flexShrink: 0, textAlign: "end", color: "muted-foreground" }}>
                  {item.task ? (item.checked ? "☑" : "☐") : ordered ? `${(Number(start) || 1) + index()}.` : "•"}
                </Text>
                <Box style={{ flexGrow: 1, flexShrink: 1, gap: 4 }}>
                  <Blocks tokens={item.tokens} />
                </Box>
              </Box>
            )}
          </For>
        </Box>
      )
    }
    case "code":
      return (
        <Box style={{ borderRadius: "sm", backgroundColor: "muted" }}>
          <Plain text={(token as Tokens.Code).text} />
        </Box>
      )
    case "blockquote":
      return (
        <Box style={{ gap: 12, paddingLeft: 12, borderLeftWidth: 1, color: "muted-foreground" }}>
          <Blocks tokens={(token as Tokens.Blockquote).tokens} />
        </Box>
      )
    case "hr":
      return <Box role="separator" style={{ borderBottomWidth: 1 }} />
    case "table": {
      const { header, rows } = token as Tokens.Table
      const row = (cells: Tokens.TableCell[], font: "label" | "body") => (
        <Box role="listitem" style={{ flexDirection: "row", gap: 16, paddingBlock: 6, borderBottomWidth: 1 }}>
          <For each={cells}>
            {(cell) => (
              <Text style={{ font, flexGrow: 1, flexBasis: 0 }}>
                <Inline tokens={cell.tokens} />
              </Text>
            )}
          </For>
        </Box>
      )
      return (
        <Box role="list">
          {row(header, "label")}
          <For each={rows}>{(cells) => row(cells, "body")}</For>
        </Box>
      )
    }
    default:
      return null
  }
}

function Inline(props: { tokens: Token[] }) {
  return <For each={props.tokens}>{(token) => <Span token={token} />}</For>
}

function Span(props: { token: Token }) {
  const token = props.token
  switch (token.type) {
    case "text":
    case "escape": {
      const { tokens, text } = token as Tokens.Text
      return tokens ? <Inline tokens={tokens} /> : <>{plain(text)}</>
    }
    case "strong":
    case "link":
      return (
        <Text style={{ font: "label" }}>
          <Inline tokens={(token as Tokens.Strong).tokens} />
        </Text>
      )
    case "em":
    case "del":
      return <Inline tokens={(token as Tokens.Em).tokens} />
    case "codespan":
      return <Text style={{ backgroundColor: "muted", borderRadius: "sm" }}> {plain((token as Tokens.Codespan).text)} </Text>
    case "br":
      return <>{"\n"}</>
    case "image":
      return <>{(token as Tokens.Image).text}</>
    default:
      return null
  }
}

/// A file, its lines numbered.
function File(props: { repo: string; branch: string; path: string }) {
  const file = createMemo(() => json<{ text?: string | null; size: number; binary?: boolean }>("git-repos_file", { repo: props.repo, ref: props.branch, path: props.path }))
  const lines = () => file().text!.replace(/\n$/, "").split("\n")
  return (
    <Box style={{ gap: "gutter", paddingTop: "gutter" }}>
      <Box style={bar}>
        <Box style={{ flexDirection: "row", alignItems: "center", gap: 6, paddingBlock: 6, paddingInline: 12, borderWidth: 1, borderRadius: "md" }}>
          <Branchy />
          <Text style={{ font: "label" }}>{props.branch}</Text>
        </Box>
        <Crumbs repo={props.repo} path={props.path} />
      </Box>
      <Box style={card}>
        <Text style={{ ...head, font: "footnote", color: "muted-foreground" }}>
          {file().text != null ? `${plural(lines().length, "line")} · ` : ""}
          {size(file().size)}
        </Text>
        <Show when={file().text != null} fallback={<Empty>{file().binary ? "It isn't text. Clone the repo to open it." : "It's too large to show. Clone the repo to open it."}</Empty>}>
          <Lines lines={lines().map((text, index) => ({ text, number: index + 1 }))} />
        </Show>
      </Box>
    </Box>
  )
}

// TODO: code is in a proportional font that wraps, its indentation kept as no-break spaces, its added and removed lines
// tinted by their text only: restore the mono type style, no wrapping with sideways scrolling, and the diff tints
// (`diff-add`, `diff-del`) in the strict contract.
const kept = (text: string) => text.replace(/^[ \t]+/, (space) => space.replace(/\t/g, "    ").replace(/ /g, "\u00a0")) || "\u00a0"

/// Lines of code, each with its number where it has one.
function Lines(props: { lines: { text: string; number?: number; removed?: boolean; muted?: boolean }[] }) {
  return (
    <Box style={{ paddingBlock: 8 }}>
      <For each={props.lines}>
        {(line) => (
          <Box style={{ flexDirection: "row", gap: 16, paddingInline: 16 }}>
            <Show when={line.number}>
              <Text style={{ font: "footnote", width: 32, flexShrink: 0, textAlign: "end", color: "muted-foreground" }}>{line.number}</Text>
            </Show>
            <Text style={{ font: "footnote", flexShrink: 1, color: line.removed ? "destructive" : line.muted ? "muted-foreground" : undefined }}>{kept(line.text)}</Text>
          </Box>
        )}
      </For>
    </Box>
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
  const line = (text: string) => {
    if (/^(\+\+\+|---|diff |index )/.test(text) || text.startsWith("@@")) return { text, muted: true }
    return { text, removed: text[0] === "-" }
  }
  const more = () => commit().message.split("\n").slice(1).join("\n").trim()
  return (
    <Box style={{ gap: "gutter", paddingTop: "gutter" }}>
      <Box style={bar}>
        <Button variant="ghost" onClick={() => to({ tab: "history" }, true)}>
          <Icon name="caret-left" along /> History
        </Button>
      </Box>
      <Box style={card}>
        <Text role="heading" aria-level={2} style={{ font: "label", padding: 16, paddingBottom: more() ? 8 : 16 }}>
          {commit().message.split("\n")[0]}
        </Text>
        <Show when={more()}>{(more) => <Plain text={more()} />}</Show>
        <Text style={{ ...head, font: "footnote", color: "muted-foreground", borderBottomWidth: 0, borderTopWidth: 1 }}>
          {commit().author} committed {since(commit().at)} · {commit().sha.slice(0, 7)}
        </Text>
      </Box>
      <Box style={card}>
        <Lines lines={commit().diff.replace(/^---\n/, "").split("\n").map(line)} />
      </Box>
    </Box>
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
                      <Box style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                        <Button variant="secondary" onClick={() => merge(one)}>
                          Merge
                        </Button>
                        <Button variant="ghost" onClick={() => close(one)}>
                          Close
                        </Button>
                      </Box>
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

render(() => (
  <Errored fallback={(error) => <Problem heading="Can't show your repos" text={error() as Error} />}>
    <Loading fallback={<Skeleton rows={4} />}>
      <App />
    </Loading>
  </Errored>
))
