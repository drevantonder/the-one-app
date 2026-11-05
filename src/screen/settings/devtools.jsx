// Developer's panel: tools for whoever builds the app, docked under a screen (or a sheet on a phone). Its one tool is
// Timing: what the requests behind a screen did, drawn as Routes: each device a lane, time left to right, and a
// request the path it took, from this device down to the one that answered (and any it passed it to) and back.
// The traces are this client's (`one.traces`, in developer mode); `params` say which screen is in front: its app
// and screen if it's an app's, and since when it showed.
//
// A client's own screen written in React (see vault.jsx): built by @one/ui's build into dist/devtools.html.
import { useEffect, useState } from "react"
import { createRoot } from "react-dom/client"
import { Button, Empty, Icon, Tabs, TabsList, TabsTrigger, cn, useWidth } from "@one/ui"

const p = one.params || {}

const ms = (n) => (n >= 1000 ? `${(n / 1000).toFixed(1)} s` : `${Math.max(0, Math.round(n))} ms`);
// Requests that start within this long of the last one's end are part of the same burst: opening a screen, say.
const BURST = 500;

const title = (r) => (r.what === "call" ? `${r.app}.${r.op}` : r.what === "page" ? "Page code" : r.path || "");
const mine = (r) => (p.app ? r.app === p.app : true) && r.at >= (p.since || 0) - 1000;

// The device names the traces know, by key: the nodes say theirs, and the install's others.
function names(rs) {
  const out = {};
  const add = (t) => {
    if (!t || typeof t !== "object") return;
    Object.assign(out, t.names || {});
    for (const s of t.steps || []) add(s.trace);
  };
  for (const r of rs) for (const f of r.fetches || []) {
    add(f.node);
    const answered = (f.tries || []).find((t) => t.result === "answered");
    if (answered && f.node?.device) out[answered.node] = f.node.device;
  }
  return out;
}

const SAID = { "no connection": "couldn't connect", "not taking requests": "took no requests", "no answer": "didn't answer", unreachable: "unreachable", lost: "lost the connection", failed: "failed", elsewhere: "said elsewhere" };

