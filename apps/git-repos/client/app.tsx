// Git Repos: the repos someone may read, and each one opening beside them as GitHub shows a repo: its files at a branch
// with the newest commit and its README, its history, and its pull requests, which whoever pushes merges or
// closes. A file and a commit open in the repo's place, with Back.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { marked, type Token, type Tokens } from "marked"
import { Button, call, changed, confirm, Empty, Field, go, Icon, Image, Input, List, Menu, Pressable, Problem, render, Row, ScrollView, Screen, Section, Select, share, Sheet, since, size, Skeleton, Split, styles, Tabs, Text, toast, tokens, View } from "ui"

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
const plural = (count: number, one: string) => `${count} ${one}${count === 1 ? "" : "s"}`

// TODO: the UI library's Icon has no folder or git-branch (Phosphor's, drawn here from their SVGs in a mid grey, which
// reads on light and dark), and an image can't take a colour of the tokens: restore them as Icon names in the UI library's contract.
const svg = (markup: string) => `data:image/svg+xml,${encodeURIComponent(markup)}`
const FOLDER = svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="#8a8a8a"><path d="M216,72H131.31L104,44.69A15.86,15.86,0,0,0,92.69,40H40A16,16,0,0,0,24,56V200.62A15.4,15.4,0,0,0,39.38,216H216.89A15.13,15.13,0,0,0,232,200.89V88A16,16,0,0,0,216,72Z"/></svg>')
const BRANCH = svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" fill="#8a8a8a"><path d="M232,64a32,32,0,1,0-40,31v17a8,8,0,0,1-8,8H96a23.84,23.84,0,0,0-8,1.38V95a32,32,0,1,0-16,0v66a32,32,0,1,0,16,0V144a8,8,0,0,1,8-8h88a24,24,0,0,0,24-24V95A32.06,32.06,0,0,0,232,64ZM64,64A16,16,0,1,1,80,80,16,16,0,0,1,64,64ZM96,192a16,16,0,1,1-16-16A16,16,0,0,1,96,192Z"/></svg>')
const Folder = () => <Image src={FOLDER} alt="" aria-hidden="true" style={look.glyph} />
const Branchy = () => <Image src={BRANCH} alt="" aria-hidden="true" style={look.glyph} />

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

