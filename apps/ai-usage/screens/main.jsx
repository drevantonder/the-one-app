// AI Usage: each provider and its accounts' windows, metered. The accounts fill the room: more columns where there's more of it.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Empty, Grid, Hint, LargeTitle, LoadFailure, Problem, Progress, Skeleton } from "@one/ui"

function Meter({ label, w }) {
  if (!w || w.used == null) return null
  return (
    <div className="grid grid-cols-[4.5rem_minmax(2rem,1fr)_2.5rem] items-center gap-x-3 gap-y-1 text-sm @sm:grid-cols-[4.5rem_minmax(2rem,1fr)_2.5rem_10rem]">
      <span className="truncate text-muted-foreground" title={label}>{label}</span>
      <Progress value={w.used} meter aria-label={label} />
      <span className="text-right tabular-nums">{w.used}%</span>
      <span className="col-span-3 -mt-0.5 text-xs text-muted-foreground @sm:col-span-1 @sm:mt-0 @sm:text-right @sm:whitespace-nowrap">
        {w.countdown ? `resets ${w.countdown}` : ""}
      </span>
    </div>
  )
}

const on = (p) => ((p.devices || []).length ? `on ${p.devices.join(", ")}` : "")

function Account({ p, several }) {
  const [team, email] = (p.account || "").split(" · ")
  const state = p.active && several ? "In use" : p.disabled ? "Not used" : ""
  return (
    <div className="@container flex min-w-0 flex-col gap-2">
      {p.account && (
        <div>
          <div className="flex items-baseline justify-between gap-2">
            <span className="min-w-0 truncate font-semibold">{team}</span>
            <span className="shrink-0 text-sm text-muted-foreground">{state}</span>
          </div>
          <div className="truncate text-sm text-muted-foreground">{[email, on(p)].filter(Boolean).join(" · ")}</div>
        </div>
      )}
      <Meter label="Session" w={p.session} />
      <Meter label="Weekly" w={p.weekly} />
      <Meter label="Monthly" w={p.monthly} />
      {(p.models || []).map((m) => <Meter key={m.name} label={m.name} w={m} />)}
      {p.extra && (
        <div className="flex justify-between gap-3 text-sm">
          <span className="text-muted-foreground">Extra</span>
          <span className="text-muted-foreground">${p.extra.used} of ${p.extra.limit}</span>
        </div>
      )}
      {p.alarm && <Problem warn={p.alarm.tone === "warn"} title={p.alarm.text} className="mt-1" />}
    </div>
  )
}

function Provider({ reads, mark }) {
  const p = reads[0]
  const stale = [...new Set(reads.map((r) => r.stale).filter(Boolean))]
  const sub = [p.plan, p.account ? "" : on(p), p.updatedAgo ? `updated ${p.updatedAgo}` : ""].filter(Boolean).join(" · ")
  const drawn = mark && /^data:image\/svg\+xml,/.test(mark.image)
  return (
    <section className="flex min-w-0 flex-col gap-4">
      <div className="flex items-center gap-3">
        {drawn && <AppIcon app={{ name: p.id, title: p.title, image: mark.image, image_kind: mark.kind }} size={32} />}
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{p.title}</h2>
          {sub && <Hint>{sub}</Hint>}
        </div>
      </div>
      <Grid min="20rem" max={4} gap="1.5rem" className="items-start">
        {reads.map((r) => <Account key={r.account || r.id} p={r} several={reads.length > 1} />)}
      </Grid>
      {stale.length > 0 && <Problem warn title={`Can’t read ${p.title} now`} text={stale.join("; ")} />}
    </section>
  )
}

function Waiting() {
  const [late, setLate] = useState(false)
  useEffect(() => {
    const t = setTimeout(() => setLate(true), 100)
    return () => clearTimeout(t)
  }, [])
  if (!late) return null
  return (
    <Grid min="22rem" max={4}>
      {[0, 1].map((i) => <Skeleton key={i} className="h-56 rounded-xl" />)}
    </Grid>
  )
}

function Body() {
  const [state, setState] = useState({})
  useEffect(() => {
    const run = () =>
      one.call("status", { format: "json" }).then(
        (data) => setState({ data }),
        (error) => setState((s) => (s.data ? s : { error })),
      )
    one.on("changed", run)
    run()
  }, [])

  const { data, error } = state
  if (error) return <LoadFailure problem={one.plain(error)} />
  if (!data) return <Waiting />
  if (data.problem) return <Problem title="Can’t read your AI plans" text={data.problem} />
  if (!data.providers || !data.providers.length) return <Empty>No AI sign-ins on your devices.</Empty>
  const ids = [...new Set(data.providers.map((p) => p.id))]
  return (
    <div className="flex flex-col gap-8">
      {ids.map((id) => <Provider key={id} reads={data.providers.filter((p) => p.id === id)} mark={(data.marks || {})[id]} />)}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<div className="flex flex-col gap-4"><LargeTitle>AI Usage</LargeTitle><Body /></div>)