// One request as marks on lanes, times from its start: `hop` (from, to, a, b, kind) a line between lanes, `block`
// (lane, a, b, kind, label) time spent on one, and `parts` its steps as lines for the list.
function route(r, known) {
  const marks = [], parts = [];
  const name = (key) => known[key] || `a device (${String(key).slice(0, 6)})`;
  const me = "This device";
  let t = 0, used = 0;
  const wait = (r.waited || 0) + (r.network || 0);
  if (wait >= 1) {
    marks.push({ block: me, a: 0, b: wait, kind: "wait" });
    parts.push([r.waited ? "Waited for this device to be let in" : "Joined the network", wait]);
    t = used = wait;
  }
  if (r.source === "kept") {
    marks.push({ block: me, a: 0, b: Math.max(r.ms || 0, 2), kind: "code" });
    parts.push(["Shown from this device's copy", r.ms || 0]);
    return { marks, parts, end: r.ms || 0 };
  }
  // What a node did, from `at` on its lane: the devices it passed the call to, the other apps and services the app
  // asked while it ran (on this device or another), and its own steps.
  const node = (tr, lane, at) => {
    let listed = 0;
    for (const s of tr?.steps || []) {
      const asked = s.step === "device" && s.app && s.app !== r.app;
      if (!asked && !(s.step === "device" && s.result === "ran" && (s.here || s.device === lane))) listed += s.step === "logs" && s.ms < 5 ? 0 : s.ms || 0;
      const a = at + (s.at || 0), b = a + (s.ms || 0);
      if (asked) {
        const to = s.here ? lane : s.device || "a device";
        const warn = s.result !== "ran";
        if (to === lane) {
          node(s.trace, lane, a);
          parts.push([`${s.app} asked on ${lane}${warn ? `: ${SAID[s.result] || s.result}` : ""}`, s.ms || 0, warn]);
          continue;
        }
        const inner = s.trace?.ms || 0, rt = Math.max(0, (s.ms || 0) - inner) / 2;
        marks.push({ hop: [lane, to], a, b: a + rt, kind: warn ? "warn" : "net" });
        if (s.trace) node(s.trace, to, a + rt);
        marks.push({ hop: [to, lane], a: b - rt, b, kind: warn ? "warn" : "net" });
        parts.push([`${lane} asked ${s.app} on ${to}${warn ? `: ${SAID[s.result] || s.result}` : ""}`, s.ms || 0, warn]);
      } else if (s.step === "device") {
        const to = s.device || "a device";
        if (s.result === "ran" && (s.here || to === lane)) { node(s.trace, lane, at); continue; }
        const warn = s.result !== "ran" && s.result !== "needs keys";
        const inner = s.trace?.ms || 0, rt = Math.max(0, (s.ms || 0) - inner) / 2;
        marks.push({ hop: [lane, to], a, b: a + rt, kind: warn ? "warn" : "net" });
        if (s.trace) node(s.trace, to, a + rt);
        marks.push({ hop: [to, lane], a: b - rt, b, kind: warn ? "warn" : "net" });
        parts.push([warn ? `${lane} tried ${to}: ${SAID[s.result] || s.result}` : `${lane} passed it to ${to}`, s.ms || 0, warn]);
      } else if (s.step === "bucket" || s.step === "behind") {
        marks.push({ hop: [lane, "Bucket"], a, b: a + (s.ms || 0) / 2, kind: "bucket" }, { hop: ["Bucket", lane], a: a + (s.ms || 0) / 2, b, kind: "bucket" });
        parts.push([s.step === "bucket" ? `${lane} fetched ${s.found} of ${s.asks} from the bucket` : `${lane} asked the bucket first: ${s.result}`, s.ms || 0, s.step === "behind" && s.result !== "caught up"]);
      } else if (s.step === "data" || s.step === "open" || (s.step === "logs" && s.ms >= 5)) {
        marks.push({ block: lane, a, b, kind: "data" });
        parts.push([s.step === "open" ? `Opened ${s.app}'s data on ${lane}` : s.step === "logs" ? `Read who's who on ${lane}` : `Read ${lane}'s copy${s.new ? `, ${s.new} new steps` : ""}`, s.ms || 0]);
      } else if (s.step === "run") {
        marks.push({ block: lane, a, b, kind: "app" });
        parts.push([`${s.app} ran on ${lane}`, s.ms || 0]);
      }
    }
    // What it did that it didn't time on its own: finding the call's app and keys, writing its answer.
    const rest = (tr?.ms || 0) - listed;
    if (rest >= 1) parts.push([`Other work on ${lane}`, rest]);
  };
  // Each send: the devices it tried, one after another, and what the one that answered did. A node says only how
  // long it took itself, so the rest is the trip there and back, drawn as half each way.
  for (const f of r.fetches || []) {
    const handed = f.handed?.ms || 0;
    if (handed >= 1) (marks.push({ block: me, a: t, b: t + handed, kind: "wait" }), parts.push(["Opened the keys to hand with it", handed]), (used += handed));
    const start = t + handed;
    let end = start;
    for (const tr of f.tries || []) {
      const a = start + (tr.at || 0), b = a + (tr.ms || 0), lane = name(tr.node);
      end = Math.max(end, b);
      if (tr.connect) (marks.push({ block: me, a, b: a + tr.connect, kind: "wait" }), parts.push([`Connected to ${lane}`, tr.connect]), (used += tr.connect));
      const from = a + (tr.connect || 0);
      if (tr.result !== "answered") {
        marks.push({ hop: [me, lane], a: from, b: from + (b - from) / 2, kind: "warn" }, { hop: [lane, me], a: from + (b - from) / 2, b, kind: "warn" });
        parts.push([`${lane}: ${SAID[tr.result] || tr.result}, gave up`, tr.ms || 0, true]);
        used += (tr.ms || 0) - (tr.connect || 0);
        continue;
      }
      const inner = f.node?.ms || 0, rt = Math.max(0, b - from - inner);
      marks.push({ hop: [me, lane], a: from, b: from + rt / 2, kind: "net", label: rt });
      node(f.node, lane, from + rt / 2);
      marks.push({ hop: [lane, me], a: b - rt / 2, b, kind: "net" });
      parts.push([`There and back to ${lane}`, rt]);
      used += rt + inner;
    }
    t = end;
    if (f.client?.ms >= 1) (marks.push({ block: me, a: t, b: t + f.client.ms, kind: "wait" }), parts.push(["Found the keys it needed", f.client.ms]), (t += f.client.ms), (used += f.client.ms));
  }
  // What this device did that nothing above timed: picking a device to send it to, and reading the answer.
  if ((r.ms || 0) - used >= 1) parts.push(["Other work on this device", (r.ms || 0) - used]);
  if (r.failed) parts.push(["Failed", 0, true]);
  return { marks, parts, end: Math.max(t, r.ms || 0) };
}

