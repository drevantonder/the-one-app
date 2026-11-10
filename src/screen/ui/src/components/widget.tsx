// A widget drawn from its tree (`GET /api/widgets`): a card under its title, the same on the pages (src/screen/home)
// and in a chat's view (src/mcp_view.html). The Apple, Android and Windows widgets draw the same trees natively. A
// tree is the app's, so nothing in it is ever markup: what's drawn is text, a number (`num`), or one of the names here.
// `host` is what buttons, toggles and links act through: `run(app, tool, input, widget)` → the error's text for users if
// it failed, and `follow(link, widget)` for a web page's address or `{ screen, params }`.
// A size is what fits in it, the same on every client: small is one fact, medium a few rows, large those rows with a
// detail each and a chart, xl the list beside the chart. A tree that's one muted text or an empty list has nothing to
// show yet, and a page gives it the smallest card.
import * as React from "react"
import { useLayoutEffect, useRef, useState } from "react"
import { Button } from "@/components/ui/button"
import { Spinner } from "@/components/ui/spinner"
import { Switch } from "@/components/ui/switch"
import { Icon } from "@/components/icon"
import { cn } from "@/lib/utils"

/// A widget as an app gives it, and a node of its tree: the app's JSON, read only as the names here say.
type Widget = { app: string; widget: string; title?: string; error?: string; root?: Tree; [key: string]: any }
type Tree = { [key: string]: any }
/// What a widget's buttons, toggles and links act through.
type WidgetHost = {
  run: (app: string, tool: string, input: object, w: Widget) => Promise<string | void | undefined>
  follow: (link: string | { screen: string; params?: object }, w: Widget) => unknown
}
type Parts = { n: Tree; w: Widget; host: WidgetHost }
const one = () => (globalThis as unknown as { one: { plain: (e: unknown) => { title: string } } }).one

// Colour only where it means something: fine is plain text, warn and bad need you.
const TONES = { muted: "var(--muted-foreground)", accent: "var(--foreground)", good: "var(--foreground)", warn: "var(--warning)", bad: "var(--destructive)" }
const SIZES = { xl: "text-[26px] font-semibold", lg: "text-[17px] font-semibold", sm: "text-[13px]", xs: "text-xs" }
const WEIGHTS = { medium: "font-medium", bold: "font-bold" }
// A number from a tree, or `or` where it isn't one.
const num = (v: any, or?: any): any => (v === null || v === "" || !Number.isFinite(Number(v)) ? or : Number(v))
const fmt = (v: unknown) => (typeof v === "number" ? v.toLocaleString(undefined, { maximumFractionDigits: 2 }) : String(v ?? "–"))
const tone = (n: Tree): string | null => (Object.hasOwn(TONES, n.tone) ? TONES[n.tone as keyof typeof TONES] : null)

function time(n: Tree) {
  const d = new Date(n.at || 0), s = (Date.now() - d.getTime()) / 1000
  if (n.style === "date") return d.toLocaleDateString(undefined, { day: "numeric", month: "short" })
  if (n.style === "clock") return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  if (n.style === "timer") { const a = Math.abs(Math.round(s)); return `${Math.floor(a / 60)}:${String(a % 60).padStart(2, "0")}` }
  if (n.style === "relative") return new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" }).format(...((Math.abs(s) < 3600 ? [-Math.round(s / 60), "minute"] : Math.abs(s) < 86400 ? [-Math.round(s / 3600), "hour"] : [-Math.round(s / 86400), "day"]) as [number, Intl.RelativeTimeFormatUnit]))
  if (s < 60) return "now"
  if (s < 3600) return `${Math.floor(s / 60)} min`
  if (s < 86400) return `${Math.floor(s / 3600)} h`
  return d.toLocaleDateString(undefined, s < 7 * 86400 ? { weekday: "short" } : { day: "numeric", month: "short" })
}

