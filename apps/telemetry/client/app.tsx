// Telemetry: what a person acts on of what the network recorded, laid out as a trace and
// metrics viewer is, cut down. The alerts first, then calls over time, what's failing,
// what's slowest and what AI cost, over the range picked above them, and last what's kept. A failure opens its trace beside the list, as a waterfall; a slow tool opens how
// long it took over time. What's fine is silent: with nothing failing, there's no Failing.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { call, changed, Empty, Icon, List, Problem, render, Row, Screen, Section, since, size, Skeleton, Split, styles, Tabs, Text, tokens, View } from "ui"
import { sizes } from "./sizes.ts"

const look = styles({
  spark: (width: number | "100%", height: number) => ({ flexDirection: "row", alignItems: "flex-end", gap: tokens.length.hairline, width, height }),
  /// A bar of a chart, as tall as its share of the tallest.
  bar: (height: `${number}%`) => ({ flexGrow: 1, flexBasis: 0, height, minHeight: tokens.length.hairline }),
  sparkBar: { backgroundColor: tokens.colour.label },
  calls: { gap: tokens.grid[2], maxWidth: tokens.length.readable },
  chart: { flexDirection: "row", alignItems: "flex-end", gap: tokens.grid[0.5], height: sizes.chart, borderBottomWidth: tokens.length.hairline, borderColor: tokens.colour.quaternaryLabel },
  axis: { flexDirection: "row", justifyContent: "space-between" },
  callsBar: { borderRadius: tokens.radius.small, overflow: "hidden" },
  grows: (flexGrow: number) => ({ flexGrow, flexBasis: 0 }),
  fine: { backgroundColor: tokens.colour.label, opacity: 0.22 },
  wrongFill: { backgroundColor: tokens.colour.red },
  note: { color: tokens.colour.secondaryLabel },
  readableNote: { color: tokens.colour.secondaryLabel, maxWidth: tokens.length.readable },
  wrong: { color: tokens.colour.red },
  spans: { gap: tokens.grid[3], maxWidth: tokens.length.readable },
  span: { gap: tokens.grid[1] },
  // A span's words are indented under its parent's; its track is the whole width, so every start lines up.
  indent: (paddingStart: number) => ({ paddingStart }),
  message: { maxWidth: tokens.length.readable },
  spanHeading: { flexDirection: "row", justifyContent: "space-between", gap: tokens.length.gutter },
  took: { color: tokens.colour.secondaryLabel },
  track: { flexDirection: "row", height: tokens.grid[1.5], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.quinaryLabel, overflow: "hidden" },
  width: (width: `${number}%`) => ({ width }),
  tookBar: { minWidth: tokens.grid[0.5], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.label },
  footnote: { font: tokens.type.caption, color: tokens.colour.secondaryLabel },
  slow: { maxWidth: tokens.length.readable, gap: tokens.grid[1] },
})

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
  calls: { at: number; calls: number; failed: number }[]
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
  /** Whether a client's, a phone's app or the CLI, rather than a server's. */
  client?: boolean
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
const [open, setOpen] = createSignal<{ failure?: Failure; slow?: Slowest & { key: string } }>()
const keyOf = (failure: Failure) => `${failure.where}|${failure.message}`

