// A file, as GitHub shows one: its path, its newest commit, and its text in the code viewer (Markdown as a preview, or
// its code). Where there's room its folder's files are beside it, to go from one to the next without leaving.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Card, CodeView, CopyButton, Empty, Icon, List, Tabs, TabsList, TabsTrigger } from "@one/ui"
import { CommitCard, PathBar } from "../lib/frame.jsx"
import { Markdown } from "../lib/parts.jsx"
import { Beside, Frame, Pending, plural, short, useCall, useWide } from "../lib/use.jsx"

const P = one.params
const dirOf = (path) => path.split("/").slice(0, -1).join("/")
const isMarkdown = (path) => /\.(md|mdx|markdown)$/i.test(path)

// The file's text. A Markdown one starts as a preview; MDX's import and export lines, and lines that are only a
// component, are left out of it.
function Body({ d, path }) {
  const [view, setView] = useState(isMarkdown(path) ? "preview" : "code")
  if (d.submodule) return <Empty>This is a submodule of commit <code>{short(d.submodule)}</code> of another repo.</Empty>
  if (d.binary) return <Empty>This binary file ({one.size(d.size)}) can't be previewed.</Empty>
  if (d.too_big) return <Empty>This file is too big to preview ({one.size(d.size)}). Clone the repo to read it.</Empty>
  const markdown = isMarkdown(path)
  const text = d.text.split("\n").filter((l) => !/\.mdx$/i.test(path) || (!/^\s*(import|export)\s/.test(l) && !/^\s*<\/?[A-Z][\w.]*[^>]*\/?>\s*$/.test(l))).join("\n")
  return (
    <div className="flex flex-col gap-3">
      {markdown && (
        <Tabs value={view} onValueChange={setView}>
          <TabsList>
            <TabsTrigger value="preview">Preview</TabsTrigger>
            <TabsTrigger value="code">Code</TabsTrigger>
          </TabsList>
        </Tabs>
      )}
      {markdown && view === "preview" ? (
        <Card className="gap-0 py-0">
          <div className="flex items-center justify-between gap-2 border-b py-1.5 pr-1.5 pl-4 text-sm text-muted-foreground">
            <span>{plural(d.text.split("\n").length, "line", "lines")} · {one.size(d.size)}</span>
            <CopyButton text={d.text} />
          </div>
          <div className="px-6 py-5"><Markdown text={text} /></div>
        </Card>
      ) : (
        <CodeView text={d.text} path={path} />
      )}
    </div>
  )
}

function File({ repo, rev, path }) {
  const file = useCall("file", { repo, ref: rev, path })
  useEffect(() => {
    one.title(path.split("/").pop())
  }, [path])
  const d = file.data
  if (!d) return <Pending call={file} rows={2} />
  return (
    <div className="flex min-w-0 flex-col gap-3">
      <PathBar d={d} file />
      <CommitCard c={d.latest} d={{ ...d, path }} />
      <Body d={d} path={path} />
    </div>
  )
}

// The folder the file is in, beside it: pick a file to read it here, a folder to see its files.
function Tree({ repo, rev, dir, path, pick, enter }) {
  const tree = useCall("tree", { repo, ref: rev, path: dir })
  const d = tree.data
  if (!d) return <Pending call={tree} rows={4} />
  const here = (e) => (dir ? dir + "/" : "") + e.name
  const Row = ({ icon, name, selected, onClick }) => (
    <button
      type="button"
      onClick={onClick}
      data-selected={selected || undefined}
      className="flex w-full min-w-0 cursor-pointer items-center gap-2.5 px-3 py-2 text-left text-sm outline-none hover:bg-accent/50 focus-visible:bg-accent/50 data-[selected]:bg-accent"
    >
      <Icon name={icon} className="text-muted-foreground" />
      <span className="truncate">{name}</span>
    </button>
  )
  return (
    <List>
      {dir && <Row icon="folder" name=".." onClick={() => enter(dirOf(dir))} />}
      {(d.entries || []).map((e) =>
        e.type === "tree" ? <Row key={e.name} icon="folder" name={e.name} onClick={() => enter(here(e))} />
        : e.type === "blob" ? <Row key={e.name} icon="file" name={e.name} selected={here(e) === path} onClick={() => pick(here(e))} />
        : <Row key={e.name} icon="folder-dashed" name={e.name} />,
      )}
    </List>
  )
}

function Screen() {
  const wide = useWide()
  const [path, setPath] = useState(P.path)
  const [dir, setDir] = useState(dirOf(P.path))
  const file = <File key={path} repo={P.repo} rev={P.ref} path={path} />
  return wide ? (
    <Beside
      size="20rem"
      list={<Tree repo={P.repo} rev={P.ref} dir={dir} path={path} pick={setPath} enter={setDir} />}
      detail={file}
    />
  ) : (
    file
  )
}

createRoot(document.getElementById("root")).render(<Frame><Screen /></Frame>)