const look = styles({
  // What a repo's page is laid out with: a bordered card the readable width, and a bar of what's where.
  card: { maxWidth: tokens.length.readable, borderWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, borderRadius: tokens.radius.medium, overflow: "hidden", backgroundColor: tokens.colour.card },
  bar: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: tokens.length.gutter, maxWidth: tokens.length.readable },
  head: { paddingBlock: tokens.grid[2.5], paddingInline: tokens.grid[4], borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, backgroundColor: tokens.colour.quinaryLabel },
  commitHead: { flexDirection: "row", alignItems: "baseline", gap: tokens.grid[2] },
  readmeHead: { backgroundColor: tokens.colour.card },
  foot: { borderBottomWidth: 0, borderTopWidth: tokens.length.hairline },
  page: { gap: tokens.length.gutter, paddingTop: tokens.length.gutter },
  glyph: { width: tokens.length.icon, height: tokens.length.icon },
  actions: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2] },
  strong: { fontWeight: tokens.weight.medium },
  title: { font: tokens.type.title },
  footnote: { font: tokens.type.caption, color: tokens.colour.secondaryLabel },
  quiet: { color: tokens.colour.secondaryLabel },
  removed: { backgroundColor: tokens.colour.redTint },
  added: { backgroundColor: tokens.colour.greenTint },
  oneLine: { lineClamp: 1 },
  shrinks: { flexGrow: 1, flexShrink: 1 },
  crumbs: { flexDirection: "row", flexWrap: "wrap", alignItems: "center", gap: tokens.grid[1] },
  branch: { flexDirection: "row", alignItems: "center", gap: tokens.grid[1.5], paddingBlock: tokens.grid[1.5], paddingInline: tokens.grid[3], borderWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, borderRadius: tokens.radius.medium },
  between: { borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  entry: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3], paddingBlock: tokens.grid[2.5], paddingInline: tokens.grid[4] },
  markdown: { gap: tokens.grid[3], paddingBlock: tokens.grid[3], paddingInline: tokens.grid[6] },
  heading: { paddingTop: tokens.grid[2] },
  majorHeading: { paddingBottom: tokens.grid[1], borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  items: { gap: tokens.grid[1] },
  item: { flexDirection: "row", gap: tokens.grid[2] },
  bullet: { width: tokens.grid[6], flexShrink: 0, textAlign: "end", color: tokens.colour.secondaryLabel },
  itemBody: { flexGrow: 1, flexShrink: 1, gap: tokens.grid[1] },
  code: { borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel },
  quote: { gap: tokens.grid[3], paddingStart: tokens.grid[3], borderStartWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, color: tokens.colour.secondaryLabel },
  rule: { borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  tableRow: { flexDirection: "row", gap: tokens.grid[4], paddingBlock: tokens.grid[1.5], borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  cell: { flexGrow: 1, flexBasis: 0 },
  scroll: { flexDirection: "row" },
  lines: { flexGrow: 1, flexShrink: 0, paddingBlock: tokens.grid[2] },
  line: { flexDirection: "row", gap: tokens.grid[4], paddingInline: tokens.grid[4] },
  number: { width: tokens.grid[8], flexShrink: 0, textAlign: "end" },
  lineText: { flexShrink: 0, color: tokens.colour.label, fontFamily: tokens.font.mono, whiteSpace: "nowrap" },
  message: { padding: tokens.grid[4] },
  messageMore: { paddingBottom: tokens.grid[2] },
})

/// The name a new repo may have.
const NAME = /^[a-z0-9][a-z0-9_-]*$/

function NewRepo() {
  const groups = createMemo(() => (adding() ? json<{ groups: { group: string; called?: string }[] }>("permissions_groups").then(({ groups }) => groups) : []))
  const whose = createMemo(() => [{ value: "", label: "You" }, ...groups().map((group) => ({ value: group.group, label: group.called ?? short(group.group) }))])
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
        <Input required autoFocus autoCapitalize="none" autoCorrect={false} spellCheck={false} value={name()} onChangeText={setName} onSubmitEditing={make} />
      </Field>
      <Loading>
        <Show when={groups().length}>
          <Field label="Owner">
            <Select.Root items={whose()} value={forWhom()} onValueChange={setFor} />
          </Field>
        </Show>
      </Loading>
      <Button onPress={make}>Create</Button>
    </Sheet>
  )
}

/// Who's asking, for the name of a repo they just made.
const mine = async () => (await json<{ people: { who: string }[] }>("sign-in_people")).people[0].who

function GitRepos() {
  return (
    <Screen heading="Git Repos" action={<Button onPress={() => setAdding(true)}>New</Button>}>
      <Show when={repos().length} fallback={<Empty>No repos yet.</Empty>}>
        <List each={repos()}>
          {(repo) => (
            <Row
              heading={short(repo.repo)}
              text={`${short(repo.whose)} · ${repo.pushed ? `Pushed ${since(repo.pushed)}` : "Empty"}`}
              current={repo.repo === open()}
              onPress={() => go(() => (setAt({ tab: "code", path: "" }), setRef(), setOpen(repo.repo)))}
            />
          )}
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
        <View style={look.actions}>
          <Button onPress={clone}>Clone</Button>
          <Show when={about().owns}>
            <Menu.Root onSelect={(value) => value === "remove" && remove()}>
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
        </View>
      }
    >
      <Text style={look.footnote}>
        {short(about().whose)} · {about().pushes ? "You push to it" : "You read it"}
      </Text>
      <Tabs.Root value={at().tab} onValueChange={(tab) => setAt({ tab, path: "" })}>
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
    <Pressable role="button" onPress={() => to(where, true)}>
      <Text style={look.strong}>{name}</Text>
    </Pressable>
  )
  return (
    <View role="navigation" aria-label="Path" style={look.crumbs}>
      {crumb(short(props.repo), { tab: "code", path: "" })}
      <For each={parts()}>
        {(part, index) => (
          <>
            <Text aria-hidden="true" style={look.quiet}>/</Text>
            <Show when={index() < parts().length - 1 || props.folder} fallback={<Text style={look.strong}>{part}</Text>}>
              {crumb(part, { tab: "code", path: parts().slice(0, index() + 1).join("/") })}
            </Show>
          </>
        )}
      </For>
    </View>
  )
}