const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 })
const count = (n?: number) => compact.format(n ?? 0)
/// How long something took: "0.4 ms", "12 ms", "1.2 s", "3 min".
function took(ms?: number | null) {
  if (ms == null) return "–"
  if (ms < 10) return `${ms.toFixed(1)} ms`
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`
  return `${Math.round(ms / 60_000)} min`
}
/// The host of an address, `https://host/path` as `host`.
const host = (address: string) => address.replace(/^[a-z][a-z0-9+.-]*:\/\//i, "").replace(/[/?#].*$/, "")
const dollars = (cost: number) => cost.toLocaleString(undefined, { style: "currency", currency: "USD", maximumFractionDigits: cost < 1 ? 4 : 2 })

/// A span's name as a person reads it: its app's title, then its tool.
/// One Core's own tools are named by themselves.
function named(where: string, names: Record<string, string>) {
  const [app, ...rest] = where.replace(/^tools\/call (?=[^ ]+_)/, "").replace(/^([^ _]+)_/, "$1 ").split(" ")
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
    <View aria-hidden="true" style={look.spark(props.width ?? 72, props.height ?? 24)}>
      <For each={props.values}>
        {(value) => <View style={[look.bar(`${((value ?? 0) / max()) * 100}%`), look.sparkBar]} />}
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
    <View style={look.calls}>
      <View role="img" aria-label={`${all()} calls, ${failed()} failed`} style={look.chart}>
        <For each={props.steps}>
          {(step) => (
            <View aria-hidden="true" style={[look.bar(`${(step.calls / max()) * 100}%`), look.callsBar]}>
              <View style={[look.grows(step.calls - step.failed), look.fine]} />
              <View style={[look.grows(step.failed), look.wrongFill]} />
            </View>
          )}
        </For>
      </View>
      <Show when={props.steps.length}>
        <View style={look.axis}>
          <Text style={look.footnote}>{since(props.steps[0].at, "narrow")}</Text>
          <Text style={look.footnote}>Now</Text>
        </View>
      </Show>
      <Text style={look.note}>
        {many(all(), "call")}<Show when={failed()}> · <Text style={look.wrong}>{count(failed())} failed</Text></Show>
      </Text>
    </View>
  )
}

/// A trace whole, as a waterfall: each span under the one it's in, where it started and how long it took.
function Trace(props: { id: string; names: Record<string, string> }) {
  const trace = createMemo(() => call("telemetry_trace", { id: props.id, hours: 336 }).then((text): { spans: Span[] } => JSON.parse(text)))
  const total = () => Math.max(...trace().spans.map((s) => s.start + s.took), 0.001)
  // A client's span is no server: the app or the CLI that asked.
  const servers = () => new Set(trace().spans.filter((s) => !s.client).map((s) => s.server)).size
  return (
    <Errored fallback={(error) => <Problem heading="Can't show this trace" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={4} />}>
        <Text style={look.readableNote}>
          {took(total())}
          <Show when={servers() > 1}> across {servers()} servers</Show>
        </Text>
        <View role="list" style={look.spans}>
          <For each={trace().spans}>
            {(span) => (
              <View role="listitem" style={look.span}>
                <View style={[look.spanHeading, look.indent(span.depth * 12)]}>
                  <Text style={[span.outcome === "error" && look.wrong]}>{named(span.name, props.names)}</Text>
                  <Text style={look.took}>{took(span.took)}</Text>
                </View>
                <View aria-hidden="true" style={look.track}>
                  <View style={look.width(`${(span.start / total()) * 100}%`)} />
                  <View style={[look.width(`${(span.took / total()) * 100}%`), look.tookBar, span.outcome === "error" && look.wrongFill]} />
                </View>
                <Show when={(servers() > 1 && !span.client) || span.client || span.message}>
                  <Text style={[look.footnote, look.indent(span.depth * 12), span.outcome === "error" && look.wrong]}>{[(servers() > 1 || span.client) && span.server, span.message].filter(Boolean).join(" · ")}</Text>
                </Show>
              </View>
            )}
          </For>
        </View>
      </Loading>
    </Errored>
  )
}

/// A failure: its whole message, where and how often it happened, and the trace it was in where there is one.
function Failed(props: { failure: Failure; names: Record<string, string> }) {
  const failure = () => props.failure
  return (
    <Screen heading={failure().span ? named(failure().where, props.names) : failure().where} back>
      <Text style={[look.message, look.wrong]}>{failure().message || "Failed"}</Text>
      <List>
        <Show when={failure().from}>{(from) => <Row heading="On" end={from()} />}</Show>
        <Row heading="Times" end={count(failure().count)} />
        <Row heading="Last" end={since(failure().at)} />
        <Show when={failure().took != null && failure().took! >= 1000}>
          <Row heading="Took" end={took(failure().took)} />
        </Show>
      </List>
      <Show when={failure().trace}>
        {(id) => (
          <Section heading="Trace">
            <Trace id={id()} names={props.names} />
          </Section>
        )}
      </Show>
    </Screen>
  )
}

/// How long a tool or an app took over the range: its 95th percentile over time, and its percentiles.
function Slow(props: { slow: Slowest }) {
  return (
    <Screen heading={slowName(props.slow)} back>
      <View style={look.slow}>
        <Spark values={props.slow.over_time} width="100%" height={80} />
        <Text style={look.note}>95th percentile, over time</Text>
      </View>
      <List>
        <Row heading="Median" end={took(props.slow.p50)} />
        <Row heading="95th Percentile" end={took(props.slow.p95)} />
        <Row heading="99th Percentile" end={took(props.slow.p99)} />
        <Row heading={props.slow.view ? "Opened" : "Calls"} end={count(props.slow.count)} />
        <Show when={props.slow.failed}>
          <Row heading="Failed" end={<Text style={look.wrong}>{count(props.slow.failed)}</Text>} />
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
    if (shown?.slow) return { slow: slowest().find((slow) => slow.key === shown.slow!.key) ?? shown.slow }
    if (shown?.failure) return { failure: failing().find((failure) => keyOf(failure) === keyOf(shown.failure!)) ?? shown.failure }
    return shown
  }
  return (
    <Split
      open={!!opened()}
      onClose={() => setOpen()}
      detail={
        <Show when={opened()} fallback={<Show when={failing().length || slowest().length}><Empty>{failing().length ? "Pick a failure to see what happened." : "Pick a tool to see how long it takes."}</Empty></Show>}>
          {(shown) => (
            <Show when={shown().failure} fallback={<Slow slow={shown().slow!} />}>
              {(failure) => <Failed failure={failure()} names={all().names} />}
            </Show>
          )}
        </Show>
      }
    >
      <Screen heading="Telemetry">
        <Tabs.Root value={hours()} onValueChange={(value) => (setOpen(), setHours(value))}>
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
                    start={<View style={[alert().state !== "resolved" && look.wrong]}><Icon name={alert().state === "resolved" ? "check" : "warning-circle"} /></View>}
                    heading={alert().title}
                    text={alert().text ?? undefined}
                    end={alert().ended ? `Resolved ${since(alert().ended!, "narrow")}` : since(alert().began, "narrow")}
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
                    start={<View style={look.wrong}><Icon name="warning-circle" /></View>}
                    heading={failure().span ? named(failure().where, all().names) : failure().where}
                    text={failure().message || "Failed"}
                    end={`${failure().count > 1 ? `${many(failure().count, "time")} · ` : ""}${since(failure().at, "narrow")}`}
                    current={!!open()?.failure && keyOf(open()!.failure!) === keyOf(failure())}
                    onPress={() => setOpen({ failure: failure() })}
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
                    onPress={() => setOpen({ slow: slow() })}
                  />
                )}
              </For>
            </List>
          </Section>
        </Show>
        <Spending days={all().spending} />
        <Text style={look.readableNote}>
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
