import { type Layout, type Scenes, styles, View, Widget } from "ui"

// Each in a box a Home Screen cell's size: 2x2, 4x2 and 4x4.
const cells = { "2x2": [170, 170], "4x2": [352, 170], "4x4": [352, 370] } as const
const look = styles({ cell: (width: number, height: number) => ({ width, height }) })
const cell = (size: keyof typeof cells, name: string, layout?: Layout, problem?: string) => () => (
  <View style={look.cell(cells[size][0], cells[size][1])}>
    <Widget name={name} size={size} layout={layout} problem={problem} onCall={async () => {}} onOpen={() => {}} />
  </View>
)

// The scenes' clock is 1 January 2026, 00:00 UTC.
const now = Date.UTC(2026, 0, 1)
const ago = (minutes: number) => now - minutes * 60_000

/** A sun, in a box of 24: a disc, and eight dots around it. */
const sun = `M7.5 12a4.5 4.5 0 1 1 9 0a4.5 4.5 0 1 1-9 0Z${Array.from({ length: 8 }, (_, at) => {
  const [x, y] = [12 + 9.5 * Math.cos((at * Math.PI) / 4), 12 + 9.5 * Math.sin((at * Math.PI) / 4)]
  return `M${(x - 1.5).toFixed(2)} ${y.toFixed(2)}a1.5 1.5 0 1 1 3 0a1.5 1.5 0 1 1-3 0Z`
}).join("")}`
const envelope = "M3 5h18a1 1 0 0 1 1 1v12a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1zm1 2.2V17h16V7.2l-8 5.3z"
const dot = "M2 12a10 10 0 1 1 20 0a10 10 0 1 1-20 0Z"
const face =
  "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCA0MCA0MCI+PHJlY3Qgd2lkdGg9IjQwIiBoZWlnaHQ9IjQwIiBmaWxsPSIjZjRjN2ExIi8+PGNpcmNsZSBjeD0iMjAiIGN5PSIxNiIgcj0iNyIgZmlsbD0iIzhhNWEzYiIvPjxwYXRoIGQ9Ik02IDQwYTE0IDEzIDAgMCAxIDI4IDB6IiBmaWxsPSIjM2I2ZmI2Ii8+PC9zdmc+"

const narrowOrWide: Layout = {
  part: "row",
  gap: 12,
  parts: [
    { part: "column", grow: true, parts: [{ part: "text", text: "Design review", weight: "medium", lines: 2 }, { part: "text", text: "10:30 – 11:15 · Room 4", size: "sm", tone: "muted" }] },
    { part: "column", from: "md", width: 120, parts: [{ part: "text", text: "Next", size: "xs", tone: "muted" }, { part: "text", text: "Lunch with Sam", size: "sm", lines: 1 }, { part: "text", text: "Gym", size: "sm", lines: 1 }] },
  ],
}

