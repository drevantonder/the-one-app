// Connect Your AI: what to paste into an AI so it connects itself, as the node says (`agent_prompt`, src/state.rs).
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/connect-ai.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { CopyButton, Problem, Skeleton } from "@one/ui"

function ConnectAI() {
  const [text, setText] = useState(null)
  const [problem, setProblem] = useState(null)

  async function load() {
    try {
      setText((await one.api("GET", "/v1/state")).agent_prompt)
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => {
    one.largeTitle("Connect Your AI")
    load()
  }, [])

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <h1 className="large-title text-3xl font-bold tracking-tight">Connect Your AI</h1>
      <p className="text-sm text-muted-foreground">Paste this into Claude, ChatGPT or another AI, and it can use your apps as you.</p>
      {text !== null ? (
        <div className="flex flex-col gap-3 rounded-xl border bg-card p-4 text-card-foreground">
          <p className="font-mono text-[13px] leading-relaxed break-words select-text [overflow-wrap:anywhere]">{text}</p>
          <CopyButton text={text} variant="default" className="self-start" />
        </div>
      ) : problem ? (
        <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: load }} />
      ) : (
        <div className="flex flex-col gap-3 rounded-xl border p-4" aria-busy="true" aria-label="Loading">
          <Skeleton className="h-3.5 w-full" />
          <Skeleton className="h-3.5 w-2/3" />
        </div>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<ConnectAI />)
