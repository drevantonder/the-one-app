// What the App Store's screens share: the list with adding an app and opening it, and a section's heading.
import { useState } from "react"
import { useCall } from "@one/ui"

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

export function Section({ title, children }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}