/// The branch open, and where there's more than one, the way to another, which starts at its top: the folder or file open
/// may not be there.
function Branch(props: { about: About; branch: string }) {
  const name = () => <Text style={look.strong}>{props.branch}</Text>
  return (
    <Show
      when={props.about.branches.length > 1}
      fallback={
        <View style={look.branch}>
          <Branchy />
          {name()}
        </View>
      }
    >
      <Menu.Root onSelect={(value) => (setRef(value), setAt({ tab: "code", path: "" }))}>
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
    <Show when={!at().file} fallback={<File repo={props.repo} about={props.about} branch={props.branch} path={at().file!} />}>
      <Show when={!tree()!.empty} fallback={<Empty>Nothing's been pushed yet. Clone it, commit and push.</Empty>}>
        <View style={look.page}>
          <View style={look.bar}>
            <Branch about={props.about} branch={props.branch} />
            <Show when={at().path}>
              <Crumbs repo={props.repo} path={at().path!} folder />
            </Show>
          </View>
          <View style={look.card}>
            <Show when={tree()!.commit}>
              {(commit) => (
                <Pressable role="button" onPress={() => to({ tab: "history", commit: commit().sha })} style={[look.head, look.commitHead]}>
                  <Text style={[look.strong, look.oneLine]}>{commit().author}</Text>
                  <Text style={[look.footnote, look.oneLine, look.shrinks]}>{commit().subject}</Text>
                  <Text style={[look.footnote, look.oneLine]}>
                    {commit().sha.slice(0, 7)} · {since(commit().at)}
                  </Text>
                </Pressable>
              )}
            </Show>
            <View role="list">
              <For each={tree()!.entries}>
                {(entry, index) => (
                  <View role="listitem" style={[index() > 0 && look.between]}>
                    <Pressable
                      role="button"
                      onPress={() => to(entry.type === "tree" ? { tab: "code", path: [at().path, entry.name].filter(Boolean).join("/") } : { tab: "code", path: at().path, file: [at().path, entry.name].filter(Boolean).join("/") })}
                      style={look.entry}
                    >
                      <View style={look.quiet}>
                        {entry.type === "tree" ? <Folder /> : <Icon name="file" />}
                      </View>
                      <Text style={[look.shrinks, look.oneLine]}>{entry.name}</Text>
                      <Text style={look.footnote}>{entry.size != null ? size(entry.size) : ""}</Text>
                    </Pressable>
                  </View>
                )}
              </For>
            </View>
          </View>
          <Show when={!at().path}>
            <Text style={look.footnote}>
              {plural(tree()!.commits, "commit")} on {props.branch}
            </Text>
          </Show>
          <Show when={tree()!.readme}>{(readme) => <Readme readme={readme()} />}</Show>
        </View>
      </Show>
    </Show>
  )
}

/// A README as GitHub shows it: its Markdown drawn from what marked makes of it, so nothing in it runs.
function Readme(props: { readme: { name: string; text: string } }) {
  const markdown = () => /\.(md|markdown)$/i.test(props.readme.name)
  return (
    <View style={look.card}>
      <Text role="heading" aria-level={2} style={[look.head, look.strong, look.readmeHead]}>
        {props.readme.name}
      </Text>
      <Show when={markdown()} fallback={<Plain text={props.readme.text} />}>
        <View style={look.markdown}>
          <Blocks tokens={marked.lexer(props.readme.text)} />
        </View>
      </Show>
    </View>
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
        <View style={[look.heading, depth <= 2 && look.majorHeading]}>
          <Text role="heading" aria-level={depth} style={depth <= 2 ? look.title : look.strong}>
            <Inline tokens={tokens} />
          </Text>
        </View>
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
        <View role="list" style={look.items}>
          <For each={items}>
            {(item, index) => (
              <View role="listitem" style={look.item}>
                <Text aria-hidden="true" style={look.bullet}>
                  {item.task ? (item.checked ? "☑" : "☐") : ordered ? `${(Number(start) || 1) + index()}.` : "•"}
                </Text>
                <View style={look.itemBody}>
                  <Blocks tokens={item.tokens} />
                </View>
              </View>
            )}
          </For>
        </View>
      )
    }
    case "code":
      return (
        <View style={look.code}>
          <Plain text={(token as Tokens.Code).text} />
        </View>
      )
    case "blockquote":
      return (
        <View style={look.quote}>
          <Blocks tokens={(token as Tokens.Blockquote).tokens} />
        </View>
      )
    case "hr":
      return <View role="separator" style={look.rule} />
    case "table": {
      const { header, rows } = token as Tokens.Table
      const row = (cells: Tokens.TableCell[], heading: boolean) => (
        <View role="listitem" style={look.tableRow}>
          <For each={cells}>
            {(cell) => (
              <Text style={[look.cell, heading && look.strong]}>
                <Inline tokens={cell.tokens} />
              </Text>
            )}
          </For>
        </View>
      )
      return (
        <View role="list">
          {row(header, true)}
          <For each={rows}>{(cells) => row(cells, false)}</For>
        </View>
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
        <Text style={look.strong}>
          <Inline tokens={(token as Tokens.Strong).tokens} />
        </Text>
      )
    case "em":
    case "del":
      return <Inline tokens={(token as Tokens.Em).tokens} />
    case "codespan":
      return <Text style={look.code}> {plain((token as Tokens.Codespan).text)} </Text>
    case "br":
      return <>{"\n"}</>
    case "image":
      return <>{(token as Tokens.Image).text}</>
    default:
      return null
  }
}

