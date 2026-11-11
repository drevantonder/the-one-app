// Insights: an OpenTelemetry viewer, opening as an APM does: every service's rate, errors and duration, then a service's
// operations, then their traces. It knows OTEL's shapes (a span's id, parent, name, service, time, duration and status; a
// histogram's count, sum, errors and buckets; attributes as keys and values) and no particular signal: everything it
// lists, filters by and groups by comes from what the `insights` tool gives it.
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/insights.html.
import { useEffect, useMemo, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { quantile, LAST_BOUND } from "./quantile.js"
import { Button, Empty, Icon, Input, LargeTitle, ListSkeleton, LoadFailure, Problem, Section, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Sparkline, Split, Tabs, TabsList, TabsTrigger, useWidth } from "@one/ui"

const HOUR = 3600000
const ALL = "*"
const RANGES = [["1h", "1 Hour", HOUR], ["6h", "6 Hours", 6 * HOUR], ["1d", "24 Hours", 24 * HOUR], ["7d", "7 Days", 168 * HOUR], ["30d", "30 Days", 720 * HOUR], ["90d", "90 Days", 2160 * HOUR]]
const STATS = { sum: "Sum", count: "Count", avg: "Average", p95: "P95" }
// As much as the store gives of each table at once.
const MOST_ROWS = 10000
// The most traces a list finds at once.
const MOST_TRACES = 200
const MOST_SERIES = 8
// The most things Insights opens by saying are failing.
const MOST_FAILING = 3
const OTHER = "Other"
// The most places a time chart stretches a short history over, so a few hours of data still reads as a chart.
const LEAST_HOURS = 12
// A metric's own columns, which aren't attributes to group by.
const COLUMNS = ["count", "sum", "hour", "name", "type", "unit", "buckets", "instance"]
// A span's or log's columns the row itself shows.
const SHOWN = ["name", "body", "service", "kind", "duration", "status", "severity_number", "severity_text", "id"]
// Categorical hues in a fixed order, each validated for its own theme (light, dark).
const HUES = [["#2a78d6", "#3987e5"], ["#eb6834", "#d95926"], ["#1baf7a", "#199e70"], ["#eda100", "#c98500"], ["#e87ba4", "#d55181"], ["#008300", "#008300"], ["#6250d6", "#9085e9"], ["#e34948", "#e66767"]]
const hue = (i) => HUES[i % HUES.length][matchMedia("(prefers-color-scheme: dark)").matches ? 1 : 0]
const ERROR = "var(--destructive)"
const MUTED = "var(--muted-foreground)"

const when = (ms) => new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric", minute: "2-digit", second: "2-digit" })
const hourOf = (ms) => new Date(ms).toLocaleString(undefined, { month: "short", day: "numeric", hour: "numeric" })
const number = (n) => (n == null ? "–" : new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(n))
const percent = (r) => (r == null ? "–" : r === 0 ? "0%" : r < 0.001 ? "<0.1%" : `${(r * 100).toFixed(r < 0.1 ? 1 : 0)}%`)
const took = (ms) => (ms == null ? "–" : ms >= 60000 ? `${(ms / 60000).toFixed(1)}m` : ms >= 1000 ? `${(ms / 1000).toFixed(ms >= 10000 ? 0 : 1)}s` : `${Math.round(ms)}ms`)
const p95 = (ms) => (ms > LAST_BOUND ? `>${took(LAST_BOUND)}` : took(ms))
const ago = (ms) => {
  const s = Math.max(0, Math.round((Date.now() - ms) / 1000))
  return s < 60 ? `${s}s ago` : s < 3600 ? `${Math.round(s / 60)}m ago` : s < 86400 ? `${Math.round(s / 3600)}h ago` : `${Math.round(s / 86400)}d ago`
}
const text = (v) => (typeof v === "string" ? v : JSON.stringify(v))
const sorted = (values) => [...new Set(values)].sort((a, b) => String(a).localeCompare(String(b)))
const plural = (n, one) => `${number(n)} ${one}${n === 1 ? "" : "s"}`

// "1.5s", "200ms" or "2m" as milliseconds.
function duration(s) {
  const m = /^\s*([\d.]+)\s*(ms|s|m)?\s*$/.exec(s)
  return m ? Number(m[1]) * { ms: 1, s: 1000, m: 60000 }[m[2] || "ms"] : 0
}

const fields = (row) => {
  const { attrs, ...own } = row
  return Object.fromEntries(Object.entries({ ...own, ...(attrs || {}) }).filter(([, v]) => v !== null && v !== ""))
}

// Space-separated terms, as Jaeger's tags are: `key=value` is that field, anything else is found in any.
function matcher(query) {
  const terms = query.split(/\s+/).filter(Boolean).map((t) => t.toLowerCase())
  return (row) => {
    const lower = Object.entries(fields(row)).map(([k, v]) => [k.toLowerCase(), text(v).toLowerCase()])
    return terms.every((t) => {
      const at = t.indexOf("=")
      if (at > 0) return lower.some(([k, v]) => k === t.slice(0, at) && v === t.slice(at + 1))
      return lower.some(([k, v]) => k.includes(t) || v.includes(t))
    })
  }
}

function tracesOf(rows) {
  return [...Map.groupBy(rows, (s) => s.trace).entries()].map(([id, spans]) => {
    const ids = new Set(spans.map((s) => s.span))
    const roots = spans.filter((s) => !s.parent || !ids.has(s.parent)).sort((a, b) => a.time - b.time)
    const start = Math.min(...spans.map((s) => s.time))
    const end = Math.max(...spans.map((s) => s.time + s.duration))
    // Named by the request it was, which a span of trying devices can sit above.
    return { id, spans, root: spans.filter((s) => s.kind === "server").sort((a, b) => a.time - b.time)[0] || roots[0] || spans[0], start, ms: end - start, errors: spans.filter((s) => s.status === "error" && s.kind !== "client").length, services: sorted(spans.map((s) => s.service)) }
  }).sort((a, b) => b.start - a.start)
}

const colours = (services) => Object.fromEntries(sorted(services).map((s, i) => [s, hue(i)]))

const asked = (body) => one.api("POST", "/api/insights/insights", { raw: true, ...body })

// The newest traces with a span of what's asked, over the whole range, each whole. The store finds them, so what's most
// common never crowds out what's looked for: → null while they're read, then `{ traces }` or `{ problem }`.
function useTraces(range, { service, operation, least }, at) {
  const [got, setGot] = useState(null)
  useEffect(() => {
    let live = true
    setGot(null)
    const only = { ...(service && service !== ALL && { service }), ...(operation && operation !== ALL && { name: operation }), ...(least > 0 && { min_ms: Math.round(least) }) }
    asked({ since: range, limit: MOST_TRACES, parts: ["traces"], ...only }).then((r) => live && setGot({ traces: tracesOf(r.traces || []) }), (e) => live && setGot({ problem: one.plain(e) }))
    return () => { live = false }
  }, [range, service, operation, least, at])
  return got
}

// A trace's list as it's read: its traces, or that they're being read, or why they couldn't be.
function Found({ got, children }) {
  if (!got) return <ListSkeleton rows={3} />
  if (got.problem) return <LoadFailure problem={got.problem} />
  return children(got.traces)
}

function red(rows) {
  const count = rows.reduce((n, r) => n + r.count, 0), errors = rows.reduce((n, r) => n + (r.errors || 0), 0), sum = rows.reduce((n, r) => n + r.sum, 0)
  return { count, errors, sum, rate: count ? errors / count : null, avg: count ? sum / count : null, p95: quantile(rows, 0.95) }
}

// Each hour worth drawing: from the first with data, to now.
function hoursOf(rows, from, to) {
  const last = Math.floor(to / HOUR), first = Math.floor(from / HOUR)
  const earliest = Math.min(last, ...rows.map((r) => r.hour))
  const start = Math.max(first, Math.min(earliest, last - LEAST_HOURS + 1))
  return Array.from({ length: last - start + 1 }, (_, i) => start + i)
}

function Tip({ tip }) {
  if (!tip) return null
  return (
    <div className="pointer-events-none absolute z-10 max-w-72 rounded-md bg-popover px-2 py-1.5 text-xs whitespace-nowrap text-popover-foreground shadow-md" style={{ left: tip.x, top: tip.y, transform: `translate(${tip.flip ? "-100%" : "0"}, -100%)` }}>
      {tip.lines.map((l, i) => <div key={i} className={i ? "text-muted-foreground" : "font-medium"}>{l}</div>)}
    </div>
  )
}

function Axis({ ticks, width, left, top, height }) {
  return ticks.map(({ y, label }) => (
    <g key={label}>
      <line x1={left} x2={width} y1={top + y * height} y2={top + y * height} stroke="var(--border)" />
      <text x={left - 6} y={top + y * height + 4} textAnchor="end" fontSize="11" fill={MUTED}>{label}</text>
    </g>
  ))
}

const LEFT = 44, TOP = 8, BOTTOM = 20
const TimeLabels = ({ first, last, width, height, left = LEFT }) => (
  <>
    <text x={left} y={height - 5} fontSize="11" fill={MUTED}>{first}</text>
    <text x={width - 4} y={height - 5} fontSize="11" textAnchor="end" fill={MUTED}>{last}</text>
  </>
)

// Hourly bars, each `stacks` entry `{ name, colour, values }` one on another; `axis` to draw its scale and times.
function Bars({ hours, stacks, height = 96, axis = true, format = number }) {
  const [ref, width] = useWidth()
  const [at, setAt] = useState(null)
  const left = axis ? LEFT : 0, top = axis ? TOP : 2, bottom = axis ? BOTTOM : 2, plot = height - top - bottom
  const totals = hours.map((_, i) => stacks.reduce((n, s) => n + (s.values[i] || 0), 0))
  const most = Math.max(1, ...totals)
  const place = Math.max(1, width - left - 4) / hours.length
  const bar = Math.max(1, Math.min(place - 2, 20))
  const hover = (e) => setAt(Math.max(0, Math.min(hours.length - 1, Math.floor((e.clientX - e.currentTarget.getBoundingClientRect().left - left) / place))))
  return (
    <div ref={ref} className="relative">
      <svg width={width} height={height} onMouseMove={hover} onMouseLeave={() => setAt(null)} role="img" aria-label={stacks.map((s) => s.name).join(", ")}>
        {axis && <Axis width={width} left={left} top={top} height={plot} ticks={[{ y: 0, label: format(most) }, { y: 1, label: "0" }]} />}
        {axis && <TimeLabels first={hourOf(hours[0] * HOUR)} last={hourOf(hours.at(-1) * HOUR)} width={width} height={height} />}
        {at != null && <rect x={left + at * place} y={top} width={place} height={plot} fill="var(--muted)" />}
        {hours.map((_, i) => {
          let y = top + plot
          return stacks.map((s) => {
            const v = s.values[i] || 0
            if (!v) return null
            const h = Math.max(2, (v / most) * plot)
            y -= h
            return <rect key={s.name} x={left + i * place + (place - bar) / 2} y={y} width={bar} height={Math.max(1, h - 1)} rx="2" fill={s.colour} />
          })
        })}
      </svg>
      {at != null && <Tip tip={{ x: left + at * place + place / 2, y: top + 12, flip: at > hours.length / 2, lines: [hourOf(hours[at] * HOUR), ...stacks.map((s) => `${s.name}: ${format(s.values[at] || 0)}`)] }} />}
    </div>
  )
}

// Lines over the hours, `null` a gap, a lone point a dot, with a crosshair.
function Lines({ hours, series, height = 96, format = number }) {
  const [ref, width] = useWidth()
  const [at, setAt] = useState(null)
  const plot = height - TOP - BOTTOM
  const most = Math.max(1, ...series.flatMap((s) => s.values.filter((v) => v != null)))
  const x = (i) => LEFT + ((i + 0.5) / hours.length) * Math.max(1, width - LEFT - 4)
  const y = (v) => TOP + plot - (v / most) * plot
  const path = (values) => {
    let d = "", pen = false
    values.forEach((v, i) => {
      if (v == null) return (pen = false)
      d += `${pen ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`
      pen = true
    })
    return d
  }
  const lone = (values, i) => values[i] != null && values[i - 1] == null && values[i + 1] == null
  function hover(e) {
    const r = e.currentTarget.getBoundingClientRect()
    setAt(Math.max(0, Math.min(hours.length - 1, Math.floor(((e.clientX - r.left - LEFT) / Math.max(1, width - LEFT - 4)) * hours.length))))
  }
  const listed = at == null ? [] : series.map((s) => ({ ...s, v: s.values[at] })).filter((s) => s.v != null).sort((a, b) => b.v - a.v).slice(0, MOST_SERIES + 1)
  return (
    <div ref={ref} className="relative">
      <svg width={width} height={height} onMouseMove={hover} onMouseLeave={() => setAt(null)} role="img" aria-label={series.map((s) => s.name).join(", ")}>
        <Axis width={width} left={LEFT} top={TOP} height={plot} ticks={[{ y: 0, label: format(most) }, { y: 1, label: "0" }]} />
        <TimeLabels first={hourOf(hours[0] * HOUR)} last={hourOf(hours.at(-1) * HOUR)} width={width} height={height} />
        {series.map((s) => <path key={s.name} d={path(s.values)} fill="none" stroke={s.colour} strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />)}
        {series.flatMap((s) => s.values.map((v, i) => (lone(s.values, i) ? <circle key={`${s.name}${i}`} cx={x(i)} cy={y(v)} r="4" fill={s.colour} stroke="var(--background)" strokeWidth="2" /> : null)))}
        {at != null && <line x1={x(at)} x2={x(at)} y1={TOP} y2={TOP + plot} stroke={MUTED} />}
        {listed.map((s) => <circle key={s.name} cx={x(at)} cy={y(s.v)} r="4" fill={s.colour} stroke="var(--background)" strokeWidth="2" />)}
      </svg>
      {at != null && <Tip tip={{ x: x(at) + (at > hours.length / 2 ? -10 : 10), y: TOP + 34, flip: at > hours.length / 2, lines: [hourOf(hours[at] * HOUR), ...listed.map((s) => `${s.name}: ${format(s.v)}`)] }} />}
    </div>
  )
}

function Panel({ title, value, note, children }) {
  return (
    <div className="flex min-w-0 flex-col gap-1">
      <div className="text-sm text-muted-foreground">{title}</div>
      <div className="flex items-baseline gap-2"><span className="text-2xl font-semibold tabular-nums">{value}</span>{note && <span className="text-sm text-muted-foreground">{note}</span>}</div>
      {children}
    </div>
  )
}

// Rate, errors and duration of some spans, over the hours they ran.
function Health({ rows, from, to }) {
  const hours = hoursOf(rows, from, to)
  const byHour = Map.groupBy(rows, (r) => r.hour)
  const at = (f) => hours.map((h) => (byHour.has(h) ? f(red(byHour.get(h))) : null))
  const all = red(rows)
  const failed = at((r) => r.errors)
  return (
    <div className="grid grid-cols-[repeat(auto-fit,minmax(15rem,1fr))] gap-x-8 gap-y-5">
      <Panel title="Requests" value={number(all.count)}>
        <Bars hours={hours} stacks={[{ name: "Requests", colour: hue(0), values: at((r) => r.count) }]} />
      </Panel>
      <Panel title="Errors" value={percent(all.rate)} note={plural(all.errors, "error")}>
        <Bars hours={hours} stacks={[{ name: "Errors", colour: ERROR, values: failed }]} />
      </Panel>
      <Panel title="Duration" value={p95(all.p95)} note="P95">
        <Lines hours={hours} format={took} series={[{ name: "P95", colour: hue(0), values: at((r) => (r.p95 == null ? null : Math.min(r.p95, LAST_BOUND))) }]} />
      </Panel>
    </div>
  )
}

// A table whose columns sort, as APMs' are. A column: `{ key, label, cell, value, right, wide }`, `wide` only where there's room.
function Table({ columns, rows, open, start, label }) {
  const [sort, setSort] = useState(start)
  const by = columns.find((c) => c.key === sort.key)
  const ordered = [...rows].sort((a, b) => {
    const x = by.value(a) ?? -Infinity, y = by.value(b) ?? -Infinity
    return (x < y ? -1 : x > y ? 1 : 0) * (sort.down ? -1 : 1)
  })
  const hide = (c) => (c.wide ? "hidden @2xl:table-cell" : "")
  return (
    <div className="@container">
      <table className="w-full border-collapse text-sm" aria-label={label}>
        <thead>
          <tr className="text-muted-foreground">
            {columns.map((c) => (
              <th key={c.key} aria-sort={sort.key === c.key ? (sort.down ? "descending" : "ascending") : undefined} className={`px-2 py-1.5 font-normal ${c.right ? "text-right" : "text-left"} ${hide(c)}`}>
                {c.label && <button type="button" className="inline-flex cursor-pointer items-center gap-1 border-0 bg-transparent p-0 text-inherit" onClick={() => setSort({ key: c.key, down: sort.key === c.key ? !sort.down : c.right })}>
                  {c.label}{sort.key === c.key && <Icon name={sort.down ? "caret-down" : "caret-up"} className="size-3" />}
                </button>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {ordered.map((r) => (
            <tr key={r.id} onClick={() => open(r)} className="cursor-pointer border-0 border-t border-solid border-border hover:bg-muted">
              {columns.map((c, i) => (
                <td key={c.key} className={`px-2 py-2 ${c.right ? "text-right tabular-nums" : ""} ${hide(c)}`}>
                  {i === 0 ? <button type="button" className="cursor-pointer border-0 bg-transparent p-0 text-left text-inherit">{c.cell(r)}</button> : c.cell(r)}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

const redColumns = (name, trend) => [
  { key: "name", label: name, value: (r) => r.name, cell: (r) => <span className="font-medium break-all">{r.name}</span> },
  { key: "count", label: "Requests", right: true, value: (r) => r.count, cell: (r) => number(r.count) },
  { key: "errors", label: "Errors", right: true, value: (r) => r.rate, cell: (r) => <span className={r.errors ? "text-destructive" : "text-muted-foreground"}>{percent(r.rate)}</span> },
  { key: "avg", label: "Average", right: true, wide: true, value: (r) => r.avg, cell: (r) => took(r.avg) },
  { key: "p95", label: "P95", right: true, value: (r) => r.p95, cell: (r) => p95(r.p95) },
  ...(trend ? [{ key: "trend", label: "", wide: true, value: (r) => r.count, cell: (r) => <Sparkline values={r.trend} className="h-5 w-24 text-muted-foreground" /> }] : []),
]

function grouped(rows, key, hours) {
  return [...Map.groupBy(rows, key).entries()].map(([name, g]) => {
    const byHour = Map.groupBy(g, (r) => r.hour)
    const counts = hours.map((h) => (byHour.get(h) || []).reduce((n, r) => n + r.count, 0))
    const most = Math.max(1, ...counts)
    return { id: name, name, ...red(g), trend: counts.map((c) => (c / most) * 100) }
  }).sort((a, b) => b.count - a.count)
}

// What broke, one row for each thing that broke where it did, whichever builds it ran in: the newest of its message and trace.
function failures(rows) {
  return [...Map.groupBy(rows, (r) => `${r.service}|${r.name}|${r.status}|${r.type}|${r.frame}`).entries()].map(([id, group]) => {
    const newest = group.reduce((a, b) => (b.last > a.last ? b : a))
    return { ...newest, id, count: group.reduce((n, r) => n + r.count, 0), first: Math.min(...group.map((r) => r.first)) }
  })
}

// What a failure said, as what's read first: its type and where only if it said nothing.
const failureCell = (p) => (
  <span className="wrap-anywhere">
    <span className="block font-medium">{p.message || p.type}</span>
    <span className="block text-muted-foreground"><span className="@2xl:hidden">{p.service} {p.name} · </span>{p.message ? `${p.type} ` : ""}{p.frame}</span>
  </span>
)
const failureColumns = [
  { key: "failure", label: "Failure", value: (p) => p.message || p.type, cell: failureCell },
  { key: "service", label: "Service", wide: true, value: (p) => p.service, cell: (p) => <span className="wrap-anywhere">{p.service}</span> },
  { key: "name", label: "Operation", wide: true, value: (p) => p.name, cell: (p) => <span className="wrap-anywhere">{p.name}</span> },
  { key: "count", label: "Count", right: true, value: (p) => p.count, cell: (p) => number(p.count) },
  { key: "last", label: "Last", right: true, value: (p) => p.last, cell: (p) => <span className="whitespace-nowrap text-muted-foreground" title={when(p.last)}>{ago(p.last)}</span> },
]

// What's breaking now, first of all: nothing, when nothing is.
function Failing({ rows, to, service, open }) {
  const now = failures(rows.filter((r) => r.status === "error" && (!service || r.service === service) && to - r.last <= HOUR)).sort((a, b) => b.count - a.count).slice(0, MOST_FAILING)
  if (!now.length) return null
  return <Table label="Failing now" columns={failureColumns} rows={now} start={{ key: "count", down: true }} open={open} />
}

function Services({ rows, errors, range, at, from, to, open: [service, setService], colour, showTrace, showTraces, showFailure }) {
  const hours = hoursOf(rows, from, to)
  if (service) return <Service rows={rows} errors={errors} range={range} at={at} from={from} to={to} service={service} close={() => setService(null)} colour={colour} showTrace={showTrace} showTraces={showTraces} showFailure={showFailure} />
  if (!rows.length) return <Empty>Nothing recorded in this range.</Empty>
  // Each service on its own, never a total: one request is a server span in each service it goes through.
  return (
    <div className="flex flex-col gap-6">
      <Failing rows={errors} to={to} open={showFailure} />
      <Table label="Services" columns={redColumns("Service", true)} rows={grouped(rows, (r) => r.service, hours)} start={{ key: "errors", down: true }} open={(r) => setService(r.name)} />
    </div>
  )
}

function Service({ rows, errors, range, at, from, to, service, close, colour, showTrace, showTraces, showFailure }) {
  const hours = hoursOf(rows, from, to)
  const mine = rows.filter((r) => r.service === service)
  const got = useTraces(range, { service }, at)
  const recent = got?.traces?.slice(0, 12) || []
  return (
    <div className="flex flex-col gap-6">
      <div><Button variant="ghost" onClick={close}><Icon name="caret-left" />Services</Button></div>
      <h2 className="m-0 break-all text-xl font-semibold">{service}</h2>
      <Failing rows={errors} to={to} service={service} open={showFailure} />
      <Health rows={mine} from={from} to={to} />
      <Section title="Operations">
        <Table label="Operations" columns={redColumns("Operation", true)} rows={grouped(mine, (r) => r.name, hours)} start={{ key: "errors", down: true }} open={(r) => showTraces({ service, operation: r.name })} />
      </Section>
      {recent.length > 0 && (
        <Section title="Recent Traces" action={<Button variant="ghost" onClick={() => showTraces({ service, operation: ALL })}>All Traces</Button>}>
          <TraceTable traces={recent} open={showTrace} colour={colour} />
        </Section>
      )}
    </div>
  )
}

function TraceTable({ traces, open, colour }) {
  const longest = Math.max(1, ...traces.map((t) => t.ms))
  const columns = [
    { key: "name", label: "Trace", value: (t) => t.name, cell: (t) => <span className="flex min-w-0 items-center gap-2"><span className="size-2 shrink-0 rounded-full" style={{ background: t.errors ? ERROR : colour[t.root.service] }} /><span className="min-w-0 break-all"><span className="text-muted-foreground">{t.root.service}</span> <span className="font-medium">{t.root.name}</span></span></span> },
    { key: "spans", label: "Spans", right: true, wide: true, value: (t) => t.spans.length, cell: (t) => t.spans.length },
    { key: "ms", label: "Duration", right: true, value: (t) => t.ms, cell: (t) => <span className="inline-flex items-center justify-end gap-2"><span className="hidden h-1 w-20 overflow-hidden rounded-sm bg-muted @2xl:block"><span className="block h-1 rounded-sm" style={{ width: `${Math.max(2, (t.ms / longest) * 100)}%`, background: t.errors ? ERROR : MUTED }} /></span><span className={t.errors ? "text-destructive" : ""}>{took(t.ms)}</span></span> },
    { key: "start", label: "Started", right: true, value: (t) => t.start, cell: (t) => <span className="text-muted-foreground" title={when(t.start)}>{ago(t.start)}</span> },
  ]
  return <Table label="Traces" columns={columns} rows={traces.map((t) => ({ ...t, name: `${t.root.service} ${t.root.name}` }))} start={{ key: "start", down: true }} open={(t) => open(t.id, t)} />
}

// Duration against time on a log scale, as durations differ by orders of magnitude: one dot a trace.
function Scatter({ traces, open, colour }) {
  const [ref, width] = useWidth()
  const [tip, setTip] = useState(null)
  const height = 150, plot = height - TOP - BOTTOM
  const times = traces.map((t) => t.start)
  const first = Math.min(...times), last = Math.max(...times, first + 60000)
  const lows = traces.map((t) => Math.max(1, t.ms))
  const floor = Math.floor(Math.log10(Math.min(...lows))), ceil = Math.max(floor + 1, Math.ceil(Math.log10(Math.max(...lows))))
  const x = (t) => LEFT + 6 + ((t - first) / (last - first)) * Math.max(1, width - LEFT - 14)
  const y = (ms) => TOP + plot - ((Math.log10(Math.max(1, ms)) - floor) / (ceil - floor)) * plot
  const ticks = Array.from({ length: ceil - floor + 1 }, (_, i) => ({ y: i / (ceil - floor), label: took(10 ** (ceil - i)) }))
  return (
    <div ref={ref} className="relative">
      <svg width={width} height={height} role="img" aria-label="Traces by time and duration">
        <Axis width={width} left={LEFT} top={TOP} height={plot} ticks={ticks} />
        <TimeLabels first={when(first)} last={when(last)} width={width} height={height} />
        {traces.map((t) => (
          <g key={t.id} className="cursor-pointer" onClick={() => open(t.id, t)} onMouseEnter={() => setTip({ x: x(t.start), y: y(t.ms) - 8, flip: x(t.start) > width / 2, lines: [`${t.root.service} ${t.root.name}`, took(t.ms), when(t.start)] })} onMouseLeave={() => setTip(null)}>
            <circle cx={x(t.start)} cy={y(t.ms)} r="10" fill="transparent" />
            <circle cx={x(t.start)} cy={y(t.ms)} r="4" fill={t.errors ? ERROR : colour[t.root.service]} stroke="var(--background)" strokeWidth="1.5" />
          </g>
        ))}
      </svg>
      <Tip tip={tip} />
    </div>
  )
}

function Pick({ label, value, set, all, options }) {
  return (
    <Select value={value} onValueChange={set}>
      <SelectTrigger aria-label={label}><SelectValue /></SelectTrigger>
      <SelectContent>
        <SelectItem value={ALL}>{all}</SelectItem>
        {options.map((o) => <SelectItem key={o} value={String(o)}>{String(o)}</SelectItem>)}
      </SelectContent>
    </Select>
  )
}

function Traces({ rows, range, at, filters: [filters, setFilters], query: [query, setQuery], open, colour }) {
  const [least, setLeast] = useState("")
  const [floor, setFloor] = useState(0)
  // What's typed is asked for once it's stopped changing.
  useEffect(() => {
    const later = setTimeout(() => setFloor(duration(least)), 500)
    return () => clearTimeout(later)
  }, [least])
  const { service, operation } = filters
  const got = useTraces(range, { service, operation, least: floor }, at)
  const found = useMemo(() => {
    const matches = matcher(query)
    return (got?.traces || []).filter((t) => t.spans.some(matches))
  }, [got, query])
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Pick label="Service" value={service} set={(v) => setFilters({ service: v, operation: ALL })} all="All Services" options={sorted(rows.map((r) => r.service))} />
        <Pick label="Operation" value={operation} set={(v) => setFilters({ service, operation: v })} all="All Operations" options={sorted(rows.filter((r) => service === ALL || r.service === service).map((r) => r.name))} />
        <Input className="w-28" aria-label="Minimum duration" placeholder="Min 1.2s" value={least} onChange={(e) => setLeast(e.target.value)} />
        <Input className="min-w-52 flex-1" aria-label="Tags" placeholder="Tags, like error.type=timeout" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <Found got={got}>
        {(traces) => found.length ? (
          <>
            <Scatter traces={found} open={open} colour={colour} />
            {traces.length >= MOST_TRACES && <p className="m-0 text-sm text-muted-foreground">The newest {MOST_TRACES} only.</p>}
            <TraceTable traces={found} open={open} colour={colour} />
          </>
        ) : <Empty>No traces match.</Empty>}
      </Found>
    </div>
  )
}

// One trace by its id, read whole: the one a list showed, or else read by its id.
function OpenTrace({ id, trace, back, colour }) {
  const [got, setGot] = useState(trace ? { trace } : null)
  useEffect(() => {
    if (trace) return
    let live = true
    asked({ trace_id: id, parts: ["spans"] }).then((r) => live && setGot({ trace: tracesOf(r.spans || [])[0] }), (e) => live && setGot({ problem: one.plain(e) }))
    return () => { live = false }
  }, [id])
  if (got?.trace) return <Trace trace={got.trace} back={back} colour={colour} />
  return (
    <div className="flex flex-col gap-6">
      <LargeTitle action={<Button variant="ghost" onClick={back}><Icon name="caret-left" />Back</Button>}>Trace</LargeTitle>
      {!got ? <ListSkeleton rows={3} /> : got.problem ? <LoadFailure problem={got.problem} /> : <Empty>Nothing of this trace has arrived yet: a span takes a minute or two.</Empty>}
    </div>
  )
}

function Fields({ row, skip = [] }) {
  const all = fields(row)
  const shown = (k, v) => (["time", "first", "last"].includes(k) && typeof v === "number" ? when(v) : text(v))
  return (
    <dl className="m-0 grid grid-cols-[minmax(0,12rem)_minmax(0,1fr)] gap-x-3 gap-y-1 font-mono text-xs">
      {Object.keys(all).filter((k) => !skip.includes(k)).sort().map((k) => [<dt key={k} className="break-all text-muted-foreground">{k}</dt>, <dd key={`${k}=`} className="m-0 whitespace-pre-wrap break-all">{shown(k, all[k])}</dd>])}
    </dl>
  )
}

const stackOf = (s) => s.attrs?.events?.[0]?.attributes?.["exception.stacktrace"]

// One trace, as a waterfall: each span under its parent, its bar where it ran, and what it says under it when opened.
function Trace({ trace, back, colour }) {
  const [closed, setClosed] = useState(() => new Set())
  // A failed trace opens at what failed.
  const [open, setOpen] = useState(() => trace.spans.find((s) => s.status === "error" && s.message)?.span ?? null)
  const kids = Map.groupBy(trace.spans, (s) => s.parent || "")
  const known = new Set(trace.spans.map((s) => s.span))
  const flat = []
  const walk = (s, depth) => {
    flat.push({ s, depth })
    if (closed.has(s.span)) return
    for (const k of (kids.get(s.span) || []).sort((a, b) => a.time - b.time)) walk(k, depth + 1)
  }
  trace.spans.filter((s) => !s.parent || !known.has(s.parent)).sort((a, b) => a.time - b.time).forEach((s) => walk(s, 0))
  const whole = Math.max(1, trace.ms)
  const toggle = (id) => setClosed((c) => { const n = new Set(c); n.has(id) ? n.delete(id) : n.add(id); return n })
  const grid = "grid grid-cols-1 gap-x-3 @2xl:grid-cols-[minmax(0,20rem)_minmax(0,1fr)]"
  return (
    <div className="@container flex flex-col gap-4">
      <LargeTitle action={<Button variant="ghost" onClick={back}><Icon name="caret-left" />Back</Button>}>{trace.root.name}</LargeTitle>
      <div>
        <p className="m-0 text-sm">{trace.services.join(", ")}</p>
        <p className="m-0 text-sm text-muted-foreground">{when(trace.start)} · {took(trace.ms)} · {plural(trace.spans.length, "span")}{trace.errors ? <span className="text-destructive"> · {plural(trace.errors, "error")}</span> : null}</p>
        <p className="m-0 break-all font-mono text-xs text-muted-foreground">{trace.id}</p>
      </div>
      <div className="flex flex-col">
        <div className={`${grid} border-0 border-b border-solid border-border pb-1 text-xs text-muted-foreground`}>
          <span className="hidden @2xl:block">Span</span>
          <div className="flex justify-between font-mono">{[0, 0.25, 0.5, 0.75, 1].map((p) => <span key={p}>{took(p * whole)}</span>)}</div>
        </div>
        {flat.map(({ s, depth }) => {
          const parent = (kids.get(s.span) || []).length > 0
          const shown = open === s.span
          return (
            <div key={s.span} className="border-0 border-b border-solid border-border">
              <div className={`${grid} cursor-pointer items-center py-1 hover:bg-muted`} onClick={() => setOpen(shown ? null : s.span)}>
                <div className="flex min-w-0 items-center gap-1 text-sm" style={{ paddingLeft: Math.min(depth, 8) * 14 }}>
                  {parent ? (
                    <button type="button" aria-label={closed.has(s.span) ? "Expand" : "Collapse"} onClick={(e) => (e.stopPropagation(), toggle(s.span))} className="grid size-5 shrink-0 cursor-pointer place-items-center border-0 bg-transparent p-0 text-muted-foreground">
                      <Icon name={closed.has(s.span) ? "caret-right" : "caret-down"} />
                    </button>
                  ) : <span className="size-5 shrink-0" />}
                  <span className="min-w-0 truncate"><span className={s.status === "error" ? "text-destructive" : ""}>{s.name}</span>{s.service !== trace.root.service && <span className="text-muted-foreground"> {s.service}</span>}</span>
                </div>
                <div className="relative h-4">
                  <div className="absolute inset-y-1 rounded-sm" style={{ left: `${((s.time - trace.start) / whole) * 100}%`, width: `max(2px, ${(s.duration / whole) * 100}%)`, background: s.status === "error" ? ERROR : colour[s.service] }} />
                  {(() => {
                    const start = ((s.time - trace.start) / whole) * 100, end = ((s.time - trace.start + s.duration) / whole) * 100
                    if (end <= 80) return <span className="absolute inset-y-0 flex items-center px-1 text-xs tabular-nums text-muted-foreground" style={{ left: `${end}%` }}>{took(s.duration)}</span>
                    if (start >= 20) return <span className="absolute inset-y-0 flex items-center px-1 text-xs tabular-nums text-muted-foreground" style={{ right: `${100 - start}%` }}>{took(s.duration)}</span>
                    return <span className="absolute inset-y-0 right-0 flex items-center px-1 text-xs tabular-nums text-primary-foreground">{took(s.duration)}</span>
                  })()}
                </div>
              </div>
              {shown && (
                <div className="flex flex-col gap-2 px-1 pb-3 pt-1">
                  {s.message && <p className="m-0 text-sm font-medium wrap-anywhere">{s.message}</p>}
                  {stackOf(s) && <pre className="m-0 font-mono text-xs whitespace-pre-wrap text-muted-foreground">{stackOf(s)}</pre>}
                  <Fields row={s} skip={[...SHOWN, "message", "events"]} />
                </div>
              )}
            </div>
          )
        })}
      </div>
    </div>
  )
}

// How many in each slice of time, red for the failed: the volume bar above a log search.
const failed = (r) => r.severity_number >= 17

function Volume({ rows }) {
  const [ref, width] = useWidth()
  const slices = 48, height = 48
  const first = Math.min(...rows.map((r) => r.time)), last = Math.max(first + 60000, ...rows.map((r) => r.time))
  const buckets = Array.from({ length: slices }, () => [0, 0])
  for (const r of rows) buckets[Math.min(slices - 1, Math.floor(((r.time - first) / (last - first)) * slices))][failed(r) ? 1 : 0]++
  const most = Math.max(1, ...buckets.map(([a, b]) => a + b))
  const w = width / slices
  return (
    <div ref={ref}>
      <svg width={width} height={height} role="img" aria-label="How many over time">
        {buckets.map(([ok, bad], i) => {
          const total = ((ok + bad) / most) * height, red = (bad / most) * height
          return (
            <g key={i}>
              {ok > 0 && <rect x={i * w + 1} width={Math.max(1, w - 2)} y={height - total} height={Math.max(1, total - red - 1)} rx="2" fill={MUTED} opacity=".5" />}
              {bad > 0 && <rect x={i * w + 1} width={Math.max(1, w - 2)} y={height - red} height={red} rx="2" fill={ERROR} />}
            </g>
          )
        })}
      </svg>
    </div>
  )
}

function Logs({ rows, more, query: [query, setQuery] }) {
  const [service, setService] = useState(ALL)
  const [open, setOpen] = useState(null)
  const found = useMemo(() => {
    const matches = matcher(query)
    return rows.map((r, id) => ({ ...r, id })).filter((r) => (service === ALL || r.service === service) && matches(r))
  }, [rows, service, query])
  // Only what differs from row to row is worth a place on the line.
  const varying = useMemo(() => {
    const seen = {}
    for (const r of found) for (const [k, v] of Object.entries(r.attrs || {})) (seen[k] ||= new Set()).add(text(v))
    return new Set(Object.keys(seen).filter((k) => seen[k].size > 1))
  }, [found])
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <Pick label="Service" value={service} set={setService} all="All Services" options={sorted(rows.map((r) => r.service))} />
        <Input className="min-w-52 flex-1" aria-label="Search" placeholder="Search, or key=value" value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      {found.length ? (
        <>
          <Volume rows={found} />
          <p className="m-0 text-sm text-muted-foreground">{plural(found.length, "log")}{more ? ", the newest only" : ""}</p>
          <div className="flex flex-col font-mono text-xs">
            {found.map((r) => (
              <div key={r.id} className="border-0 border-b border-solid border-border">
                <div className="flex cursor-pointer gap-3 py-1.5 hover:bg-muted" onClick={() => setOpen(open === r.id ? null : r.id)}>
                  <span className="shrink-0 tabular-nums text-muted-foreground">{when(r.time)}</span>
                  <span className={`w-10 shrink-0 ${failed(r) ? "text-destructive" : "text-muted-foreground"}`}>{r.severity_text}</span>
                  <span className="min-w-0 truncate">{r.service} {r.body} <span className="text-muted-foreground">{Object.entries(r.attrs || {}).filter(([k]) => varying.has(k)).map(([k, v]) => `${k}=${text(v)}`).join(" ")}</span></span>
                </div>
                {open === r.id && <div className="px-1 pb-3 pt-1"><Fields row={r} skip={SHOWN} /></div>}
              </div>
            ))}
          </div>
        </>
      ) : <Empty>No logs match.</Empty>}
    </div>
  )
}

/// Whether a metric's rows say how long something took: a histogram in ms.
const timed = (rows) => rows.some((r) => r.type === "histogram" && r.unit === "ms")
function statOver(rows, stat) {
  const r = red(rows)
  return { sum: r.sum, count: r.count, avg: r.avg, p95: r.p95 }[stat]
}
const zeroAt = (stat) => (stat === "avg" || stat === "p95" ? null : 0)
function seriesOf(rows, hours, stat, split) {
  const named = (r) => (split === ALL ? "All" : text(fields(r)[split] ?? "–"))
  const groups = Map.groupBy(rows, named)
  const total = (g) => g.reduce((n, r) => n + r.count, 0)
  const ranked = [...groups.entries()].sort((a, b) => total(b[1]) - total(a[1]))
  const stable = sorted([...groups.keys()])
  const line = (name, g, colour) => {
    const byHour = Map.groupBy(g, (r) => r.hour)
    return { name, colour, values: hours.map((h) => (byHour.has(h) ? statOver(byHour.get(h), stat) : zeroAt(stat))) }
  }
  const lines = ranked.slice(0, MOST_SERIES).map(([name, g]) => line(name, g, hue(stable.indexOf(name))))
  if (ranked.length > MOST_SERIES) lines.push(line(OTHER, ranked.slice(MOST_SERIES).flatMap(([, g]) => g), MUTED))
  return lines
}

function Metrics({ rows, from, to, more, query: [query, setQuery] }) {
  const [open, setOpen] = useState(null)
  const [stat, setStat] = useState("sum")
  const [split, setSplit] = useState(ALL)
  const shown = useMemo(() => rows.filter(matcher(query)), [rows, query])
  const search = <Input className="min-w-52 flex-1" aria-label="Filter" placeholder="Filter, or key=value" value={query} onChange={(e) => setQuery(e.target.value)} />
  if (open) {
    const these = shown.filter((r) => r.name === open)
    const hours = hoursOf(these, from, to)
    const keys = sorted(rows.filter((r) => r.name === open).flatMap((r) => Object.keys(fields(r)))).filter((k) => !COLUMNS.includes(k))
    const series = seriesOf(these, hours, stat, split)
    const shownAs = timed(these) && (stat === "avg" || stat === "p95") ? took : number
    return (
      <div className="flex flex-col gap-4">
        <div><Button variant="ghost" onClick={() => (setOpen(null), setSplit(ALL))}><Icon name="caret-left" />Metrics</Button></div>
        <h2 className="m-0 break-all text-xl font-semibold">{open}</h2>
        <div className="flex flex-wrap items-center gap-2">
          <Select value={stat} onValueChange={setStat}>
            <SelectTrigger aria-label="Statistic"><SelectValue /></SelectTrigger>
            <SelectContent>{Object.entries(STATS).map(([k, v]) => <SelectItem key={k} value={k}>{v}</SelectItem>)}</SelectContent>
          </Select>
          <Pick label="Group by" value={split} set={setSplit} all="No Grouping" options={keys} />
          {search}
        </div>
        {these.length ? (
          <>
            <Lines hours={hours} series={series} height={260} format={shownAs} />
            <table className="w-full max-w-2xl border-collapse text-sm">
              <thead><tr className="text-left text-muted-foreground"><th className="py-1 font-normal">Series</th><th className="py-1 text-right font-normal">Peak</th><th className="py-1 text-right font-normal">Latest</th></tr></thead>
              <tbody>
                {series.map((s) => {
                  const known = s.values.filter((v) => v != null)
                  return (
                    <tr key={s.name} className="border-0 border-t border-solid border-border">
                      <td className="py-1.5"><span className="flex items-center gap-2"><span className="size-2.5 shrink-0 rounded-full" style={{ background: s.colour }} /><span className="break-all">{s.name}</span></span></td>
                      <td className="py-1.5 text-right tabular-nums">{shownAs(known.length ? Math.max(...known) : null)}</td>
                      <td className="py-1.5 text-right tabular-nums">{shownAs(known.at(-1))}</td>
                    </tr>
                  )
                })}
              </tbody>
            </table>
          </>
        ) : <Empty>No data in this range.</Empty>}
      </div>
    )
  }
  const names = [...Map.groupBy(shown, (r) => r.name).entries()].sort((a, b) => a[0].localeCompare(b[0]))
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">{search}</div>
      {more && <p className="m-0 text-sm text-muted-foreground">The newest only.</p>}
      {names.length ? (
        <div className="grid grid-cols-[repeat(auto-fill,minmax(16rem,1fr))] gap-x-8 gap-y-5">
          {names.map(([name, g]) => {
            const hours = hoursOf(g, from, to)
            // How long something took reads as its P95; anything else as its sum.
            const stat = timed(g) ? "p95" : "sum"
            return (
              <button key={name} type="button" onClick={() => (setOpen(name), setStat(stat))} className="flex min-w-0 cursor-pointer flex-col gap-1 border-0 bg-transparent p-0 text-left text-foreground">
                <span className="flex items-baseline justify-between gap-2"><span className="min-w-0 truncate text-sm font-medium">{name}</span><span className="tabular-nums text-sm">{stat === "p95" ? `P95 ${took(statOver(g, stat))}` : number(statOver(g, stat))}</span></span>
                <Bars hours={hours} axis={false} height={44} stacks={[{ name, colour: hue(0), values: seriesOf(g, hours, stat, ALL)[0].values }]} />
              </button>
            )
          })}
        </div>
      ) : <Empty>No metrics match.</Empty>}
    </div>
  )
}

function Errors({ rows, showTraces, showTrace, open: [open, setOpen], query: [query, setQuery] }) {
  const [service, setService] = useState(ALL)
  const [status, setStatus] = useState("error")
  const found = useMemo(() => {
    const matches = matcher(query)
    return failures(rows.filter((r) => (service === ALL || r.service === service) && (status === ALL || r.status === status) && matches(r)))
  }, [rows, service, status, query])
  const chosen = found.find((r) => r.id === open)
  return (
    <Split
      title={chosen && `${chosen.type} ${chosen.frame}`}
      onClose={() => setOpen(null)}
      list={
        <div className="flex flex-col gap-4">
          <div className="flex flex-wrap items-center gap-2">
            <Pick label="Service" value={service} set={setService} all="All Services" options={sorted(rows.map((r) => r.service))} />
            <Pick label="Status" value={status} set={setStatus} all="All Statuses" options={sorted(rows.map((r) => r.status))} />
            <Input className="min-w-52 flex-1" aria-label="Search" placeholder="Search, or key=value" value={query} onChange={(e) => setQuery(e.target.value)} />
          </div>
          {found.length ? <Table label="Errors" columns={failureColumns} rows={found} start={{ key: "count", down: true }} open={(r) => setOpen(r.id)} /> : <Empty>No errors match.</Empty>}
        </div>
      }
      detail={chosen && (
        <div className="flex flex-col items-start gap-3">
          <Fields row={chosen} skip={["id"]} />
          {chosen.trace ? <Button variant="secondary" onClick={() => showTrace(chosen.trace)}>Open Latest Trace</Button> : <Button variant="secondary" onClick={() => showTraces(chosen)}>Show Traces</Button>}
        </div>
      )}
    />
  )
}

// What's failing of telemetry's own path, for an admin (the `insights` tool's `problem`): as `load` shows it.
const CANARY = { write: "This device can't send what it records.", read: "What's recorded can't be read back.", arriving: "What this device sends isn't coming back." }

function Insights() {
  const [data, setData] = useState(null)
  const [problem, setProblem] = useState(null)
  const [tab, setTab] = useState("Services")
  const [range, setRange] = useState("1d")
  const [loading, setLoading] = useState(false)
  // The trace open: its id, and the trace itself if a list showed it.
  const [opened, setOpened] = useState(null)
  const showTrace = (id, trace) => setOpened({ id, trace })
  // An app's page opens this at its app (`service`).
  const service = useState(one.params?.service || null)
  const filters = useState({ service: ALL, operation: ALL })
  const failure = useState(null)
  const queries = { Traces: useState(""), Logs: useState(""), Metrics: useState(""), Errors: useState("") }
  const loads = useRef(0)

  async function load({ check } = {}) {
    const mine = ++loads.current
    setLoading(true)
    try {
      // What's summed first, as it's what the first tab shows; the rows afterwards, which are many and most of the wait.
      const got = await asked({ since: range, limit: MOST_ROWS, check: !!check, parts: ["requests", "errors"] })
      if (mine !== loads.current) return
      setData({ ...got, at: Date.now() })
      setProblem(null)
      const rows = await asked({ since: range, limit: MOST_ROWS, parts: ["logs", "metrics"] })
      if (mine === loads.current) setData((had) => ({ ...had, ...rows }))
    } catch (e) {
      if (mine === loads.current) setProblem(one.plain(e))
    } finally {
      if (mine === loads.current) setLoading(false)
    }
  }
  useEffect(() => void load(), [range])

  if (!data) {
    return (
      <div className="flex flex-col gap-6">
        <LargeTitle>Insights</LargeTitle>
        {problem ? <LoadFailure problem={problem} retry={load} /> : <ListSkeleton rows={3} />}
      </div>
    )
  }

  const to = data.at, from = to - RANGES.find(([v]) => v === range)[2]
  const colour = colours(data.requests.map((r) => r.service))
  if (opened) return <OpenTrace key={opened.id} id={opened.id} trace={opened.trace} back={() => setOpened(null)} colour={colour} />
  const toTraces = (next) => (filters[1](next), setTab("Traces"))
  const more = (rows) => rows.length >= MOST_ROWS

  return (
    <div className="flex flex-col gap-4">
      <LargeTitle
        action={
          <span className="flex items-center gap-2">
            <Select value={range} onValueChange={setRange}>
              <SelectTrigger aria-label="Time range"><SelectValue /></SelectTrigger>
              <SelectContent>{RANGES.map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
            </Select>
            <Button variant="ghost" size="icon" aria-label="Refresh" disabled={loading} onClick={load}><Icon name="arrow-clockwise" /></Button>
          </span>
        }
      >Insights</LargeTitle>
      {problem && <LoadFailure problem={problem} retry={load} />}
      {data.problem && <Problem title="Insights has stopped arriving" text={`${CANARY[data.problem.kind] || CANARY.arriving} Since ${when(data.problem.since)}.`} action={{ label: "Check Again", run: () => load({ check: true }) }} />}
      <div>
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList><TabsTrigger value="Services">Services</TabsTrigger>{Object.keys(queries).map((t) => <TabsTrigger key={t} value={t}>{t}</TabsTrigger>)}</TabsList>
        </Tabs>
      </div>
      {tab === "Services" && <Services rows={data.requests} errors={data.errors} showFailure={(p) => (failure[1](p.id), setTab("Errors"))} range={range} at={data.at} from={from} to={to} open={service} colour={colour} showTrace={showTrace} showTraces={toTraces} />}
      {(tab === "Logs" || tab === "Metrics") && !data.logs && <ListSkeleton rows={3} />}
      {tab === "Traces" && <Traces rows={data.requests} range={range} at={data.at} filters={filters} query={queries.Traces} open={showTrace} colour={colour} />}
      {tab === "Logs" && data.logs && <Logs rows={data.logs} more={more(data.logs)} query={queries.Logs} />}
      {tab === "Metrics" && data.metrics && <Metrics rows={data.metrics} from={from} to={to} more={more(data.metrics)} query={queries.Metrics} />}
      {tab === "Errors" && <Errors rows={data.errors} open={failure} query={queries.Errors} showTrace={showTrace} showTraces={(p) => (queries.Traces[1](`error.type=${p.type}`), toTraces({ service: p.service, operation: ALL }))} />}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Insights />)
