// The board: its topics, the top first. Where there's room, the topic you pick shows its posts beside the list; where
// there isn't, it opens as its own screen.
import { useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, List, ListRow, Split, useWidth } from "@one/ui"
import { Feed, FeedSkeleton, Loading, readTopic, readTopics, Trouble, useLoaded } from "../lib/board"

const WIDE = 720
const top = { name: "board", color: "#ff9500", icon: "star" }
const topic = { name: "topic", icon: "hash" }

function Posts({ path, description }) {
  const { data, problem, retry } = useLoaded(() => readTopic(path))
  return (
    <div className="flex h-[calc(100svh-2rem)] flex-col gap-4">
      <div>
        <h2 className="text-base font-semibold">{path || "Top of the board"}</h2>
        {(data?.description ?? description) && <p className="text-sm text-muted-foreground">{data?.description ?? description}</p>}
      </div>
      {problem ? <Trouble problem={problem} retry={retry} /> : data ? <Feed own className="min-h-0 flex-1" posts={data.posts} /> : <FeedSkeleton />}
    </div>
  )
}

function Main() {
  const [shell, width] = useWidth()
  const beside = width >= WIDE
  const [open, setOpen] = useState("")
  const { data: topics, problem, retry } = useLoaded(readTopics)

  const row = (t) => {
    const n = t.newest
    const first = t.path === ""
    return (
      <ListRow
        key={t.path}
        leading={<AppIcon app={first ? top : topic} size={32} />}
        title={
          <span className="flex items-baseline justify-between gap-2">
            <span className="min-w-0 truncate">{first ? "Top of the board" : t.path}</span>
            {n && <span className="shrink-0 text-xs font-normal text-muted-foreground">{one.ago(n.time)}</span>}
          </span>
        }
        sub={n ? `${n.name}: ${n.text}` : t.description || "Nothing here yet"}
        selected={beside && open === t.path}
        onClick={() => (beside ? setOpen(t.path) : one.open("topic", { path: t.path }))}
      />
    )
  }

  let list
  if (problem) list = <Trouble problem={problem} retry={retry} />
  else if (!topics) list = <Loading>Loading topics</Loading>
  else {
    const rest = topics.slice(1)
    list = (
      <div className="flex flex-col gap-4">
        <List>{row(topics[0])}</List>
        <h2 className="px-1 text-xs font-medium text-muted-foreground">Topics</h2>
        {rest.length ? <List>{rest.map(row)}</List> : <Empty>No topics yet</Empty>}
      </div>
    )
  }
  const chosen = topics?.find((t) => t.path === open)
  return (
    <div ref={shell}>
      <Split
        wide={WIDE}
        list={list}
        detail={beside && topics ? <Posts key={open} path={open} description={chosen?.description} /> : undefined}
      />
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
