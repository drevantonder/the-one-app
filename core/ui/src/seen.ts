// When something was last seen, written once for the DOM and a native host, whose sandbox has Intl too.

let ago: Intl.RelativeTimeFormat | undefined
/// `seen(device.seen)`: when a device or a machine that's off was last on, as its grey line says it, in the largest
/// unit that's whole: "Seen 2 minutes ago". `at` is a time, or milliseconds since 1970.
export function seen(at: string | number | Date): string {
  ago ??= new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
  const minutes = Math.round((new Date(at).getTime() - Date.now()) / 60_000)
  if (minutes > -60) return `Seen ${ago.format(Math.min(minutes, -1), "minute")}`
  if (minutes > -48 * 60) return `Seen ${ago.format(Math.round(minutes / 60), "hour")}`
  return `Seen ${ago.format(Math.round(minutes / (24 * 60)), "day")}`
}