/// When a widget that's old (`w.old`, in ms) was last read, as the home-screen widgets say it: "Updated 2 h ago".
function updated(at: number) {
  const said = time({ at })
  return Date.now() - at < 86_400_000 ? `Updated ${said} ago` : `Updated ${said}`
}

/// A widget, as a card: its tree under its title (unless the tree labels itself), dimmed with when it's from if it's
/// old (`w.old`); a broken one says so, and why. Where a window draws screens at its smaller text (a zoom on the page),
/// a widget keeps its own size.
function WidgetCard({ w, host, tree = w.root, className, style, ...props }: { w: Widget; host: WidgetHost; tree?: Tree } & React.ComponentProps<"div">) {
  const why = w.error ? one().plain(w.error).title : ""
  return (
    <div
      data-slot="widget"
      className={cn("flex min-w-0 flex-col gap-1.5 overflow-hidden rounded-3xl bg-muted p-4 text-[15px] leading-snug text-foreground", tree?.link && "cursor-pointer active:opacity-80", className)}
      style={{ zoom: "var(--unzoom, 1)", ...style }}
      onClick={tree?.link && !w.error ? (e) => !e.defaultPrevented && host.follow(tree.link, w) : undefined}
      {...props}
    >
      {(tree?.header !== false || w.error) && <div className="shrink-0 truncate text-[13px] font-medium text-muted-foreground">{w.title || w.widget}</div>}
      {w.error ? (
        <div className="flex min-h-0 flex-col gap-1">
          <div className="flex items-center gap-1.5 font-medium text-warning"><Icon name="warning-fill" className="size-4 shrink-0" />This widget stopped working.</div>
          {why && why !== "That didn’t work." && <div className="line-clamp-4 select-text break-words text-[13px] text-muted-foreground">{why}</div>}
        </div>
      ) : (
        <div className={cn("flex min-h-0 flex-1 flex-col [&>[data-w=col]]:flex-1 [&>.contents>[data-w=col]]:flex-1", w.old && "opacity-50")}>
          <Node n={tree} w={w} host={host} />
        </div>
      )}
      {w.old && !w.error && <div className="shrink-0 truncate text-xs text-muted-foreground">{updated(w.old)}</div>}
    </div>
  )
}

