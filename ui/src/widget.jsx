import { createSignal, For, Match, Show, Switch as Which } from "solid-js"
import { Spinner } from "./button.jsx"

/// A number from a layout, or `or` where it isn't one.
const number = (value, or) => (value === null || value === "" || !Number.isFinite(Number(value)) ? or : Number(value))
const share = (value) => Math.max(0, Math.min(1, number(value, 0)))
const points = (value) => (number(value) === undefined ? undefined : `${number(value)}px`)
const figure = (value) => (typeof value === "number" ? value.toLocaleString(undefined, { maximumFractionDigits: 2 }) : String(value ?? "–"))

/// When something was, as a widget says it.
function time(at, style) {
  const date = new Date(number(at, 0))
  const ago = (Date.now() - date.getTime()) / 1000
  if (style === "date") return date.toLocaleDateString(undefined, { day: "numeric", month: "short" })
  if (style === "clock") return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const [count, unit] = Math.abs(ago) < 3600 ? [ago / 60, "minute"] : Math.abs(ago) < 86400 ? [ago / 3600, "hour"] : [ago / 86400, "day"]
  return new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" }).format(-Math.round(count), unit)
}

/// What every part may say of itself: how much room it takes, and whether it's faded.
const room = (part) => ({ grow: part.grow, dim: part.dim })
const sized = (part) => ({ width: points(part.width), flex: part.width ? "none" : undefined })

/// A line of values across a box `width` by `height`, as an SVG polyline's points.
const line = (values, max, width, height) =>
  values.map((value, at) => `${values.length > 1 ? (at / (values.length - 1)) * width : width / 2},${height - 1 - (Math.min(number(value, 0), max) / max) * (height - 2)}`).join(" ")

function Spark(props) {
  const values = () => (Array.isArray(props.part.values) ? props.part.values : [])
  const height = () => number(props.part.height, 28)
  return (
    <svg class={["spark", room(props.part)]} data-tone={props.part.tone} viewBox={`0 0 100 ${height()}`} preserveAspectRatio="none" style={{ width: points(props.part.width) ?? "100%", height: points(height()) }} role="img" aria-label={props.part.label}>
      <Show when={values().length} fallback={<line x1="0" y1={height() / 2} x2="100" y2={height() / 2} stroke-dasharray="2 3" vector-effect="non-scaling-stroke" />}>
        <polygon points={`${line(values(), number(props.part.max, Math.max(...values(), 1)), 100, height())} 100,${height()} 0,${height()}`} />
        <polyline points={line(values(), number(props.part.max, Math.max(...values(), 1)), 100, height())} vector-effect="non-scaling-stroke" />
      </Show>
    </svg>
  )
}

function Chart(props) {
  const values = () => (Array.isArray(props.part.values) ? props.part.values.map((value) => number(value, 0)) : [])
  const max = () => number(props.part.max, Math.max(...values(), 1))
  return (
    <div class={["chart", room(props.part)]} data-tone={props.part.tone} style={{ height: points(number(props.part.height, 70)) }}>
      <Show when={props.part.kind === "line"} fallback={
        <div class="bars">
          <For each={values()}>{(value) => <i style={{ height: `${(Math.min(value, max()) / max()) * 100}%` }} />}</For>
        </div>
      }>
        <svg viewBox="0 0 100 100" preserveAspectRatio="none">
          <polyline points={line(values(), max(), 100, 100)} vector-effect="non-scaling-stroke" />
        </svg>
      </Show>
      <Show when={Array.isArray(props.part.labels)}>
        <div class="labels">
          <For each={props.part.labels}>{(label) => <span>{String(label ?? "")}</span>}</For>
        </div>
      </Show>
    </div>
  )
}

/// A ring filled to a share, no thicker than a sixth of itself.
function Gauge(props) {
  const size = () => number(props.part.width, 46)
  const stroke = () => Math.min(5, size() / 6)
  const radius = () => (size() - stroke()) / 2
  const round = () => 2 * Math.PI * radius()
  return (
    <svg class="gauge" data-tone={props.part.tone} viewBox={`0 0 ${size()} ${size()}`} width={size()} height={size()} role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(share(props.part.value) * 100)} aria-label={props.part.label}>
      <circle cx={size() / 2} cy={size() / 2} r={radius()} stroke-width={stroke()} />
      <circle cx={size() / 2} cy={size() / 2} r={radius()} stroke-width={stroke()} stroke-linecap="round" stroke-dasharray={`${round() * share(props.part.value)} ${round()}`} transform={`rotate(-90 ${size() / 2} ${size() / 2})`} />
      <Show when={props.part.label && size() >= 36}>
        <text x={size() / 2} y={size() / 2 + 3.5} text-anchor="middle" font-size={Math.min(10, size() / 4.6)}>{props.part.label}</text>
      </Show>
    </svg>
  )
}

/// A button or a switch calls its plugin's tool, and shows that it's under way until the widget is drawn again.
function acts(props) {
  const [busy, setBusy] = createSignal(false)
  const act = async (inputs) => {
    setBusy(true)
    try {
      await props.onCall?.(props.part.tool, inputs)
    } finally {
      setBusy(false)
    }
  }
  return [busy, act]
}

