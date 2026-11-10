// The quantile of histograms added together, interpolated within its bucket as Prometheus does. `insights`'s lines add up
// the same way (src/insights/view.rs), which tests/contracts.rs holds them to.
export const BUCKETS = [["under_10", 10], ["under_100", 100], ["under_1000", 1000], ["under_10000", 10000], ["over", 60000]]

export function quantile(rows, q) {
  const counts = BUCKETS.map(([k]) => rows.reduce((n, r) => n + (r[k] || 0), 0))
  const total = counts.reduce((a, b) => a + b, 0)
  if (!total) return null
  let seen = 0, low = 0
  for (let i = 0; i < counts.length; i++) {
    if (counts[i] && seen + counts[i] >= q * total) return low + ((q * total - seen) / counts[i]) * (BUCKETS[i][1] - low)
    seen += counts[i]
    low = BUCKETS[i][1]
  }
  return low
}