/// A file, its lines numbered.
function File(props: { repo: string; about: About; branch: string; path: string }) {
  const file = createMemo(() => json<{ text?: string | null; size: number; binary?: boolean }>("git-repos_file", { repo: props.repo, ref: props.branch, path: props.path }))
  const lines = () => file().text!.replace(/\n$/, "").split("\n")
  // Back goes to the folder it's in.
  const folder = () => props.path.split("/").slice(0, -1).join("/")
  return (
    <View style={look.page}>
      <View style={look.bar}>
        <Button variant="ghost" onPress={() => to({ tab: "code", path: folder() }, true)}>
          <Icon name="caret-left" along /> {folder().split("/").pop() || short(props.repo)}
        </Button>
        <Branch about={props.about} branch={props.branch} />
        <Crumbs repo={props.repo} path={props.path} />
      </View>
      <View style={look.card}>
        <Text style={[look.head, look.footnote]}>
          {file().text != null ? `${plural(lines().length, "line")} · ` : ""}
          {size(file().size)}
        </Text>
        <Show when={file().text != null} fallback={<Empty>{file().binary ? "It isn't text. Clone the repo to open it." : "It's too large to show. Clone the repo to open it."}</Empty>}>
          <Lines lines={lines().map((text, index) => ({ text, number: index + 1 }))} />
        </Show>
      </View>
    </View>
  )
}

// Whitespace is kept as no-break spaces, which a host that collapses it leaves alone.
const kept = (text: string) => text.replace(/^[ \t]+/, (space) => space.replace(/\t/g, "    ").replace(/ /g, "\u00a0")) || "\u00a0"

/// Lines of code, each with its number where it has one, in the device's monospace on one line each: wider than the
/// card, they scroll sideways. A diff's added and removed lines are tinted across the card.
function Lines(props: { lines: { text: string; number?: number; added?: boolean; removed?: boolean; muted?: boolean }[] }) {
  return (
    <ScrollView style={look.scroll}>
      <View style={look.lines}>
        <For each={props.lines}>
          {(line) => (
            <View style={[look.line, line.added && look.added, line.removed && look.removed]}>
              <Show when={line.number}>
                <Text style={[look.footnote, look.number]}>{line.number}</Text>
              </Show>
              <Text style={[look.footnote, look.lineText, line.muted && look.quiet]}>{kept(line.text)}</Text>
            </View>
          )}
        </For>
      </View>
    </ScrollView>
  )
}