function Press(props) {
  const [busy, act] = acts(props)
  return (
    <button type="button" class="press" data-tone={props.part.tone} disabled={busy()} onClick={(event) => (event.stopPropagation(), act(props.part.inputs ?? {}))}>
      <Show when={busy()} fallback={props.part.label}><Spinner /></Show>
    </button>
  )
}

function Flip(props) {
  const [busy, act] = acts(props)
  return (
    <label class="ui-choice switch" onClick={(event) => event.stopPropagation()}>
      <span>{props.part.label}</span>
      <input type="checkbox" role="switch" checked={!!props.part.on} disabled={busy()} onChange={(event) => act({ ...(props.part.inputs ?? {}), on: event.currentTarget.checked })} />
    </label>
  )
}

/// One part of a layout. A part the UI library doesn't have draws nothing.
function Part(props) {
  const part = () => props.part
  const held = () => <For each={Array.isArray(part().parts) ? part().parts : []}>{(inside) => <Part part={inside} onCall={props.onCall} />}</For>
  return (
    <Which>
      <Match when={part().part === "column" || part().part === "row"}>
        <div class={[part().part, room(part())]} data-tone={part().tone} style={{ ...sized(part()), gap: points(part().gap), "align-items": part().align === "center" ? "center" : undefined }}>{held()}</div>
      </Match>
      <Match when={part().part === "text"}>
        <span class={["text", part().size, part().weight, room(part()), { clamp: !!number(part().lines) }]} data-tone={part().tone} style={{ ...sized(part()), "-webkit-line-clamp": number(part().lines) }}>{part().text ?? ""}</span>
      </Match>
      <Match when={part().part === "time"}>
        <span class={["text", part().size, room(part())]} data-tone={part().tone}>{time(part().at, part().style)}</span>
      </Match>
      <Match when={part().part === "stat"}>
        <div class={["stat", room(part())]} data-tone={part().tone}>
          <div><strong>{figure(part().value)}</strong><Show when={part().unit}><span>{part().unit}</span></Show></div>
          <Show when={part().label}><small>{part().label}</small></Show>
        </div>
      </Match>
      <Match when={part().part === "gauge"}><Gauge part={part()} /></Match>
      <Match when={part().part === "progress"}>
        <div class={["progress", room(part())]} data-tone={part().tone} style={sized(part())}>
          <Show when={part().label}><small>{part().label}</small></Show>
          <div role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(share(part().value) * 100)} aria-label={part().label}><i style={{ width: `${share(part().value) * 100}%` }} /></div>
        </div>
      </Match>
      <Match when={part().part === "spark"}><Spark part={part()} /></Match>
      <Match when={part().part === "chart"}><Chart part={part()} /></Match>
      <Match when={part().part === "item"}>
        <div class={["item", room(part())]} data-tone={part().tone}>
          <div><span>{part().title ?? ""}</span><Show when={part().subtitle}><small>{part().subtitle}</small></Show></div>
          <Show when={part().at || part().trailing}><span class="end">{part().at ? time(part().at) : part().trailing}</span></Show>
        </div>
      </Match>
      <Match when={part().part === "image"}>
        <img alt="" src={/^data:image\//.test(part().url ?? "") ? part().url : undefined} class={{ round: part().round }} style={{ width: points(part().width) ?? "100%", height: points(part().height) }} />
      </Match>
      <Match when={part().part === "icon"}>
        <svg class="icon" data-tone={part().tone} viewBox="0 0 24 24" width={number(part().width, 16)} height={number(part().width, 16)} aria-hidden="true">
          <Show when={part().path}><path d={String(part().path)} /></Show>
          <Show when={part().badge}><path class="badge" d={String(part().badge)} transform={part().path ? "translate(13 13) scale(.458)" : undefined} /></Show>
        </svg>
      </Match>
      <Match when={part().part === "divider"}><hr /></Match>
      <Match when={part().part === "button"}><Press part={part()} onCall={props.onCall} /></Match>
      <Match when={part().part === "switch"}><Flip part={part()} onCall={props.onCall} /></Match>
    </Which>
  )
}

/// `<Widget name="Weather" size="2x2" layout={layout()} onCall={(tool, inputs) => call(tool, inputs)} onOpen={open} />`:
/// a plugin's widget, as a card: the `layout` its tool answered, drawn from the parts a layout is made of (the
/// skill `building-plugins` lists them), under its `name`, at a `size` of `2x2`, `4x2` or `4x4`. A button or a
/// switch in it calls `onCall` with its tool and inputs, and pressing the card anywhere else calls `onOpen`. With a
/// `problem` it says the widget stopped working, and why. With no layout yet it's a grey card.
export function Widget(props) {
  return (
    <div class={["ui-widget", { opens: !!props.onOpen }]} data-size={props.size} onClick={() => props.onOpen?.()}>
      <Show when={props.problem || props.layout?.title !== false}>
        <div class="name">{props.name}</div>
      </Show>
      <Show when={!props.problem} fallback={<div class="text sm" data-tone="warn">This widget stopped working. {props.problem}</div>}>
        <Show when={props.layout}>
          <Part part={props.layout} onCall={props.onCall} />
        </Show>
      </Show>
    </div>
  )
}
