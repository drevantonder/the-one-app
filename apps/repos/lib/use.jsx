// What every Repos screen shares: the room a screen has; times and names as a user
// reads them; and where git reaches a repo.
import { createContext, useContext, useEffect, useState } from "react"
import { useWidth } from "@one/ui"

export const said = (error) => one.plain(error)

export const WIDE = 640

const Room = createContext(0)

export function Frame({ children }) {
  const [ref, width] = useWidth()
  return (
    <div ref={ref}>
      <Room.Provider value={width}>{children}</Room.Provider>
    </div>
  )
}

export const useWide = () => useContext(Room) >= WIDE
// Room for two columns that can each be read: a list and what it opens, or a repo's files and its README.
export const WIDE_BESIDE = 1100
export const useBeside = () => useContext(Room) >= WIDE_BESIDE

export const ago = (iso) => one.ago(Date.parse(iso), { long: true })
export const short = (sha) => String(sha || "").slice(0, 7)
export const plural = (n, single, many) => `${n.toLocaleString()} ${n === 1 ? single : many}`
export const refOf = (d) => d.branch || d.sha
export const isGithub = (repo) => String(repo || "").startsWith("github.com/")
export const go = (screen, params) => one.open(screen, params)

let base = "<address>/repos/"
let mine = "<you>"
const based = one.call("_address").then((b) => ((base = b?.base || base), (mine = b?.handle || mine)), () => {})

export function useAddress() {
  const [, redraw] = useState(0)
  useEffect(() => void based.then(() => redraw((n) => n + 1)), [])
  return { address: (repo) => `${base}${repo}.git`, mine }
}

export const where = `On any computer where you’ve run ${one.config?.cli || "one"} sign-in`