/// A node of a widget's tree.
function Node({ n, w, host }: { n?: Tree; w: Widget; host: WidgetHost }): React.ReactNode {
  if (!n || typeof n !== "object") return null
  const color = tone(n)
  const colored = color ? { color, "--tone": color } : undefined
  const kids = (n.c || []).map((c: Tree, i: number) => <Node key={i} n={c} w={w} host={host} />)
  let el: React.ReactNode
  switch (n.t) {
    case "col": case "row": {
      const style: React.CSSProperties = {}
      if (num(n.gap) != null) style.gap = `${num(n.gap)}px`
      if (num(n.w)) Object.assign(style, { width: `${num(n.w)}px`, flex: "none" })
      if (n.grow) style.flex = "1"
      if (n.align === "center") style.alignItems = "center"
      if (n.dim) style.opacity = 0.5
      el = (
        <div
          data-w={n.t}
          className={cn("flex min-w-0", n.t === "col" ? "flex-col gap-1" : "items-center gap-1.5 [&>*]:min-w-0 [&>[data-w=text]]:truncate")}
          style={style}
        >
          {kids}
        </div>
      )
      break
    }
    case "text": {
      const style: React.CSSProperties = { ...colored }
      if (num(n.lines)) Object.assign(style, { display: "-webkit-box", WebkitBoxOrient: "vertical", WebkitLineClamp: Math.round(num(n.lines)), overflow: "hidden" })
      el = <span data-w="text" className={cn(SIZES[n.size as keyof typeof SIZES], WEIGHTS[n.weight as keyof typeof WEIGHTS])} style={style}>{n.text ?? ""}</span>
      break
    }
    case "time":
      el = <span className={cn("shrink-0", SIZES[n.size as keyof typeof SIZES])} style={colored}>{time(n)}</span>
      break
    case "stat":
      el = (
        <div className="flex min-w-0 flex-col" style={colored}>
          <div className="flex min-w-0 items-baseline gap-[3px]">
            <Fit className="text-[34px] font-semibold tabular-nums">{fmt(n.value)}</Fit>
            {n.unit && <span className="shrink-0 text-[17px] font-medium text-muted-foreground">{n.unit}</span>}
          </div>
          {n.label && <small className="truncate text-[13px] text-muted-foreground">{n.label}</small>}
        </div>
      )
      break
    case "gauge": {
      // Drawn at its own size, as the other widgets draw it: its ring no thicker than a sixth of it.
      const f = Math.max(0, Math.min(1, num(n.value, 0) / (num(n.max) || 1))), size = num(n.w) || 46
      const line = Math.min(5, size / 6), r = (size - line) / 2, c = 2 * Math.PI * r, mid = size / 2
      el = (
        <svg viewBox={`0 0 ${size} ${size}`} width={size} height={size} className="shrink-0">
          <circle cx={mid} cy={mid} r={r} fill="none" stroke="var(--foreground)" strokeOpacity=".15" strokeWidth={line} />
          <circle cx={mid} cy={mid} r={r} fill="none" stroke={color || "var(--foreground)"} strokeWidth={line} strokeLinecap="round" strokeDasharray={`${c * f} ${c}`} transform={`rotate(-90 ${mid} ${mid})`} />
          {n.label !== "" && <text x={mid} y={mid + 3.5} textAnchor="middle" fontSize={Math.min(10, size / 4.6)} fontWeight="600" fill="currentColor">{n.label ?? Math.round(f * 100) + "%"}</text>}
        </svg>
      )
      break
    }
    case "progress": {
      const f = Math.max(0, Math.min(1, num(n.value, 0) / (num(n.max) || 1)))
      el = (
        <div className="flex min-w-0 flex-col gap-1" style={num(n.w) ? { width: num(n.w) } : undefined}>
          {n.label && <span className="text-[13px] text-muted-foreground">{n.label}</span>}
          <div role="progressbar" aria-label={n.label || undefined} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(f * 100)} className="h-1.5 overflow-hidden rounded-full bg-foreground/15">
            <i className="block h-full" style={{ width: `${f * 100}%`, background: color || "var(--foreground)" }} />
          </div>
        </div>
      )
      break
    }
    case "spark": case "chart": el = <Chart n={n} color={color} />; break
    case "list":
      el = (
        <div className="flex min-w-0 flex-col">
          {!(n.c || []).length && <div className="text-muted-foreground">{n.empty || "Nothing here"}</div>}
          {kids}
        </div>
      )
      break
    case "item":
      el = (
        <div className="flex items-center gap-2 border-t border-foreground/10 py-[5px] first:border-t-0" style={colored}>
          {n.icon && <span className="size-2 shrink-0 rounded-full" style={{ background: color || "var(--muted-foreground)" }} />}
          <div className="min-w-0">
            <div className="truncate">{n.title || ""}</div>
            {n.subtitle && <div className="truncate text-[13px] text-muted-foreground">{n.subtitle}</div>}
          </div>
          {(n.at || n.trailing) && <span className="ml-auto shrink-0 tabular-nums text-muted-foreground">{n.at ? time({ at: n.at }) : n.trailing}</span>}
        </div>
      )
      break
    case "image": {
      const src = /^(https:|data:image\/)/.test(n.url || "") ? n.url : undefined
      // Without a height, a shape of its own (square beside a width, else 16:9), so the card doesn't move when it loads.
      el = (
        <img
          src={src}
          alt=""
          className="shrink-0 bg-foreground/10 object-cover"
          style={{ width: num(n.w) ? num(n.w) : "100%", height: num(n.h) ? num(n.h) : "auto", ...(num(n.h) ? {} : { aspectRatio: num(n.w) ? "1" : "16 / 9" }), borderRadius: n.round ? "50%" : 8 }}
        />
      )
      break
    }
    case "dot":
      el = <span className="size-2 shrink-0 rounded-full" style={{ background: color || "var(--muted-foreground)" }} />
      break
    case "icon": el = <Glyph n={n} color={color} />; break
    case "link":
      el = <a className="cursor-pointer underline underline-offset-4">{n.text || ""}</a>
      break
    case "divider":
      el = <hr className="my-1 w-full border-0 border-t border-foreground/15" />
      break
    case "button": el = <WidgetButton n={n} w={w} host={host} color={color} />; break
    case "toggle": el = <WidgetToggle n={n} w={w} host={host} />; break
    default: return null
  }
  // A node with a link of its own opens it, never the card's.
  if (n.link && n.t !== "button" && n.t !== "toggle") {
    return <Linked n={n} w={w} host={host}>{el}</Linked>
  }
  return el
}

