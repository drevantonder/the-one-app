import * as React from "react"
import { Grid } from "@/components/layout"
import { Skeleton } from "@/components/ui/skeleton"
import { Problem } from "@/components/problem"
import { bridge } from "@/lib/bridge"
import { appear } from "@/lib/moves"
import { cn } from "@/lib/utils"

/// A failure's first sentence, without "Try again." (what the button says), as the window words it (`one.plain`).
function failureSentence(err: unknown): string {
  return bridge().plain(err).title.replace(/(\. Try again)?\.$/, "")
}

/// What a failed first load says (`problem` is `one.plain`'s answer), with Try Again where trying again can work.
function LoadFailure({ problem, retry }: { problem: { title: string; next?: string; final?: boolean }; retry: () => unknown }) {
  return <Problem title={problem.title} text={problem.next} action={problem.final ? undefined : { label: "Try Again", run: retry }} />
}

/// Grey shapes the shape of what's coming, once it's taken longer than a blink, so a quick load shows nothing between:
/// `rows` of a List, or with `tiles` (the width of a column) that many tiles of a Grid; `icon`, how many pixels each one's
/// is, as what's coming draws it. What takes their place fades in.
function ListSkeleton({ rows = 2, tiles, icon = tiles ? 56 : 32 }: { rows?: number; tiles?: string; icon?: number }) {
  const [late, setLate] = React.useState(false)
  const shapes = React.useRef<HTMLDivElement>(null)
  React.useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  React.useLayoutEffect(() => {
    const parent = shapes.current?.parentElement
    if (!parent) return
    return () => {
      // React takes these away before it puts in what replaces them.
      const before = new Set(parent.children)
      queueMicrotask(() => [...parent.children].filter((el) => !before.has(el)).forEach((el) => appear(el)))
    }
  }, [late])
  if (!late) return null
  const each = Array.from({ length: rows }, (_, i) => <SkeletonRow key={i} icon={icon} className={icon > 32 ? "p-2" : "px-3 py-2.5"} />)
  const fades = "motion-safe:animate-in motion-safe:fade-in-0 motion-safe:duration-200"
  return tiles
    ? <Grid ref={shapes} data-slot="list-skeleton" aria-busy="true" min={tiles} max={4} gap="0.25rem" className={fades}>{each}</Grid>
    : <div ref={shapes} data-slot="list-skeleton" aria-busy="true" className={cn("w-full max-w-2xl divide-y overflow-hidden rounded-xl border bg-card", fades)}>{each}</div>
}

function SkeletonRow({ icon, className }: { icon: number; className: string }) {
  return (
    <div className={cn("flex min-w-0 items-center gap-3", className)}>
      <Skeleton className="shrink-0 rounded-lg" style={{ width: icon, height: icon }} />
      <div className="flex min-w-0 flex-1 flex-col">
        <div className="flex h-5 items-center"><Skeleton className="h-3 w-2/5" /></div>
        <div className="flex h-5 items-center"><Skeleton className="h-3 w-3/4" /></div>
      </div>
    </div>
  )
}

/// What a call hasn't answered yet: grey shapes, or why it couldn't.
function Pending({ call, rows = 3, tiles, icon }: { call: Call; rows?: number; tiles?: string; icon?: number }) {
  return call.error ? <LoadFailure problem={bridge().plain(call.error)} retry={call.reload} /> : <ListSkeleton rows={rows} tiles={tiles} icon={icon} />
}

/// A call's answer, from the app's node: `data` is null until the first comes, and a later failure keeps what's shown, in
/// `error`. It's asked for when drawn and whenever the app changes. When `input` changes the last answer stays, with
/// `moving` true, until the next comes (`of` says which input `data` answers). With `input` null it isn't asked for (yet).
type Call<T = any> = { data: T | null; error: unknown; of: string | null; moving: boolean; reload: () => Promise<unknown> }

// The bridge has no way to stop listening for "changed", so this is its one listener.
const listening = new Set<() => unknown>()
let hearing = false

function useCall<T = any>(tool: string, input: object | null): Call<T> {
  const key = JSON.stringify(input)
  const [state, setState] = React.useState<{ data: T | null; error: unknown; of: string | null }>({ data: null, error: null, of: null })
  // An answer that comes after the next was asked for is out of date, whichever was quicker.
  const wanted = React.useRef(key)
  const reload = React.useCallback(() => {
    if (input === null) return Promise.resolve()
    return bridge().call(tool, input).then(
      (data: T) => void (wanted.current === key && setState({ data, error: null, of: key })),
      (error: unknown) => void (wanted.current === key && setState((s) => ({ ...s, error }))),
    )
  }, [tool, key])
  React.useEffect(() => {
    if (!hearing) {
      hearing = true
      bridge().on("changed", () => listening.forEach((run) => run()))
    }
    wanted.current = key
    setState((s) => (s.error ? { ...s, error: null } : s))
    reload()
    listening.add(reload)
    return () => void listening.delete(reload)
  }, [reload])
  return { ...state, moving: state.of !== null && state.of !== key, reload }
}

/// What shows for one of several things, the open one of a list or what a filter chose (`call`, its `useCall`): grey
/// shapes until there's an answer; going to another, the last stays where it is, dimming only if it's slow, until the
/// next has come and fades in. If that one fails, its problem shows instead. What's in `children` is drawn afresh for
/// each answer of another input, so nothing typed in one carries over to the next.
function Swap<T>({ call, rows = 3, children }: { call: Call<T>; rows?: number; children: (data: T) => React.ReactNode }) {
  const pane = React.useRef<HTMLDivElement>(null)
  const was = React.useRef(call.of)
  React.useLayoutEffect(() => {
    if (was.current && was.current !== call.of && pane.current) appear(pane.current, 0.4)
    was.current = call.of
  }, [call.of])
  const data = call.moving && call.error ? null : call.data
  if (data === null) return <Pending call={call} rows={rows} />
  return (
    <div ref={pane} data-slot="swap" className={cn("min-w-0 transition-opacity duration-150", call.moving && "opacity-50 delay-200")}>
      <React.Fragment key={call.of}>{children(data)}</React.Fragment>
    </div>
  )
}

/// One grey line with a spinner, for what's under way.
function Loading({ children }: { children: React.ReactNode }) {
  return (
    <p data-slot="loading" className="flex items-center gap-2 text-sm text-muted-foreground">
      <Spinner />
      {children}
    </p>
  )
}

export { failureSentence, Loading, LoadFailure, ListSkeleton, Pending, Swap, useCall }
export type { Call }
