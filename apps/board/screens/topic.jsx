// A topic: its description, then its posts. Where the board's main screen has room it shows this beside its topics;
// this is the screen it opens where it doesn't (and where a widget's post opens).
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { Feed, FeedSkeleton, readTopic, Trouble, useLoaded } from "../lib/board"

const path = one.params.path || ""

function Topic() {
  const { data, problem, retry } = useLoaded(() => readTopic(path))
  useEffect(() => {
    one.title(path || "Top of the board")
  }, [])
  return (
    <div className="flex max-w-2xl flex-col gap-4">
      {problem ? (
        <Trouble problem={problem} retry={retry} />
      ) : !data ? (
        <FeedSkeleton />
      ) : (
        <>
          {data.description && <p className="text-sm text-muted-foreground">{data.description}</p>}
          <Feed posts={data.posts} />
        </>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Topic />)
