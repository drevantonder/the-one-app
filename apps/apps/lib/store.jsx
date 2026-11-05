// What the App Store's screens share: a call's answer, loaded and kept fresh; waiting and failing as one part says; and
// a section's heading.
import { useCallback, useEffect, useState } from "react"
import { Problem, Skeleton } from "@one/ui"

// The node's "changed" events go to every part that has loaded something: the bridge has no way to stop listening, so
// this is its one listener.
const listening = new Set()
one.on("changed", () => listening.forEach((reload) => reload()))

/// A call's answer: { data, error, reload }. Asked for when drawn, and again whenever the app changes; a later failure
/// keeps what's shown.
export function useCall(op, input) {
  const [state, setState] = useState({ data: null, error: null })
  const key = JSON.stringify(input)
  const reload = useCallback(
    () => one.call(op, input).then((data) => setState({ data, error: null }), (error) => setState((s) => ({ data: s.data, error }))),
    [op, key],
  )
  useEffect(() => {
    reload()
    listening.add(reload)
    return () => listening.delete(reload)
  }, [reload])
  return { ...state, reload }
}

/// Every app, where each is installed, and the pages the person may add them to.
export const useList = () => useCall("list", { full: true })

/// What a failed call says, for a person.
export const said = (error) => one.plain(error)

/// A call that failed with nothing to show yet: what happened, and Try Again.
export function Failed({ error, retry }) {
  const p = said(error)
  return <Problem title={p.title} text={p.next} action={{ label: "Try Again", run: retry }} />
}

/// Grey shapes where rows will be, once it's taken longer than a blink, so a quick load shows nothing in between.
export function Waiting({ rows = 4 }) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  if (!late) return null
  return (
    <div className="flex flex-col gap-2">
      {Array.from({ length: rows }, (_, i) => <Skeleton key={i} className="h-14 w-full max-w-2xl" />)}
    </div>
  )
}

/// A group of rows under its heading.
export function Section({ title, children }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}
