// What both of the board's screens share: reading a topic, and drawing its posts.
import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { cn, Problem, Skeleton } from "@one/ui"

const scope = () => (one.scope ? { scope: one.scope } : {})

/// Runs `load` now, and again whenever the app's data changes. What it gives is `{ data, problem }`: neither while the
/// first answer is out; a failed redraw leaves what's there, which was right a moment ago.
export function useLoaded(load) {
  const [state, setState] = useState({})
  const latest = useRef(load)
  latest.current = load
  useEffect(() => {
    const run = () =>
      latest.current().then(
        (data) => setState({ data }),
        (err) => setState((s) => (s.data ? s : { problem: one.plain(err), retry: run })),
      )
    one.on("changed", run)
    run()
  }, [])
  return state
}

export const readTopic = (path) => one.call("read", { format: "json", ...(path ? { topic: path } : {}), ...scope() })
export const readTopics = () => one.call("topics", { format: "json", ...scope() })

/// A first answer that's slow shows as a grey line, after a moment, so a quick one never flashes.
export function Loading({ children }) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  return late ? <p className="px-6 py-12 text-center text-sm text-muted-foreground">{children}</p> : null
}

export function Trouble({ problem, retry }) {
  return <Problem title={problem.title} text={problem.next} action={{ label: "Try Again", run: retry }} />
}

const day = (t) => new Date(t).toDateString()
const initials = (name) => String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase()

function Avatar({ name }) {
  return (
    <span className="grid size-8 shrink-0 place-items-center rounded-full bg-muted text-xs font-semibold text-muted-foreground">
      {initials(name)}
    </span>
  )
}

/// A topic's posts, oldest first, with a divider for each day; posts by the same author within 10 minutes stack under
/// one header. It stays on its newest post while it grows or the view resizes, unless scrolled up: in the window's
/// scroll, or its own (`own`) where it's one pane beside another.
export function Feed({ posts, own = false, className }) {
  const box = useRef(null)
  const pinned = useRef(true)
  const scroller = () => (own ? box.current : null)
  const bottom = () => (own ? box.current.scrollTo({ top: box.current.scrollHeight }) : window.scrollTo(0, document.body.scrollHeight))
  const atBottom = () =>
    own
      ? box.current.scrollTop + box.current.clientHeight >= box.current.scrollHeight - 40
      : innerHeight + scrollY >= document.body.scrollHeight - 40

  useEffect(() => {
    const target = scroller() || window
    const seen = () => (pinned.current = atBottom())
    target.addEventListener("scroll", seen, { passive: true })
    const resize = new ResizeObserver(() => pinned.current && bottom())
    resize.observe(own ? box.current.firstElementChild : document.documentElement)
    return () => {
      target.removeEventListener("scroll", seen)
      resize.disconnect()
    }
  }, [own])
  useLayoutEffect(() => {
    if (pinned.current) bottom()
  }, [posts])

  const rows = []
  posts.forEach((p, i) => {
    const last = posts[i - 1]
    if (!last || day(last.time) !== day(p.time))
      rows.push(
        <div key={`day-${p.id}`} className="pt-4 text-center text-xs font-medium text-muted-foreground first:pt-0">
          {new Date(p.time).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}
        </div>,
      )
    const same = last && day(last.time) === day(p.time) && last.user === p.user && last.credit === p.credit && p.time - last.time < 10 * 60000
    rows.push(
      same ? (
        <p key={p.id} className="-mt-3 break-words pl-11 text-sm">{p.text}</p>
      ) : (
        <div key={p.id} className="flex items-start gap-3">
          <Avatar name={p.name} />
          <div className="min-w-0 flex-1">
            <div className="text-sm">
              <span className="font-semibold">{p.credit || p.name}</span>
              <span className="ml-2 whitespace-nowrap text-xs text-muted-foreground">{one.ago(p.time)}</span>
            </div>
            <p className="break-words text-sm">{p.text}</p>
          </div>
        </div>
      ),
    )
  })
  const feed = (
    <div className="flex flex-col gap-4">
      {posts.length ? rows : <p className="px-6 py-12 text-center text-sm text-muted-foreground">No posts yet</p>}
    </div>
  )
  return own ? (
    <div ref={box} className={cn("overflow-y-auto", className)}>{feed}</div>
  ) : (
    feed
  )
}

export function FeedSkeleton() {
  return (
    <div className="flex flex-col gap-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex gap-3">
          <Skeleton className="size-8 rounded-full" />
          <div className="flex-1 space-y-2"><Skeleton className="h-4 w-1/3" /><Skeleton className="h-4 w-full" /></div>
        </div>
      ))}
    </div>
  )
}
