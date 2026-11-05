import { cn } from "@/lib/utils"

/// A small line of how something changed, `values` (percents, oldest first) drawn filling the room it has, in the
/// text's colour. With none, a grey bar in its place.
function Sparkline({ values, className }: { values: number[]; className?: string }) {
  if (!values.length) return <div data-slot="sparkline" className={cn("h-1.5 rounded-full bg-muted", className)} />
  // At least 24 places wide, so a short history sits at the right and grows leftward.
  const n = Math.max(values.length, 24), w = 100, h = 28
  const pts = values.map((v, i) => [((n - values.length + i) / (n - 1)) * w, h - 1 - (v / 100) * (h - 2)])
  const d = pts.map((p, i) => `${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`).join(" ")
  return (
    <svg data-slot="sparkline" viewBox={`0 0 ${w} ${h}`} preserveAspectRatio="none" className={cn("block h-7 w-full", className)}>
      <path d={`${d} L${w} ${h} L${pts[0][0].toFixed(1)} ${h} Z`} fill="currentColor" opacity=".15" />
      <path d={d} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
    </svg>
  )
}

export { Sparkline }