function Linked({ n, w, host, children }: Parts & { children: React.ReactNode }) {
  return (
    <span className="contents [&>*]:cursor-pointer [&>*]:active:opacity-60" onClickCapture={(e) => { e.preventDefault(); e.stopPropagation(); host.follow(n.link, w) }}>
      {children}
    </span>
  )
}

function Glyph({ n, color }: { n: Tree; color: string | null }) {
  const size = num(n.w) || 16
  // A named icon is Phosphor's, drawn where the bridge is.
  if (n.name) {
    return (
      <span className="flex shrink-0" style={{ width: size, height: size, color: color || "currentColor" }}>
        <Icon name={String(n.name)} className="size-full" />
      </span>
    )
  }
  return (
    <svg viewBox="0 0 24 24" width={size} height={size} className="shrink-0 overflow-visible" style={{ color: color || "currentColor" }}>
      {n.path && <path d={String(n.path)} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />}
      {n.badge && <path d={String(n.badge)} fill="currentColor" transform={n.path || n.name ? "translate(15.5 15.5) scale(.458)" : undefined} />}
    </svg>
  )
}

/// A chart or sparkline at the size the tree gave it: its height in pixels (`h`), and the width it has (or its `w`), so
/// its labels and strokes are drawn as they are and never grow with the card.
function Chart({ n, color }: { n: Tree; color: string | null }) {
  const ref = useRef<HTMLDivElement>(null)
  const [room, setRoom] = useState(0)
  // The width its box has, in its own units (not the page's, which a zoom changes).
  useLayoutEffect(() => {
    const box = ref.current!
    setRoom(box.clientWidth)
    const seen = new ResizeObserver(() => setRoom(box.clientWidth))
    seen.observe(box)
    return () => seen.disconnect()
  }, [])
  const spark = n.t === "spark", labels = Array.isArray(n.labels) ? n.labels : []
  const v = (Array.isArray(n.values) ? n.values : []).map((y: unknown): number => num(y, 0)), max = num(n.max) || Math.max(...v, 1)
  const h = num(n.h) || (spark ? 28 : 70), fixed = spark && num(n.w) ? num(n.w) : 0
  const wd = fixed || room, step = wd / Math.max(v.length, 1), fill = color || "var(--foreground)"
  const total = h + (n.labels ? 14 : 0)
  let body
  if (spark && !v.length) {
    body = <line x1="2" y1={h / 2} x2={wd - 2} y2={h / 2} stroke="var(--muted-foreground)" strokeOpacity=".5" strokeWidth="1.5" strokeDasharray="2 3" />
  } else if (spark) {
    const pts = v.map((y, i) => [v.length > 1 ? (i / (v.length - 1)) * wd : wd / 2, h - 1 - (Math.min(y, max) / max) * (h - 2)])
    const line = pts.map((p) => p.join(",")).join(" ")
    body = (
      <>
        <polygon points={`${line} ${wd},${h} 0,${h}`} fill={fill} opacity=".2" />
        <polyline points={line} fill="none" stroke={fill} strokeWidth="1.4" strokeLinejoin="round" />
      </>
    )
  } else if (n.kind === "line") {
    body = <polyline points={v.map((y, i) => `${(i + 0.5) * step},${h - (y / max) * (h - 4) - 2}`).join(" ")} fill="none" stroke={fill} strokeWidth="2" strokeLinejoin="round" strokeLinecap="round" />
  } else {
    body = v.map((y, i) => <rect key={i} x={i * step + step * 0.15} y={h - (y / max) * h} width={step * 0.7} height={(y / max) * h} rx="3" fill={fill} />)
  }
  return (
    <div ref={ref} className="min-w-0 shrink-0" style={{ width: fixed || "100%", height: total }}>
      {wd > 0 && (
        <svg viewBox={`0 0 ${wd} ${total}`} width={wd} height={total} className="block">
          {body}
          {labels.map((l, i) => <text key={i} x={(i + 0.5) * step} y={h + 12} textAnchor="middle" fontSize="10" fill="var(--muted-foreground)">{String(l ?? "")}</text>)}
        </svg>
      )}
    </div>
  )
}

