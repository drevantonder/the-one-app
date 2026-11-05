// What every Repos screen shares: a call's answer, kept fresh; the room a screen has; times and names as a person
// reads them; and where git reaches a repo.
import { createContext, useCallback, useContext, useEffect, useState } from "react"
import { Problem, Skeleton, useWidth } from "@one/ui"

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

/// What a call's answer shows while it isn't in: its problem, or grey shapes.
export const Pending = ({ call, rows }) => (call.error ? <Failed error={call.error} retry={call.reload} /> : <Waiting rows={rows} />)

/// A group of rows under its heading.
export function Section({ title, children }) {
  return (
    <section className="flex flex-col gap-2">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}

/// How wide the window's room is where a list opens its page beside it, rather than as a screen of its own.
export const WIDE = 720

const Room = createContext(0)

/// A screen's frame: its padding, and how wide its own room is, which the parts in it size themselves by (`useWide`),
/// not the window.
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

/// Whether the frame's room is wide enough for GitHub's layout: tabs, and what a list opens beside it.
export const useWide = () => useContext(Room) >= WIDE

/// A list, and what it opens beside it where there's room (nothing in `detail` while there's none, or no room).
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

/// A time from an operation, ISO 8601, as how long ago it was.
export const ago = (iso) => one.ago(Date.parse(iso), { long: true })
export const short = (sha) => String(sha || "").slice(0, 7)
export const plural = (n, single, many) => `${n.toLocaleString()} ${n === 1 ? single : many}`
/// The ref to keep using: the branch if there is one, else the commit.
export const refOf = (d) => d.branch || d.sha
/// A repo of GitHub's, connected: github.com/<owner>/<name>.
export const isGithub = (repo) => String(repo || "").startsWith("github.com/")
export const go = (screen, params) => one.open(screen, params)

// Where git reaches a repo, the same on every computer: <address>/repos/<owner>/<repo>.git, once it's asked for; and
// the person's handle, for a repo of theirs. Git goes through the CLI there, so on a phone or in a browser this says
// where to run it.
let base = "<address>/repos/"
let mine = "<you>"
const based = one.call("_address").then((b) => ((base = b?.base || base), (mine = b?.handle || mine)), () => {})

/// Where git reaches repos: `address(repo)`, and the person's handle `mine`, which change once they're known.
export function useAddress() {
  const [, redraw] = useState(0)
  useEffect(() => void based.then(() => redraw((n) => n + 1)), [])
  return { address: (repo) => `${base}${repo}.git`, mine }
}

/// Where to run git from, for a repo reached through the CLI.
export const where = `On any computer where you’ve run ${one.config?.cli || "one"} sign-in`