export default {
  "Widgets/Weather, 2x2": cell("2x2", "Weather", {
    part: "column",
    title: false,
    parts: [
      { part: "row", parts: [{ part: "text", text: "Melbourne", weight: "bold", grow: true }, { part: "icon", path: sun, width: 20, tone: "warn" }] },
      { part: "text", text: "21°", size: "xl" },
      { part: "column", grow: true, parts: [] },
      { part: "text", text: "Sunny", size: "sm", weight: "medium" },
      { part: "text", text: "H 24° L 14°", size: "xs", tone: "muted" },
    ],
  }),
  "Widgets/A stat with a spark, 2x2": cell("2x2", "Server Load", {
    part: "column",
    parts: [
      { part: "stat", value: 42, unit: "%", label: "CPU on mac-mini" },
      { part: "column", grow: true, parts: [] },
      { part: "spark", values: [12, 18, 15, 30, 26, 41, 35, 52, 44, 42], max: 100, height: 32, label: "The last hour" },
    ],
  }),
  "Widgets/A gauge, 2x2": cell("2x2", "Backups", {
    part: "column",
    gap: 10,
    parts: [
      { part: "gauge", value: 0.72, label: "72%", width: 56, tone: "warn" },
      { part: "text", text: "1.4 TB of 2 TB", weight: "medium" },
      { part: "time", at: ago(130), size: "sm", tone: "muted" },
    ],
  }),
  "Widgets/A chart with labels, 4x2": cell("4x2", "Steps", {
    part: "row",
    gap: 16,
    parts: [
      { part: "column", width: 96, parts: [{ part: "stat", value: 8412, label: "Today" }, { part: "text", text: "Goal 10,000", size: "xs", tone: "muted" }] },
      { part: "chart", grow: true, height: 96, values: [6200, 9100, 4300, 10400, 7600, 12100, 8412], labels: ["M", "T", "W", "T", "F", "S", "S"], label: "Steps this week" },
    ],
  }),
  "Widgets/A list of items, 4x4": cell("4x4", "Inbox", {
    part: "column",
    parts: [
      { part: "row", parts: [{ part: "icon", path: envelope, badge: dot, width: 22, tone: "bad" }, { part: "text", text: "3 unread", weight: "bold", size: "lg", grow: true }] },
      { part: "divider" },
      { part: "item", title: "Sam Carter", subtitle: "Lunch on Friday? I found a place near the office", at: ago(12) },
      { part: "item", title: "GitHub", subtitle: "Your pull request was merged", at: ago(95) },
      { part: "item", title: "Qantas", subtitle: "Your boarding pass for QF 401", trailing: "Fri" },
      { part: "item", title: "Bank of Melbourne", subtitle: "Statement ready", trailing: "Thu", dim: true },
      { part: "item", title: "Alex Nguyen", subtitle: "Re: plans for the weekend", trailing: "Wed", dim: true },
    ],
  }),
  "Widgets/A button and a switch, 4x2": cell("4x2", "Stay Awake", {
    part: "column",
    gap: 8,
    parts: [
      { part: "row", parts: [{ part: "image", url: face, width: 36, height: 36, round: true }, { part: "column", grow: true, parts: [{ part: "text", text: "mac-mini", size: "lg" }, { part: "text", text: "Awake for 2 hours, lid shut", size: "sm", tone: "muted" }] }] },
      { part: "row", parts: [{ part: "switch", label: "Keep Awake", on: true, tool: "set", inputs: { mac: "mac-mini" } }, { part: "button", label: "Sleep Now", tool: "sleep", inputs: { mac: "mac-mini" } }] },
    ],
  }),
  "Widgets/Everything else, 4x4": cell("4x4", "Devices", {
    part: "column",
    gap: 8,
    parts: [
      { part: "row", parts: [{ part: "text", text: "Disks", weight: "bold", grow: true }, { part: "time", at: ago(0), style: "clock", size: "sm", tone: "muted" }] },
      { part: "progress", value: 0.34, label: "mac-mini · 340 GB of 1 TB" },
      { part: "progress", value: 0.86, label: "nas · 3.4 TB of 4 TB", tone: "warn" },
      { part: "progress", value: 0.98, label: "laptop · almost full", tone: "bad" },
      { part: "divider" },
      { part: "text", text: "Network, the last day", size: "sm", weight: "medium" },
      { part: "chart", kind: "line", height: 60, values: [3, 5, 4, 8, 12, 9, 14, 11, 7, 9, 13, 10], label: "Network" },
      { part: "row", dim: true, gap: 4, parts: [{ part: "text", text: "Off since", size: "xs" }, { part: "time", at: ago(60 * 26), style: "date", size: "xs" }, { part: "spark", values: [], width: 60, height: 12 }] },
      { part: "text", text: "Pixel 8 last checked in from Sydney, and has a backup waiting to go up once it's on Wi-Fi again tonight.", size: "sm", tone: "muted", lines: 2 },
    ],
  }),
  "Widgets/Dropped where it's narrow, 2x2": cell("2x2", "Calendar", narrowOrWide),
  "Widgets/Kept where it's wide, 4x2": cell("4x2", "Calendar", narrowOrWide),
  "Widgets/A problem, 2x2": cell("2x2", "Weather", undefined, "Couldn't reach the weather service."),
  "Widgets/No layout yet, 2x2": cell("2x2", "Weather"),
} satisfies Scenes