// Bursts of requests, each those that overlap or follow within BURST of each other.
function bursts(rs) {
  const out = [];
  for (const r of rs) {
    const last = out.at(-1);
    if (last && r.at <= last.end + BURST) (last.rs.push(r), (last.end = Math.max(last.end, r.at + (r.ms || 0))));
    else out.push({ rs: [r], start: r.at, end: r.at + (r.ms || 0) });
  }
  return out;
}

// Its colours each name a kind of work, which only a developer reads: everywhere else colour is kept for "this needs you".
const COLOUR = { net: "#6e8fd8", data: "#5fb39a", app: "#e0b25c", code: "#a58bd6", bucket: "#d77f8f", wait: "#6b6f77", warn: "var(--warning)" };

function Timing() {
  const [scope, setScope] = useState("screen")
  const [picked, setPicked] = useState(null)
  // The traces, or false where Developer Mode is off; null until they're read.
  const [all, setAll] = useState(null)
  const [ref, width] = useWidth()
  // A sheet has its own Done.
  const raised = document.documentElement.hasAttribute("data-raised")

  useEffect(() => {
    // Its own background and edges: docked, it's flush with the window, its bar at the top as it scrolls.
    document.body.style.padding = "0 0 24px"
    const refresh = async () => {
      const traces = await one.traces()
      setAll((was) => (!traces ? false : was && traces.length === was.length && traces.at(-1)?.at === was.at(-1)?.at ? was : traces))
    }
    refresh()
    const t = setInterval(refresh, 1000)
    return () => clearInterval(t)
  }, [])

  const pickScope = (s) => (setScope(s), setPicked(null))

  return (
    <div ref={ref} className="tabular-nums">
      <div className="sticky top-0 z-10 flex h-9 items-center gap-4 border-b bg-card/90 px-3 backdrop-blur-xl">
        <span className="flex h-9 items-center text-sm font-semibold shadow-[inset_0_-2px_var(--foreground)]" aria-current="page">Timing</span>
        <Tabs value={scope} onValueChange={pickScope} className="ml-auto">
          <TabsList className="h-7">
            <TabsTrigger value="screen" className="text-xs">This Screen</TabsTrigger>
            <TabsTrigger value="all" className="text-xs">All Screens</TabsTrigger>
          </TabsList>
        </Tabs>
        {!raised && (
          <Button variant="ghost" size="icon-sm" aria-label="Close" onClick={() => one.close()}>
            <Icon name="x" />
          </Button>
        )}
      </div>
      {all === false ? (
        <Empty>Turn on Developer Mode in Settings › Developer to see what each screen asked for.</Empty>
      ) : all ? (
        <Drawn all={all} scope={scope} picked={picked} pick={setPicked} width={width} />
      ) : null}
    </div>
  )
}

