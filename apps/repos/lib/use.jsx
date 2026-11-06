// What every Repos screen shares: a call's answer, kept fresh; the room a screen has; times and names as a person
// reads them; and where git reaches a repo.
import { createContext, useCallback, useContext, useEffect, useState } from "react"
import { ListSkeleton, Problem, useWidth } from "@one/ui"

// The node's "changed" events go to every part that has loaded something: the bridge has no way to stop listening, so
// this is its one listener.
const listening = new Set()
one.on("changed", () => listening.forEach((reload) => reload()))

/// A call's answer: { data, error, reload }. Asked for when drawn, and again whenever the app changes; a later failure
/// keeps what's shown. With `input` null it isn't asked for (yet). With `keep`, an answer stays shown while the next one
/// for other input is out, rather than grey shapes.
export function useCall(op, input, keep = false) {
  const [state, setState] = useState({ data: null, error: null })
  const key = JSON.stringify(input)
  const reload = useCallback(
    () => (input === null ? Promise.resolve() : one.call(op, input).then((data) => setState({ data, error: null }), (error) => setState((s) => ({ data: s.data, error })))),
    [op, key],
  )
  useEffect(() => {
    if (!keep) setState({ data: null, error: null })
    reload()
    listening.add(reload)
    return () => listening.delete(reload)
  }, [reload])
  return { ...state, reload }
}

export const said = (error) => one.plain(error)

export function Failed({ error, retry }) {
  const p = said(error)
  return <Problem title={p.title} text={p.next} action={{ label: "Try Again", run: retry }} />
}

export const Pending = ({ call, rows = 4 }) => (call.error ? <Failed error={call.error} retry={call.reload} /> : <ListSkeleton rows={rows} />)

export function Section({ title, children }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}

export const WIDE = 720

const Room = createContext(0)

export function Frame({ children }) {
  const [ref, width] = useWidth()
  return (
    <div className="p-4">
      <div ref={ref}>
        <Room.Provider value={width}>{children}</Room.Provider>
      </div>
    </div>
  )
}

export const useWide = () => useContext(Room) >= WIDE

export function Beside({ list, detail, size = "24rem" }) {
  const wide = useWide()
  if (!wide) return list
  return (
    <div className="grid items-start gap-6" style={{ gridTemplateColumns: `minmax(0, ${size}) minmax(0, 1fr)` }}>
      <div className="min-w-0">{list}</div>
      <div className="min-w-0">{detail}</div>
    </div>
  )
}

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
