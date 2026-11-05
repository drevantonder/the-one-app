// The app's main screen. React and @one/ui (shadcn's components by their names, and ours: Grid, List, ListRow, Split,
// Empty, Problem, Icon, AppIcon) come from the page; Tailwind's classes, with shadcn's theme, work in className.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, List, ListRow, Problem } from "@one/ui"

function Main() {
  const [said, setSaid] = useState([])
  const [problem, setProblem] = useState(null)
  const hello = () => one.call("hello").then((r) => setSaid((s) => [r.text, ...s]), (e) => setProblem(e.message))

  useEffect(() => {
    one.title("Hello")
  }, [])

  return (
    <div className="flex flex-col gap-4 p-4">
      {problem && <Problem title="Couldn't say hello" text={problem} />}
      {said.length === 0 ? (
        <Empty action={{ label: "Say Hello", run: hello }}>Nothing said yet</Empty>
      ) : (
        <>
          <Button className="self-start" onClick={hello}>Say Hello Again</Button>
          <List>
            {said.map((text, i) => <ListRow key={said.length - i} title={text} />)}
          </List>
        </>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