/// The requests in `scope`: what to say about them, the routes of the burst picked, and its requests, then the others'.
function Drawn({ all, scope, picked, pick, width }) {
  const rs = all.filter((r) => scope === "all" || mine(r)).sort((a, b) => a.at - b.at)
  if (!rs.length) return <Empty>{scope === "all" ? "Nothing asked yet." : "This screen hasn't asked for anything since it opened."}</Empty>
  const known = names(all)
  const routes = new Map(rs.map((r) => [r, route(r, known)]))
  const warned = (r) => routes.get(r).parts.some(([, , w]) => w)
  const groups = bursts(rs)
  // The request picked, else the slowest of the latest burst.
  const sel = rs.find((r) => r.at === picked?.at && title(r) === picked?.title) || groups.at(-1).rs.reduce((a, b) => ((b.ms || 0) > (a.ms || 0) ? b : a))
  const g = groups.find((x) => x.rs.includes(sel))

  // What to say: the burst's length, and what took most of the picked request.
  const opening = scope === "screen" && g === groups[0] && p.title
  const total = g.end - g.start
  // About the slowest of the burst, whichever is picked.
  const slow = g.rs.reduce((a, b) => ((b.ms || 0) > (a.ms || 0) ? b : a))
  const biggest = routes.get(slow).parts.reduce((a, b) => (b[1] > a[1] ? b : a), ["", 0])
  const bad = g.rs.find(warned)

  const row = (r) => {
    const on = r === sel, kind = r.what === "page" ? "code" : "net"
    const from = r.source === "kept" ? "from this device's copy" : r.fetches?.at(-1)?.node?.device ? `${r.what === "page" ? "from" : "on"} ${r.fetches.at(-1).node.device}` : ""
    const warn = warned(r)
    return (
      <div key={`${r.at} ${title(r)}`}>
        <button
          type="button"
          aria-pressed={on}
          onClick={() => pick({ at: r.at, title: title(r) })}
          className={cn("grid w-full cursor-pointer grid-cols-[12px_1fr_auto_64px] items-center gap-2.5 border-b px-4 py-[7px] text-left text-[13px] text-muted-foreground", on && "bg-accent text-foreground")}
        >
          <span className="size-2.5 rounded-[3px]" style={{ background: warn ? COLOUR.warn : COLOUR[kind] }} />
          <span className={cn("min-w-0", warn && "text-warning")}>
            {r.what === "call" ? <code className="font-mono text-xs">{title(r)}</code> : title(r)} <span className="opacity-75">{from}</span>
          </span>
          <span className="opacity-75">at {ms(r.at - g.start)}</span>
          <span className={cn("text-right", warn && "text-warning")}>{ms(r.ms || 0)}</span>
        </button>
        {on && (
          <div className="border-b py-1.5 pr-4 pl-[38px] text-[13px] text-muted-foreground">
            {routes.get(r).parts.map(([w, n, warn], i) => (
              <div key={i} className={cn("flex justify-between gap-3 py-0.5", warn && "text-warning")}>
                <span>{w}</span>
                <span>{n ? ms(n) : ""}</span>
              </div>
            ))}
          </div>
        )}
      </div>
    )
  }

  const others = groups.filter((x) => x !== g).reverse()
  return (
    <>
      <p className="mx-4 mt-3.5 mb-1.5 max-w-[70ch] text-[15px]">
        {opening ? <>{p.title} opened in <b className="font-semibold">{ms(total)}</b>. </> : <>These took <b className="font-semibold">{ms(total)}</b>. </>}
        {bad ? (
          <span className="text-warning">{title(bad)} had a problem: {routes.get(bad).parts.find(([, , w]) => w)[0]}.</span>
        ) : biggest[1] >= 1 ? (
          <>The longest part of {title(slow)}'s {ms(slow.ms || 0)}: {biggest[0].replace(/^./, (c) => c.toLowerCase())}, <b className="font-semibold">{ms(biggest[1])}</b>.</>
        ) : null}
      </p>
      <Routes g={g} sel={sel} routes={routes} width={width} />
      <div className="mt-1 border-t">{g.rs.map(row)}</div>
      {others.map((o) => (
        <div key={o.start}>
          <div className="mx-4 mt-3.5 mb-1 text-xs text-muted-foreground">{new Date(o.start).toLocaleTimeString()}, {ms(o.end - o.start)}</div>
          <div className="border-t">{o.rs.map(row)}</div>
        </div>
      ))}
    </>
  )
}

