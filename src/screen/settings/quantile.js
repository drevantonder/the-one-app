// The quantile of histograms added together, interpolated within its bucket as Prometheus does. `insights`'s lines add up
// the same way (insights/src/view.rs), which tests/contracts.rs holds them to. Each row's `buckets` are counts by OTEL's
// default duration boundaries; what's over the last has no bound, so it's drawn as a minute.
export const BOUNDS = [0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000, 60000]
/// The last bound a duration is counted under: a quantile past it is only known to be over it.
export const LAST_BOUND = BOUNDS.at(-2)

export function quantile(rows, q) {
  const counts = BOUNDS.map((_, i) => rows.reduce((n, r) => n + (r.buckets?.[i] || 0), 0))
  const total = counts.reduce((a, b) => a + b, 0)
  if (!total) return null
  let seen = 0, low = 0
  for (let i = 0; i < counts.length; i++) {
    if (counts[i] && seen + counts[i] >= q * total) return low + ((q * total - seen) / counts[i]) * (BOUNDS[i] - low)
    seen += counts[i]
    low = BOUNDS[i]
  }
  return low
}