/// Text on one line that gives up size before it gives up words, down to 0.6 of it, then is cut short.
function Fit({ children, className }: { children: React.ReactNode; className?: string }) {
  const box = useRef<HTMLSpanElement>(null), text = useRef<HTMLSpanElement>(null)
  useLayoutEffect(() => {
    const outer = box.current!, inner = text.current!
    const fit = () => {
      inner.style.fontSize = ""
      const need = inner.scrollWidth, room = outer.clientWidth
      if (need > room && room > 0) inner.style.fontSize = `${Math.max(0.6, room / need)}em`
    }
    fit()
    const seen = new ResizeObserver(fit)
    seen.observe(outer)
    return () => seen.disconnect()
  }, [children])
  return <span ref={box} className={cn("block min-w-0 truncate whitespace-nowrap", className)}><span ref={text} className="inline-block">{children}</span></span>
}

// A button or toggle runs the app's tool as the user, pressed with a spinner meanwhile; the host draws the
// widget again. A toggle that failed goes back to how it was.
function useAct(w: Widget, host: WidgetHost) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const act = async (tool: string, input: object) => {
    setBusy(true)
    const failed = await host.run(w.app, tool, input, w)
    setBusy(false)
    setError(failed || "")
    return failed
  }
  return { busy, error, act }
}

function WidgetButton({ n, w, host, color }: Parts & { color: string | null }) {
  const { busy, error, act } = useAct(w, host)
  return (
    <Button
      variant="secondary"
      size="xs"
      className="relative rounded-full bg-foreground/10 hover:bg-foreground/15"
      style={color ? { color } : undefined}
      disabled={busy}
      title={error}
      onClick={(e) => { e.preventDefault(); e.stopPropagation(); act(n.tool, n.input || {}) }}
    >
      <span className={busy ? "invisible" : undefined}>{n.label || n.text || ""}</span>
      {busy && <Spinner className="absolute size-3" />}
    </Button>
  )
}

function WidgetToggle({ n, w, host }: Parts) {
  const { busy, error, act } = useAct(w, host)
  const [on, setOn] = useState(!!n.on)
  const [seen, setSeen] = useState(!!n.on)
  // The node says how it stands, once it does.
  if (seen !== !!n.on) { setSeen(!!n.on); setOn(!!n.on) }
  return (
    <label className="flex flex-1 items-center justify-between gap-2" title={error} onClick={(e) => e.stopPropagation()}>
      <span>{n.label || ""}</span>
      {busy && <Spinner className="size-3" />}
      <Switch
        size="sm"
        checked={on}
        disabled={busy}
        onClick={(e) => e.stopPropagation()}
        onCheckedChange={async (next) => {
          setOn(next)
          if (await act(n.tool, { ...(n.input || {}), on: next })) setOn(!next)
        }}
      />
    </label>
  )
}

export { WidgetCard, type Widget, type WidgetHost }