/// A burst's requests as routes over the lanes of the devices they reached: the others faded, then the one picked,
/// with its times written on it.
function Routes({ g, sel, routes, width }) {
  // Lanes: this device, each device the burst reached in the order it was first reached, and the bucket last.
  const lanes = ["This device"]
  for (const r of g.rs) for (const m of routes.get(r).marks) for (const l of m.hop || [m.block]) if (l !== "Bucket" && !lanes.includes(l)) lanes.push(l)
  // The bucket only when something in the burst asked it.
  if (g.rs.some((r) => routes.get(r).marks.some((m) => m.hop?.includes("Bucket")))) lanes.push("Bucket")

  const W = Math.max(360, width), L = 110, R = 54, top = 26, gap = 42
  const span = Math.max(1, ...g.rs.map((r) => r.at - g.start + routes.get(r).end))
  const x = (t) => L + (t / span) * (W - L - R)
  const y = (lane) => top + 10 + lanes.indexOf(lane) * gap
  const H = y(lanes.at(-1)) + 22
  const step = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000].find((s) => span / s <= 6) || 60000
  const ticks = []
  for (let t = 0; t <= span; t += step) ticks.push(t)
  // Only the longest trip of the one picked is written on it: more would sit on top of each other.
  const longest = routes.get(sel).marks.filter((m) => m.label >= 1).reduce((a, m) => (!a || m.label > a.label ? m : a), null)
  const text = { fontSize: 11, fill: "var(--muted-foreground)" }

  return (
    <svg className="block font-sans" width={W} height={H} viewBox={`0 0 ${W} ${H}`} role="img" aria-label="Where each request went">
      {ticks.map((t) => (
        <g key={t}>
          <line x1={x(t)} y1={top - 6} x2={x(t)} y2={H - 8} stroke="var(--border)" />
          <text x={x(t)} y={top - 10} textAnchor={t ? "middle" : "start"} {...text}>{t ? ms(t) : 0}</text>
        </g>
      ))}
      {lanes.map((l) => (
        <g key={l}>
          <line x1={L - 8} y1={y(l)} x2={W - 6} y2={y(l)} stroke="var(--border)" strokeDasharray={l === "Bucket" ? "2 4" : undefined} />
          <text x="12" y={y(l) + 4} fontSize="12" fontWeight="600" fill="var(--foreground)">{l}</text>
        </g>
      ))}
      {[...g.rs.filter((r) => r !== sel), sel].map((r) => {
        const off = r.at - g.start, on = r === sel
        return (
          <g key={`${r.at} ${title(r)}`} opacity={on ? undefined : 0.28}>
            {routes.get(r).marks.map((m, i) => {
              const c = COLOUR[m.kind]
              if (!m.hop) return <rect key={i} x={x(off + m.a)} y={y(m.block) - 6} width={Math.max(3, x(off + m.b) - x(off + m.a))} height="12" rx="3" fill={c} />
              const [a, b] = m.hop.map(y)
              const mx = (x(off + m.a) + x(off + m.b)) / 2, my = (a + b) / 2, left = mx > W - 160
              return (
                <g key={i}>
                  <line x1={x(off + m.a)} y1={a} x2={x(off + m.b)} y2={b} stroke={c} strokeWidth={on ? 3.5 : 3} strokeLinecap="round" />
                  {on && m === longest && (
                    <text x={left ? mx - 8 : mx + 8} y={my} textAnchor={left ? "end" : undefined} fontSize="11" fontWeight="600" fill="var(--foreground)">{ms(m.label)} there and back</text>
                  )}
                </g>
              )
            })}
            {on && (
              <>
                <circle cx={x(off)} cy={y("This device")} r="4" fill="var(--foreground)" />
                <circle cx={x(off + routes.get(r).end)} cy={y("This device")} r="4" fill="var(--foreground)" />
              </>
            )}
          </g>
        )
      })}
    </svg>
  )
}

createRoot(document.getElementById("root")).render(<Timing />)
