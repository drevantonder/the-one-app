// What the App Store's screens share: the list with adding an app and opening it, and a section's heading.
import { useState } from "react"
import { useCall } from "@one/ui"

export const useList = () => useCall("_list", {})

export const said = (error) => one.plain(error)

/// The list, and `put(a, place, on)`: an app put on a page or taken off it, drawn so at once and put back if it fails,
/// which it throws for the caller to say. Putting on gives back the line to say of the apps it added with it (those it
/// uses), or "".
export function useApps() {
  const list = useList()
  const [changing, setChanging] = useState([])
  const data = list.data && {
    ...list.data,
    apps: list.data.apps.map((a) => {
      let added = a.added
      for (const c of changing.filter((x) => x.app === a.name)) added = c.on ? (added.includes(c.place) ? added : [...added, c.place]) : added.filter((id) => id !== c.place)
      return added === a.added ? a : { ...a, added }
    }),
  }
  const put = async (a, place, on) => {
    const entry = { app: a.name, place: place.id, on }
    const also = on ? a.uses.apps.map((n) => data.apps.find((x) => x.name === n)).filter((u) => u && !u.added.includes(place.id)).map((u) => u.title) : []
    setChanging((all) => [...all, entry])
    try {
      await one.call(on ? "_add" : "_remove", { app: a.name, scope: place.scope })
      one.tap("success")
      await list.reload()
      return also.length ? `Also added ${also.join(" and ")}.` : ""
    } finally {
      setChanging((all) => all.filter((x) => x !== entry))
    }
  }
  return { ...list, data, put }
}

export const onAPage = (a, places) => a.added.some((id) => places.some((p) => p.id === id))

/// Opens an app on the first of its places that's the user's, You before a group or Everyone.
export const openApp = (a, places) => one.open("main", {}, a.name, { scope: places.find((p) => a.added.includes(p.id))?.scope })

/// Its sign-in, for an app with no screen of its own to open, on the same place.
export const addAccount = (a, places) => one.open("settings", { add: true }, a.name, { scope: places.find((p) => a.added.includes(p.id))?.scope })

