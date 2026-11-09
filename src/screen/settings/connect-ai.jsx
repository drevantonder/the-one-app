// Connect Your AI: what to paste into an AI so it connects itself, as the node says (`agent_prompt`, src/state.rs).
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/connect-ai.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { CopyButton, Hint, LargeTitle, LoadFailure, Skeleton } from "@one/ui"

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
    load()
  }, [])

  return (
    <div className="flex max-w-2xl flex-col gap-4">
      <LargeTitle>Connect Your AI</LargeTitle>
      <Hint>Send this to Claude, ChatGPT or any other AI.</Hint>
      {problem && text === null ? (
        <LoadFailure problem={problem} retry={load} />
      ) : (
        <>
          <div aria-busy={text === null || undefined} className="min-h-[4.25rem] rounded-lg bg-muted px-4 py-3 text-sm leading-relaxed text-pretty select-text [overflow-wrap:anywhere]">
            {text ?? <><Skeleton className="my-1 h-3.5 w-full" /><Skeleton className="my-1 h-3.5 w-2/3" /></>}
          </div>
          {text === null ? <Skeleton className="h-9 w-20" /> : <CopyButton text={text} variant="default" size="default" className="min-w-24 self-start" />}
        </>
      )}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<ConnectAI />)
