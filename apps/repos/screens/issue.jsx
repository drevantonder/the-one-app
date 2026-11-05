// An issue as a screen of its own: where Issues' list has no room to show it beside it, and where a link opens it.
import { useEffect } from "react"
import { createRoot } from "react-dom/client"
import { IssueView } from "../lib/issue-view.jsx"
import { Frame } from "../lib/use.jsx"

const P = one.params

function Issue() {
  useEffect(() => {
    one.title(`#${P.number}`)
  }, [])
  return <div className="max-w-3xl"><IssueView repo={P.repo} number={P.number} onGone={() => one.close()} /></div>
}

createRoot(document.getElementById("root")).render(<Frame><Issue /></Frame>)
