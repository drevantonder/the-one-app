// Telemetry: what a person acts on of what the network recorded, laid out as a trace and
// metrics viewer is, cut down. The alerts first, then calls over time, what's failing,
// what's slowest and what AI cost, over the range picked above them, and last what's kept. A failure opens its trace beside the list, as a waterfall; a slow tool opens how
// long it took over time. What's fine is silent: with nothing failing, there's no Failing.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { call, changed, Empty, Icon, List, Problem, render, Row, Screen, Section, Skeleton, Split, Tabs, Text, View } from "ui"

/** A slow tool, or an app slow to open, as `telemetry_overview` answers it. */
interface Slowest {
  app: string
  tool?: string
  view?: string
  called: string
  p50: number
  p95: number
  p99: number
  count: number
  failed?: number
  should_be_task?: boolean
  over_time: (number | null)[]
}

/** A failure, as `telemetry_overview` answers it. */
interface Failure {
  where: string
  message?: string
  span?: boolean
  count: number
  at: number
  trace?: string
  /** The client it happened on, if it wasn't a server: "App on Andre's iPhone". */
  from?: string | null
  took?: number | null
}

/** A day of a model's use of AI. */
interface Spent {
  model: string
  input: number
  output: number
  cost?: number | null
}

/** What `telemetry_overview` answers for the range picked. */
interface Overview {
  calls: { calls: number; failed: number }[]
  failing: Failure[]
  slowest: Slowest[]
  spending: Spent[]
  names: Record<string, string>
  retention: { days: number; bytes: number; most_bytes: number }
  forwarding: string | null
}

/** A span of a trace, as `telemetry_trace` answers it. */
interface Span {
  name: string
  server: string
  start: number
  took: number
  depth: number
  outcome?: string
  message?: string
}

/** An alert, as `telemetry_alerts` answers it. */
interface Alert {
  id: number
  kind: string
  state: "firing" | "resolved"
  title: string
  text?: string | null
  began: number
  ended?: number | null
}

/** What `telemetry_alerts` answers: the alerts open or resolved in the last day, and each kind. */
interface Alerts {
  alerts: Alert[]
  kinds: { kind: string; about: string }[]
}

const RANGES = [
  ["1", "Hour"],
  ["24", "Day"],
  ["168", "Week"],
  ["336", "2 Weeks"],
]
const [hours, setHours] = createSignal("24")
const overview = createMemo(() => call("telemetry_overview", { hours: Number(hours()) }).then((text): Overview => JSON.parse(text)))
const alerts = createMemo(() => call("telemetry_alerts", {}).then((text): Alerts => JSON.parse(text)))
changed(() => (refresh(overview), refresh(alerts)))

/// What's open beside the list: a failure's trace, `{ trace }`, or a slow tool or app, `{ slow }`.
const [open, setOpen] = createSignal<{ trace?: string; where?: string; slow?: Slowest & { key: string } }>()

