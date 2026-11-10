// Insights: what the network recorded of how things went, as far as this user may see it (the `insights` tool decides).
//
// A Settings screen written in React (see vault.jsx): built by @one/ui's build into dist/insights.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Empty, LargeTitle, List, ListRow, ListSkeleton, LoadFailure, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Split, Tabs, TabsList, TabsTrigger } from "@one/ui"

const api = one.api
const ALL = "*"
const NETWORK = "The One App"
const DAY = 86400000
const RANGES = { "1d": "24 Hours", "7d": "7 Days" }
// What a call's own span already covers.
const INSIDE_A_CALL = ["service.hop", "schedule.run"]
const STOPPED = { watchdog: "Stopped by its watchdog", abort: "It crashed" }

function when(ms) {
  const d = new Date(ms), time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const days = Math.round((new Date().setHours(0, 0, 0, 0) - new Date(ms).setHours(0, 0, 0, 0)) / DAY)
  return days === 0 ? `Today, ${time}` : days === 1 ? `Yesterday, ${time}` : `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })}, ${time}`
}
const seconds = (ms) => `${(ms / 1000).toFixed(1)} s`
const times = (n) => (n === 1 ? "Once" : `${n} times`)
const middle = (values) => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]
const unique = (values) => [...new Set(values)].join(", ")

function errorTitle(p) {
  const kind = p.type === "_OTHER" ? "Error" : p.type
  if (p.frame === "screen.undrawn") return "Screen never drew"
  if (p.frame.startsWith("stage:")) return `${kind} during ${p.frame.slice(6)}`
  return `${kind} in ${p.frame.split(":").slice(0, 2).join(":")}`
}
const whereIn = (tool) => (tool.startsWith("screen:") ? `${tool.slice(7)} screen` : tool)

function errors(data, title) {
  return data.fingerprints.filter((p) => p.fault === "internal").map((p) => {
    const seen = data.signals.filter((s) => s.status === "error" && s.app === p.app && (s.attrs["exception.stacktrace"] || "").startsWith(`at ${p.frame}`))
    return {
      id: `${p.app}|${p.tool}|${p.fingerprint}`,
      app: p.app,
      title: errorTitle(p),
      sub: [title(p.app), whereIn(p.tool)].filter(Boolean).join(" · "),
      fresh: Date.now() - p.first < DAY,
      trailing: String(p.count),
      facts: [["Seen", times(p.count)], ["First", when(p.first)], ["Last", when(p.last)], ["Build", p.build.slice(0, 8)], seen.length ? ["Devices", unique(seen.map((s) => s.device))] : null],
      code: seen[0]?.attrs["exception.stacktrace"],
    }
  })
}

function slow(data, title) {
  const groups = Map.groupBy(data.signals.filter((s) => s.kind === "span" && s.status === "ok" && s.attrs["one.tool"] && !INSIDE_A_CALL.includes(s.name)), (s) => s.name)
  return [...groups.values()].map((calls) => {
    const worst = calls.reduce((a, b) => (b.ms > a.ms ? b : a))
    const steps = worst.attrs["one.steps"] || []
    const longest = steps.reduce((a, b) => (b.ms > (a?.ms ?? -1) ? b : a), null)
    return {
      id: worst.name,
      app: worst.app,
      ms: middle(calls.map((c) => c.ms)),
      title: `${title(worst.app)} · ${worst.attrs["one.tool"]}`,
      sub: longest ? `Slowest in ${longest.step}` : undefined,
      trailing: seconds(middle(calls.map((c) => c.ms))),
      facts: [["Median", seconds(middle(calls.map((c) => c.ms)))], ["Worst", seconds(worst.ms)], ["Slow calls", String(calls.length)]],
      code: steps.map((s) => `${s.step} ${s.ms} ms`).join("\n"),
    }
  }).sort((a, b) => b.ms - a.ms)
}

function devices(data, nameOf) {
  const rows = []
  for (const s of data.signals) {
    const reason = s.attrs["one.reason"]
    if (s.name === "node.start" && STOPPED[reason]) {
      rows.push({ id: s.span, title: `${s.device} restarted`, sub: STOPPED[reason], facts: [["When", when(s.time)], ["Build", String(s.attrs["one.build"]).slice(0, 8)]] })
    } else if (s.name === "update.not_taken" && s.status === "error") {
      rows.push({ id: s.span, title: `${s.device} didn't take an update`, sub: reason, facts: [["When", when(s.time)], ["Part", s.attrs["one.part"]], ["Why", reason]] })
    } else if (s.name === "service.exit" && s.status === "error") {
      rows.push({ id: s.span, app: s.app, title: `${s.attrs["service.name"] || "A service"} stopped on ${s.device}`, sub: reason, facts: [["When", when(s.time)], ["Why", reason]] })
    }
  }
  const offline = Map.groupBy(data.signals.filter((s) => s.name === "device.offline"), (s) => s.attrs["one.node"])
  for (const [key, said] of offline) {
    const relay = said.filter((s) => s.attrs["one.says"] === "relay").length
    rows.push({
      id: key,
      title: `${nameOf(key)} went offline`,
      sub: `The relay said so ${times(relay).toLowerCase()}, other devices ${times(said.length - relay).toLowerCase()}`,
      trailing: `${said.length}×`,
      facts: [["Last", when(Math.max(...said.map((s) => s.time)))], ["Relay", times(relay)], ["Other devices", times(said.length - relay)]],
    })
  }
  return rows
}

