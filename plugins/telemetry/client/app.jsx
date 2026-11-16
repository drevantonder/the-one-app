// Telemetry, a part of Settings: what a person acts on of what the network recorded, laid out as a trace and
// metrics viewer is, cut down. Calls over time first, then what's failing, what's slowest and what AI cost, over
// the range picked above them. A failure opens its trace beside the list, as a waterfall; a slow tool opens how
// long it took over time. What's fine is silent: with nothing failing, there's no Failing.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { call, changed, Empty, Icon, leave, List, Problem, Row, Screen, Section, Skeleton, Split, Tabs } from "ui"
import "./app.css"

const RANGES = [
  ["1", "Hour"],
  ["24", "Day"],
  ["168", "Week"],
  ["336", "2 Weeks"],
]
const [hours, setHours] = createSignal("24")
const overview = createMemo(() => call("telemetry_overview", { hours: Number(hours()) }).then(JSON.parse))
changed(() => refresh(overview))

/// What's open beside the list: a failure's trace, `{ trace }`, or a slow tool or app, `{ slow }`.
const [open, setOpen] = createSignal()

const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 })
const count = (n) => compact.format(n ?? 0)
const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" })
/// How long ago `at` was, in the largest unit that's whole.
function since(at) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -1) return "just now"
  if (minutes > -60) return ago.format(minutes, "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
/// How long something took: "0.4 ms", "12 ms", "1.2 s", "3 min".
function took(ms) {
  if (ms == null) return "–"
  if (ms < 10) return `${ms.toFixed(1)} ms`
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`
  return `${Math.round(ms / 60_000)} min`
}
const units = ["bytes", "KB", "MB", "GB"]
function size(bytes) {
  let n = bytes, unit = 0
  while (n >= 1000 && unit < units.length - 1) (n /= 1000), unit++
  return `${n >= 100 || unit === 0 ? Math.round(n) : n.toFixed(1)} ${units[unit]}`
}
const dollars = (cost) => cost.toLocaleString(undefined, { style: "currency", currency: "USD", maximumFractionDigits: cost < 1 ? 4 : 2 })

/// A span's name as a person reads it: the plugin's app's name, then its tool.
/// One Core's own tools are named by themselves.
function named(where, names) {
  const [plugin, ...rest] = where.split(" ")
  if (!rest.length) return where
  return plugin === "one" ? rest.join(" ") : `${names[plugin] ?? plugin} › ${rest.join(" ")}`
}
/// A slow tool or app, as a person reads it.
const slowName = (slow) => (slow.view ? `Opening ${slow.called}` : slow.plugin === "one" ? slow.tool : `${slow.called} › ${slow.tool}`)
/// A count of something, said as many or one.
const many = (n, one, more = `${one}s`) => `${count(n)} ${n === 1 ? one : more}`

/// A line of values, as the widgets' sparkline draws one, with a gap where there are none.
function Spark(props) {
  const height = 24
  const points = () => {
    const values = props.values.map((v) => v ?? 0)
    const max = Math.max(...values, 1e-9)
    return values.map((v, at) => `${(at / Math.max(values.length - 1, 1)) * 100},${height - 1 - (v / max) * (height - 2)}`).join(" ")
  }
  return (
    <svg class="spark" viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" aria-hidden="true">
      <polygon points={`${points()} 100,${height} 0,${height}`} />
      <polyline points={points()} vector-effect="non-scaling-stroke" />
    </svg>
  )
}

/// Calls in each step of the range, the failed ones at the foot of each bar, under a line saying how many.
function Calls(props) {
  const max = () => Math.max(...props.steps.map((s) => s.calls), 1)
  const all = () => props.steps.reduce((sum, s) => sum + s.calls, 0)
  const failed = () => props.steps.reduce((sum, s) => sum + s.failed, 0)
  return (
    <figure class="calls">
      <div class="bars" role="img" aria-label={`${all()} calls, ${failed()} failed`}>
        <For each={props.steps}>
          {(step) => (
            <i style={{ height: `${(step.calls / max()) * 100}%` }} title={`${step.calls} calls${step.failed ? `, ${step.failed} failed` : ""}`}>
              <b style={{ height: `${step.calls ? (step.failed / step.calls) * 100 : 0}%` }} />
            </i>
          )}
        </For>
      </div>
      <figcaption>
        {many(all(), "call")}<Show when={failed()}> · <span class="bad">{count(failed())} failed</span></Show>
      </figcaption>
    </figure>
  )
}

/// A trace whole, as a waterfall: each span under the one it's in, where it started and how long it took.
function Trace(props) {
  const trace = createMemo(() => call("telemetry_trace", { id: props.id, hours: 336 }).then(JSON.parse))
  const total = () => Math.max(...trace().spans.map((s) => s.start + s.took), 0.001)
  const servers = () => new Set(trace().spans.map((s) => s.server)).size
  return (
    <Screen heading={named(props.where, props.names)} back>
      <Errored fallback={(error) => <Problem heading="Can't show this trace" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={4} />}>
          <p class="hint">
            {took(total())}
            <Show when={servers() > 1}> across {servers()} servers</Show>
          </p>
          <ol class="waterfall">
            <For each={trace().spans}>
              {(span) => (
                <li class={{ failed: span.outcome === "error" }} style={{ "--depth": span.depth }}>
                  <div class="what">
                    <span class="name">{named(span.name, props.names)}</span>
                    <span class="took">{took(span.took)}</span>
                  </div>
                  <div class="track">
                    <i style={{ left: `${(span.start / total()) * 100}%`, width: `max(2px, ${(span.took / total()) * 100}%)` }} />
                  </div>
                  <Show when={servers() > 1 || span.message}>
                    <p class="about">{[servers() > 1 && span.server, span.message].filter(Boolean).join(" · ")}</p>
                  </Show>
                </li>
              )}
            </For>
          </ol>
        </Loading>
      </Errored>
    </Screen>
  )
}

/// How long a tool or an app took over the range: its 95th percentile over time, and its percentiles.
function Slow(props) {
  return (
    <Screen heading={slowName(props.slow)} back>
      <figure class="over-time">
        <Spark values={props.slow.over_time} />
        <figcaption>95th percentile, over time</figcaption>
      </figure>
      <List>
        <Row heading="Median" end={took(props.slow.p50)} />
        <Row heading="95th Percentile" end={took(props.slow.p95)} />
        <Row heading="99th Percentile" end={took(props.slow.p99)} />
        <Row heading={props.slow.view ? "Opened" : "Calls"} end={count(props.slow.count)} />
        <Show when={props.slow.failed}>
          <Row heading="Failed" end={<span class="bad">{count(props.slow.failed)}</span>} />
        </Show>
      </List>
    </Screen>
  )
}

/// What AI used, per model over the range: tokens in and out, and what it cost where the endpoint said.
function Spending(props) {
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

function Telemetry(props) {
  const all = () => props.all
  const failing = () => all().failing
  // Each slow tool or app by a key of its own, so the one that's open stays open as it's asked again.
  const slowest = () => all().slowest.map((slow) => ({ ...slow, key: slow.view ? `open|${slow.view}` : `${slow.plugin}|${slow.tool}` }))
  const opened = () => {
    const shown = open()
    return shown?.slow ? { slow: slowest().find((slow) => slow.key === shown.slow.key) ?? shown.slow } : shown
  }
  return (
    <Split
      open={!!opened()}
      onClose={() => setOpen()}
      detail={
        <Show when={opened()} fallback={<Show when={failing().length}><Empty>Pick a failure to see its trace.</Empty></Show>}>
          {(shown) => (
            <Show when={shown().trace} fallback={<Slow slow={shown().slow} />}>
              {(id) => <Trace id={id()} where={shown().where} names={all().names} />}
            </Show>
          )}
        </Show>
      }
    >
      <Screen heading="Telemetry" back onBack={leave}>
        <Tabs.Root value={hours()} onValueChange={({ value }) => (setOpen(), setHours(value))}>
          <Tabs.List>
            <For each={RANGES}>{(range) => <Tabs.Trigger value={range[0]}>{range[1]}</Tabs.Trigger>}</For>
            <Tabs.Indicator />
          </Tabs.List>
        </Tabs.Root>
        <Calls steps={all().calls} />
        <Show when={failing().length}>
          <Section heading="Failing">
            <List>
              <For each={failing()} keyed={(failure) => `${failure.where}|${failure.message}`}>
                {(failure) => (
                  <Row
                    start={<span class="bad"><Icon name="warning-circle" /></span>}
                    heading={failure().span ? named(failure().where, all().names) : failure().where}
                    text={failure().message || "Failed"}
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
                    text={`p95 ${took(slow().p95)} · ${slow().view ? `opened ${many(slow().count, "time")}` : many(slow().count, "call")}`}
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
        <p class="hint">
          Kept for {all().kept.days} days, up to {size(all().kept.most_bytes)}. {size(all().kept.bytes)} now.
        </p>
      </Screen>
    </Split>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Screen heading="Telemetry" back onBack={leave}><Problem heading="Can't show Telemetry" text={String(error().message ?? error())} /></Screen>}>
      <Loading fallback={<Screen heading="Telemetry" back onBack={leave}><Skeleton rows={6} /></Screen>}>
        <Telemetry all={overview()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