const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 })
const count = (n?: number) => compact.format(n ?? 0)
const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" })
/// How long ago `at` was, in the largest unit that's whole.
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -1) return "just now"
  if (minutes > -60) return ago.format(minutes, "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
/// How long something took: "0.4 ms", "12 ms", "1.2 s", "3 min".
function took(ms?: number | null) {
  if (ms == null) return "–"
  if (ms < 10) return `${ms.toFixed(1)} ms`
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`
  return `${Math.round(ms / 60_000)} min`
}
const units = ["bytes", "KB", "MB", "GB"]
function size(bytes: number) {
  let n = bytes, unit = 0
  while (n >= 1000 && unit < units.length - 1) (n /= 1000), unit++
  return `${n >= 100 || unit === 0 ? Math.round(n) : n.toFixed(1)} ${units[unit]}`
}
/// The host of an address, `https://host/path` as `host`.
const host = (address: string) => address.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/[/?#].*$/, "")
const dollars = (cost: number) => cost.toLocaleString(undefined, { style: "currency", currency: "USD", maximumFractionDigits: cost < 1 ? 4 : 2 })

/// A span's name as a person reads it: its app's title, then its tool.
/// One Core's own tools are named by themselves.
function named(where: string, names: Record<string, string>) {
  const [app, ...rest] = where.split(" ")
  if (!rest.length) return where
  return app === "one" ? rest.join(" ") : `${names[app] ?? app} › ${rest.join(" ")}`
}
/// A slow tool or app, as a person reads it.
const slowName = (slow: Slowest) => (slow.view ? `Opening ${slow.called}` : slow.app === "one" ? slow.tool : `${slow.called} › ${slow.tool}`)
/// A count of something, said as many or one.
const many = (n: number, one: string, more = `${one}s`) => `${count(n)} ${n === 1 ? one : more}`

/// A run of values, as the widgets' sparkline draws them, a bar to each, with a gap where there are none.
function Spark(props: { values: (number | null)[]; height?: number; width?: number | "100%" }) {
  const max = () => Math.max(...props.values.map((v) => v ?? 0), 1e-9)
  return (
    <View aria-hidden="true" style={{ flexDirection: "row", alignItems: "flex-end", gap: 1, width: props.width ?? 72, height: props.height ?? 24 }}>
      <For each={props.values}>
        {(value) => <View style={{ flexGrow: 1, flexBasis: 0, height: `${((value ?? 0) / max()) * 100}%`, minHeight: 1, backgroundColor: "foreground" }} />}
      </For>
    </View>
  )
}

/// Calls in each step of the range, the failed ones at the foot of each bar, under a line saying how many.
function Calls(props: { steps: Overview["calls"] }) {
  const max = () => Math.max(...props.steps.map((s) => s.calls), 1)
  const all = () => props.steps.reduce((sum, s) => sum + s.calls, 0)
  const failed = () => props.steps.reduce((sum, s) => sum + s.failed, 0)
  return (
    <View style={{ gap: 8, maxWidth: "readable" }}>
      <View role="img" aria-label={`${all()} calls, ${failed()} failed`} style={{ flexDirection: "row", alignItems: "flex-end", gap: 2, height: 72 }}>
        <For each={props.steps}>
          {(step) => (
            <View aria-hidden="true" style={{ flexGrow: 1, flexBasis: 0, height: `${(step.calls / max()) * 100}%`, minHeight: 1, borderRadius: "sm", overflow: "hidden" }}>
              <View style={{ flexGrow: step.calls - step.failed, flexBasis: 0, backgroundColor: "foreground", opacity: 0.22 }} />
              <View style={{ flexGrow: step.failed, flexBasis: 0, backgroundColor: "destructive" }} />
            </View>
          )}
        </For>
      </View>
      <Text style={{ color: "muted-foreground", font: "body" }}>
        {many(all(), "call")}<Show when={failed()}> · <Text style={{ color: "destructive" }}>{count(failed())} failed</Text></Show>
      </Text>
    </View>
  )
}

/// A trace whole, as a waterfall: each span under the one it's in, where it started and how long it took.
function Trace(props: { id: string; where: string; names: Record<string, string> }) {
  const trace = createMemo(() => call("telemetry_trace", { id: props.id, hours: 336 }).then((text): { spans: Span[] } => JSON.parse(text)))
  const total = () => Math.max(...trace().spans.map((s) => s.start + s.took), 0.001)
  const servers = () => new Set(trace().spans.map((s) => s.server)).size
  return (
    <Screen heading={named(props.where, props.names)} back>
      <Errored fallback={(error) => <Problem heading="Can't show this trace" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <Text style={{ color: "muted-foreground", font: "body", maxWidth: "readable" }}>
            {took(total())}
            <Show when={servers() > 1}> across {servers()} servers</Show>
          </Text>
          <View role="list" style={{ gap: 12, maxWidth: "readable" }}>
            <For each={trace().spans}>
              {(span) => (
                <View role="listitem" style={{ gap: 4, paddingLeft: span.depth * 12 }}>
                  <View style={{ flexDirection: "row", justifyContent: "space-between", gap: "gutter" }}>
                    <Text style={{ font: "body", color: span.outcome === "error" ? "destructive" : undefined }}>{named(span.name, props.names)}</Text>
                    <Text style={{ font: "body", color: "muted-foreground", fontVariantNumeric: "tabular-nums" }}>{took(span.took)}</Text>
                  </View>
                  <View aria-hidden="true" style={{ flexDirection: "row", height: 6, borderRadius: "full", backgroundColor: "muted", overflow: "hidden" }}>
                    <View style={{ width: `${(span.start / total()) * 100}%` }} />
                    <View style={{ width: `${(span.took / total()) * 100}%`, minWidth: 2, borderRadius: "full", backgroundColor: span.outcome === "error" ? "destructive" : "foreground" }} />
                  </View>
                  <Show when={servers() > 1 || span.message}>
                    <Text style={{ font: "footnote", color: span.outcome === "error" ? "destructive" : "muted-foreground" }}>{[servers() > 1 && span.server, span.message].filter(Boolean).join(" · ")}</Text>
                  </Show>
                </View>
              )}
            </For>
          </View>
        </Loading>
      </Errored>
    </Screen>
  )
}

/// How long a tool or an app took over the range: its 95th percentile over time, and its percentiles.
function Slow(props: { slow: Slowest }) {
  return (
    <Screen heading={slowName(props.slow)} back>
      <View style={{ maxWidth: "readable", gap: 4 }}>
        <Spark values={props.slow.over_time} width="100%" height={80} />
        <Text style={{ color: "muted-foreground", font: "body" }}>95th percentile, over time</Text>
      </View>
      <List>
        <Row heading="Median" end={took(props.slow.p50)} />
        <Row heading="95th Percentile" end={took(props.slow.p95)} />
        <Row heading="99th Percentile" end={took(props.slow.p99)} />
        <Row heading={props.slow.view ? "Opened" : "Calls"} end={count(props.slow.count)} />
        <Show when={props.slow.failed}>
          <Row heading="Failed" end={<Text style={{ color: "destructive" }}>{count(props.slow.failed)}</Text>} />
        </Show>
      </List>
    </Screen>
  )
}

/// What AI used, per model over the range: tokens in and out, and what it cost where the endpoint said.
function Spending(props: { days: Spent[] }) {
  const models = () => {
    const by = Map.groupBy(props.days, (day) => day.model)
    return [...by].map(([model, days]) => ({
      model,
      input: days.reduce((sum, d) => sum + d.input, 0),
      output: days.reduce((sum, d) => sum + d.output, 0),
      cost: days.some((d) => d.cost != null) ? days.reduce((sum, d) => sum + (d.cost ?? 0), 0) : null,
      days: days.map((d) => d.input + d.output).reverse(),
    }))
  }
  return (
    <Section heading="AI Spending">
      <Show when={models().length} fallback={<Empty>No AI was used.</Empty>}>
        <List>
          <For each={models()}>
            {(model) => (
              <Row
                heading={model.model}
                text={[`${count(model.input)} in`, `${count(model.output)} out`, model.cost != null && dollars(model.cost)].filter(Boolean).join(" · ")}
                end={<Show when={model.days.length > 1}><Spark values={model.days} /></Show>}
              />
            )}
          </For>
        </List>
      </Show>
    </Section>
  )
}

function Telemetry(props: { all: Overview }) {
  const all = () => props.all
  const failing = () => all().failing
  // Each slow tool or app by a key of its own, so the one that's open stays open as it's asked again.
  const slowest = () => all().slowest.map((slow) => ({ ...slow, key: slow.view ? `open|${slow.view}` : `${slow.app}|${slow.tool}` }))
  const opened = () => {
    const shown = open()
    return shown?.slow ? { slow: slowest().find((slow) => slow.key === shown.slow!.key) ?? shown.slow } : shown
  }
  return (
    <Split
      open={!!opened()}
      onClose={() => setOpen()}
      detail={
        <Show when={opened()} fallback={<Show when={failing().length}><Empty>Pick a failure to see its trace.</Empty></Show>}>
          {(shown) => (
            <Show when={shown().trace} fallback={<Slow slow={shown().slow!} />}>
              {(id) => <Trace id={id()} where={shown().where!} names={all().names} />}
            </Show>
          )}
        </Show>
      }
    >
      <Screen heading="Telemetry">
        <Tabs.Root value={hours()} onValueChange={({ value }) => (setOpen(), setHours(value!))}>
          <Tabs.List>
            <For each={RANGES}>{(range) => <Tabs.Trigger value={range[0]}>{range[1]}</Tabs.Trigger>}</For>
            <Tabs.Indicator />
          </Tabs.List>
        </Tabs.Root>
        <Show when={alerts().alerts.length}>
          <Section heading="Alerts">
            <List>
              <For each={alerts().alerts} keyed={(alert) => alert.id}>
                {(alert) => (
                  <Row
                    start={<View style={{ color: alert().state === "resolved" ? undefined : "destructive" }}><Icon name={alert().state === "resolved" ? "check" : "warning-circle"} /></View>}
                    heading={alert().title}
                    text={alert().text ?? undefined}
                    end={alert().ended ? `Resolved ${since(alert().ended!)}` : since(alert().began)}
                  />
                )}
              </For>
            </List>
          </Section>
        </Show>
        <Calls steps={all().calls} />
        <Show when={failing().length}>
          <Section heading="Failing">
            <List>
              <For each={failing()} keyed={(failure) => `${failure.where}|${failure.message}`}>
                {(failure) => (
                  <Row
                    start={<View style={{ color: "destructive" }}><Icon name="warning-circle" /></View>}
                    heading={failure().span ? named(failure().where, all().names) : failure().where}
                    text={[failure().from, failure().message || "Failed", failure().took != null && failure().took! >= 1000 && `for ${took(failure().took)}`].filter(Boolean).join(" · ")}
                    end={`${failure().count > 1 ? `${many(failure().count, "time")} · ` : ""}${since(failure().at)}`}
                    current={open()?.trace === failure().trace}
                    onClick={failure().trace ? () => setOpen({ trace: failure().trace, where: failure().where }) : undefined}
                  />
                )}
              </For>
            </List>
          </Section>
        </Show>
        <Show when={slowest().length}>
          <Section heading="Slowest">
            <List>
              <For each={slowest()} keyed={(slow) => slow.key}>
                {(slow) => (
                  <Row
                    heading={slowName(slow())}
                    text={`p95 ${took(slow().p95)} · ${slow().view ? `opened ${many(slow().count, "time")}` : many(slow().count, "call")}${slow().should_be_task ? " · should be a task" : ""}`}
                    end={<Show when={slow().over_time.filter((v) => v != null).length > 1}><Spark values={slow().over_time} /></Show>}
                    current={open()?.slow?.key === slow().key}
                    onClick={() => setOpen({ slow: slow() })}
                  />
                )}
              </For>
            </List>
          </Section>
        </Show>
        <Spending days={all().spending} />
        <Text style={{ color: "muted-foreground", font: "body", maxWidth: "readable" }}>
          {`Kept for ${all().retention.days} days, up to ${size(all().retention.most_bytes)}. ${size(all().retention.bytes)} now.`}
          <Show when={all().forwarding}>{(to) => ` A copy goes to ${host(to())}.`}</Show>
        </Text>
      </Screen>
    </Split>
  )
}

render(() => (
  <Errored fallback={(error) => <Screen heading="Telemetry"><Problem heading="Can't show Telemetry" text={error() as Error} /></Screen>}>
    <Loading fallback={<Screen heading="Telemetry"><Skeleton rows={6} /></Screen>}>
      <Telemetry all={overview()} />
    </Loading>
  </Errored>
))