const TABS = { Errors: "No errors.", Slow: "Nothing's been slow.", Devices: "Nothing's gone wrong with a device." }

function Insights() {
  const [data, setData] = useState(null)
  const [problem, setProblem] = useState(null)
  const [names, setNames] = useState({ apps: {}, devices: {} })
  const [tab, setTab] = useState("Errors")
  const [range, setRange] = useState("7d")
  const [app, setApp] = useState(ALL)
  const [open, setOpen] = useState(null)

  async function load() {
    try {
      const [got, state, listed] = await Promise.all([api("POST", "/v1/insights/insights", { since: range, limit: 500 }), api("GET", "/v1/state"), api("GET", "/v1/apps")])
      setData(got)
      setNames({ apps: Object.fromEntries(listed.map((a) => [a.name, a.title || a.name])), devices: Object.fromEntries((state.network?.devices || []).map((d) => [d.key, d.name])) })
      setProblem(null)
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  useEffect(() => void load(), [range])

  if (!data) {
    return (
      <div className="flex max-w-2xl flex-col gap-6">
        <LargeTitle>Insights</LargeTitle>
        {problem ? <LoadFailure problem={problem} retry={load} /> : <ListSkeleton rows={3} />}
      </div>
    )
  }

  const title = (name) => (name ? names.apps[name] || name : NETWORK)
  const rows = { Errors: errors(data, title), Slow: slow(data, title), Devices: devices(data, (key) => names.devices[key] || "A device") }
  const shown = rows[tab].filter((r) => app === ALL || r.app === app)
  const chosen = shown.find((r) => r.id === open)
  const apps = [...new Set(rows.Errors.concat(rows.Slow).map((r) => r.app))].sort((a, b) => title(a).localeCompare(title(b)))

  return (
    <Split
      title={chosen?.title}
      onClose={() => setOpen(null)}
      list={
        <div className="flex max-w-2xl flex-col gap-4">
          <LargeTitle>Insights</LargeTitle>
          <div className="flex flex-wrap items-center gap-2">
            <Tabs value={tab} onValueChange={(t) => (setTab(t), setOpen(null))}>
              <TabsList>{Object.keys(TABS).map((t) => <TabsTrigger key={t} value={t}>{t}</TabsTrigger>)}</TabsList>
            </Tabs>
            <Select value={app} onValueChange={(a) => (setApp(a), setOpen(null))}>
              <SelectTrigger aria-label="Which app"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value={ALL}>All Apps</SelectItem>
                {apps.map((a) => <SelectItem key={a} value={a}>{title(a)}</SelectItem>)}
              </SelectContent>
            </Select>
            <Select value={range} onValueChange={setRange}>
              <SelectTrigger aria-label="How far back"><SelectValue /></SelectTrigger>
              <SelectContent>{Object.entries(RANGES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
            </Select>
          </div>
          {shown.length ? (
            <List>
              {shown.map((r) => (
                <ListRow
                  key={r.id}
                  title={r.title}
                  sub={r.fresh ? <>{r.sub} · <span className="text-warning">New today</span></> : r.sub}
                  trailing={r.trailing}
                  selected={r.id === open}
                  onClick={() => setOpen(r.id)}
                />
              ))}
            </List>
          ) : <Empty>{TABS[tab]}</Empty>}
        </div>
      }
      detail={chosen && <Detail row={chosen} />}
    />
  )
}

function Detail({ row }) {
  return (
    <div className="flex flex-col gap-4">
      {row.sub && <p className="text-sm text-muted-foreground">{row.sub}</p>}
      <dl className="grid grid-cols-[6rem_1fr] gap-x-3 gap-y-1.5 text-sm">
        {row.facts.filter(Boolean).map(([label, value]) => [<dt key={label} className="text-muted-foreground">{label}</dt>, <dd key={`${label}:`} className="m-0">{value}</dd>])}
      </dl>
      {row.code && <pre className="m-0 overflow-x-auto font-mono text-xs leading-relaxed">{row.code}</pre>}
    </div>
  )
}

createRoot(document.getElementById("root")).render(<Insights />)
