// A commit's page: its message, who made it, and each changed file's diff.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { CommitView } from "../lib/commit-view.jsx"
import { Frame, short } from "../lib/use.jsx"

const P = one.params

function Commit() {
  useEffect(() => {
    one.title(`Commit ${short(P.sha)}`)
  }, [])
  return <CommitView repo={P.repo} sha={P.sha} />
}

createRoot(document.getElementById("root")).render(<Frame><Commit /></Frame>)
