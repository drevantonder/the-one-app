// What the App Store's screens share: a call's answer, loaded and kept fresh; waiting and failing as one part says; and
// a section's heading.
import { useCallback, useEffect, useState } from "react"
import { Problem } from "@one/ui"

// The node's "changed" events go to every part that has loaded something: the bridge has no way to stop listening, so
// this is its one listener.
const listening = new Set()
one.on("changed", () => listening.forEach((reload) => reload()))

export function useCall(tool, input) {
  const [state, setState] = useState({ data: null, error: null })
  const key = JSON.stringify(input)
  const reload = useCallback(
    () => one.call(tool, input).then((data) => setState({ data, error: null }), (error) => setState((s) => ({ data: s.data, error }))),
    [tool, key],
  )
  useEffect(() => {
    reload()
    listening.add(reload)
    return () => listening.delete(reload)
  }, [reload])
  return { ...state, reload }
}

export const useList = () => useCall("_list", {})

export const said = (error) => one.plain(error)

export function Failed({ error, retry }) {
  const p = said(error)
  return <Problem title={p.title} text={p.next} action={{ label: "Try Again", run: retry }} />
}

export function Section({ title, children }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}
