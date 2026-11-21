// Times and sizes as a person reads them, written once for every host, whose sandbox has Intl.

const relative: Partial<Record<"long" | "narrow", Intl.RelativeTimeFormat>> = {}

/// `date(device.added)`: a day as a person reads it, "Dec 28, 2025", its spaces no-break ones so it's never broken
/// across two lines, as a date isn't in a printed page. `at` is a time, or milliseconds since 1970.
export const date = (at: string | number | Date): string => new Date(at).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" }).replaceAll(" ", "\u00a0")

/// `since(message.at)`: how long ago a time was, in the largest unit that's whole, as Intl.RelativeTimeFormat says
/// it: "now", "5 minutes ago", "3 hours ago", "yesterday", "12 days ago", and from 30 days its date (`date`), "3 Oct 2026".
/// `since(at, "narrow")` is short, for a row's corner: "5m ago", "3h ago". `at` is a time, or milliseconds since 1970.
export function since(at: string | number | Date, style: "long" | "narrow" = "long"): string {
  const ago = (relative[style] ??= new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style }))
  const minutes = Math.round((new Date(at).getTime() - Date.now()) / 60_000)
  if (minutes > -1) return ago.format(0, "second")
  if (minutes > -60) return ago.format(minutes, "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  if (minutes > -30 * 24 * 60) return ago.format(Math.round(minutes / (24 * 60)), "day")
  return date(at)
}

/// `seen(device.seen)`: when a device or a machine that's off was last on, as its grey line says it: "Seen 2 minutes
/// ago", never "Seen now". `at` is a time, or milliseconds since 1970.
export const seen = (at: string | number | Date): string => `Seen ${since(Math.min(new Date(at).getTime(), Date.now() - 60_000))}`

const units = ["kilobyte", "megabyte", "gigabyte", "terabyte", "petabyte"]

/// `size(file.bytes)`: how big something is, in thousands as Apple's devices count, as Intl.NumberFormat says it:
/// "512 bytes", "1.2 MB", "340 GB".
export function size(bytes: number): string {
  if (bytes < 1000) return new Intl.NumberFormat(undefined, { style: "unit", unit: "byte", unitDisplay: "long" }).format(bytes)
  const power = Math.min(Math.floor(Math.log10(bytes) / 3), units.length)
  const count = bytes / 1000 ** power
  const digits = count < 100 ? 1 : 0
  return new Intl.NumberFormat(undefined, { style: "unit", unit: units[power - 1], maximumFractionDigits: digits }).format(count)
}
