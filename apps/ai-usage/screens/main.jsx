// AI Usage: a card for each provider, with each of its accounts' windows metered. The cards fill the room: more columns
// where there's more of it.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Card, Empty, Grid, Problem, Skeleton } from "@one/ui"

/// A bar for how much of a window is used: neutral until it needs you, amber from 70%, red from 85%.
function Bar({ used }) {
  const color = used >= 85 ? "bg-destructive" : used >= 70 ? "bg-warning" : "bg-foreground"
  return (
    <div role="progressbar" aria-valuenow={used} aria-valuemin={0} aria-valuemax={100} className="h-1.5 overflow-hidden rounded-full bg-muted">
      <div className={`h-full rounded-full ${color}`} style={{ width: `${Math.min(100, used)}%` }} />
    </div>
  )
}

/// One window: its name, the bar, what's used, and when it resets.
function Meter({ label, w }) {
  if (!w || w.used == null) return null
  return (
    <div className="grid grid-cols-[4.5rem_minmax(2rem,1fr)_2.5rem] items-center gap-x-3 gap-y-1 text-sm @sm:grid-cols-[4.5rem_minmax(2rem,1fr)_2.5rem_8rem]">
      <span className="truncate text-muted-foreground" title={label}>{label}</span>
      <Bar used={w.used} />
      <span className="text-right tabular-nums">{w.used}%</span>
      <span className="col-span-3 -mt-0.5 text-xs text-muted-foreground @sm:col-span-1 @sm:mt-0 @sm:text-right @sm:text-sm @sm:whitespace-nowrap">
        {w.countdown ? `resets ${w.countdown}` : ""}
      </span>
    </div>
  )
}

/// Which of your devices keep this sign-in.
const on = (p) => ((p.devices || []).length ? `on ${p.devices.join(", ")}` : "")

function Account({ p, several }) {
  const [team, email] = (p.account || "").split(" · ")
  const state = p.active && several ? "In use" : p.disabled ? "Not in rotation" : ""
  return (
    <div className="flex flex-col gap-2 border-t pt-4 first:border-t-0 first:pt-0">
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
  // Reads a machine couldn't renew: one problem, on the card they're in.
  const stale = [...new Set(reads.map((r) => r.stale).filter(Boolean))]
  const sub = [p.plan, p.account ? "" : on(p), p.updatedAgo ? `updated ${p.updatedAgo}` : ""].filter(Boolean).join(" · ")
  const drawn = mark && /^data:image\/svg\+xml,/.test(mark.image)
  return (
    <Card className="@container min-w-0 gap-4 p-4">
      <div className="flex items-center gap-3">
        {drawn && <AppIcon app={{ name: p.id, title: p.title, image: mark.image, image_kind: mark.kind }} size={32} />}
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{p.title}</h2>
          {sub && <p className="text-sm text-muted-foreground">{sub}</p>}
        </div>
      </div>
      {reads.map((r) => <Account key={r.account || r.id} p={r} several={reads.length > 1} />)}
      {stale.length > 0 && <Problem warn title={`Can't read ${p.title} now`} text={stale.join("; ")} />}
    </Card>
  )
}

/// A first answer that's slow shows as grey shapes, after a moment, so a quick one never flashes.
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

function Main() {
  const [state, setState] = useState({})
  useEffect(() => {
    // A later failure keeps what's shown, which was right a moment ago.
    const run = () =>
      one.call("status", { format: "json" }).then(
        (data) => setState({ data }),
        (error) => setState((s) => (s.data ? s : { error })),
      )
    one.on("changed", run)
    run()
  }, [])

  const { data, error } = state
  if (error) return <Problem title={one.plain(error).title} text={one.plain(error).next} />
  if (!data) return <Waiting />
  if (data.problem) return <Problem title="Can't read your AI plans" text={data.problem} />
  if (!data.providers || !data.providers.length) return <Empty>No AI sign-ins on your machines.</Empty>
  const ids = [...new Set(data.providers.map((p) => p.id))]
  return (
    <Grid min="22rem" max={4} className="items-start">
      {ids.map((id) => <Provider key={id} reads={data.providers.filter((p) => p.id === id)} mark={(data.marks || {})[id]} />)}
    </Grid>
  )
}

createRoot(document.getElementById("root")).render(<Main />)
