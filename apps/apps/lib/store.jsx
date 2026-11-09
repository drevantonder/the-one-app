// What the App Store's screens share: a call's answer, loaded and kept fresh; waiting and failing as one part says; the
// list with adding an app and opening it; and a section's heading.
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

/// The list, and `add(a, place)`: to You unless given a place, drawn as added at once and put back if it fails, which
/// it throws for the caller to say.
export function useApps() {
  const list = useList()
  const [adding, setAdding] = useState([])
  const data = list.data && {
    ...list.data,
    apps: list.data.apps.map((a) => {
      const more = adding.filter((x) => x.app === a.name && !a.added.includes(x.place)).map((x) => x.place)
      return more.length ? { ...a, added: [...a.added, ...more] } : a
    }),
  }
  const add = async (a, place = data.places[0]) => {
    const entry = { app: a.name, place: place.id }
    setAdding((all) => [...all, entry])
    try {
      await one.call("_add", { app: a.name, scope: place.scope })
      one.tap("success")
      await list.reload()
    } finally {
      setAdding((all) => all.filter((x) => x !== entry))
    }
  }
  return { ...list, data, add }
}

export const onAPage = (a, places) => a.added.some((id) => places.some((p) => p.id === id))

/// Opens an app on the first of its places that's the user's, You before a group or Everyone.
export const openApp = (a, places) => one.open("main", {}, a.name, { scope: places.find((p) => a.added.includes(p.id))?.scope })

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