function History(props: { repo: string; branch: string }) {
  const [pages, setPages] = createSignal(1)
  const log = createMemo(() => Promise.all(Array.from({ length: pages() }, (_, page) => json<{ commits: Commit[]; more?: boolean }>("git-repos_log", { repo: props.repo, ref: props.branch, page }))))
  const commits = () => log().flatMap((page) => page.commits)
  return (
    <Show when={!at().commit} fallback={<CommitScreen repo={props.repo} sha={at().commit!} />}>
      <Show when={commits().length} fallback={<Empty>No commits yet.</Empty>}>
        <List each={commits()}>{(commit) => <Row heading={commit.subject} text={`${commit.author} · ${since(commit.at)} · ${commit.sha.slice(0, 7)}`} onPress={() => to({ tab: "history", commit: commit.sha })} />}</List>
        <Show when={log().at(-1)!.more}>
          <Button variant="outline" onPress={() => setPages(pages() + 1)}>
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
    return { text, added: text[0] === "+", removed: text[0] === "-" }
  }
  const more = () => commit().message.split("\n").slice(1).join("\n").trim()
  return (
    <View style={look.page}>
      <View style={look.bar}>
        <Button variant="ghost" onPress={() => to({ tab: "history" }, true)}>
          <Icon name="caret-left" along /> History
        </Button>
      </View>
      <View style={look.card}>
        <Text role="heading" aria-level={2} style={[look.strong, look.message, !!more() && look.messageMore]}>
          {commit().message.split("\n")[0]}
        </Text>
        <Show when={more()}>{(more) => <Plain text={more()} />}</Show>
        <Text style={[look.head, look.footnote, look.foot]}>
          {commit().author} committed {since(commit().at)} · {commit().sha.slice(0, 7)}
        </Text>
      </View>
      <View style={look.card}>
        <Lines lines={commit().diff.replace(/^---\n/, "").split("\n").map(line)} />
      </View>
    </View>
  )
}

function PullRequests(props: { repo: string; pushes: boolean }) {
  const pullRequests = createMemo(() => json<{ pull_requests: PullRequest[] }>("git-repos_pull_requests", { repo: props.repo }).then(({ pull_requests }) => pull_requests))
  const open = () => pullRequests().filter((one) => one.state === "open")
  const closed = () => pullRequests().filter((one) => one.state !== "open")
  const merge = async (one: PullRequest) => {
    if (!(await confirm({ heading: `Merge ${one.title}?`, text: `${plural(one.commits.length, "commit")} into ${one.into}.`, action: "Merge" }))) return
    if (await does("git-repos_merge", { repo: props.repo, pull_request: one.name })) (toast(`Merged into ${one.into}.`), refresh(pullRequests))
  }
  const close = async (one: PullRequest) => (await confirm({ heading: `Close ${one.title}?`, text: "It stays listed as closed.", action: "Close" })) && (await does("git-repos_close", { repo: props.repo, pull_request: one.name })) && refresh(pullRequests)
  const line = (one: PullRequest) => `${short(one.by)} · into ${one.into} · ${plural(one.commits.length, "commit")} · ${since(one.at)}`
  return (
    <Show when={pullRequests().length} fallback={<Empty>No pull requests.</Empty>}>
      <Show when={open().length}>
        <Section heading="Open">
          <List each={open()}>
            {(one) => (
              <Row
                heading={one.title}
                text={line(one)}
                end={
                  <Show when={props.pushes}>
                    <View style={look.actions}>
                      <Button variant="secondary" onPress={() => merge(one)}>
                        Merge
                      </Button>
                      <Button variant="ghost" onPress={() => close(one)}>
                        Close
                      </Button>
                    </View>
                  </Show>
                }
              />
            )}
          </List>
        </Section>
      </Show>
      <Show when={closed().length}>
        <Section heading="Closed">
          <List each={closed()}>{(one) => <Row heading={one.title} text={`${one.state === "merged" ? "Merged" : "Closed"} · ${line(one)}`} />}</List>
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
