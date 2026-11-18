// The Notifications inbox, as the Lynx spike drew it (spike/lynx at d17898ec), written as a strict app would be (box,
// text, image, input and the protocol's props), with today's v1 prop and event names.
import { createSignal, createMemo, For } from "solid-js"
import { h, text, mount } from "./solid-host.js"

const APPS = ["vault", "backups", "telemetry", "notifications", "soft-delete"]
const TITLES = ["Backup finished", "New sign-in", "Request waiting", "Weekly report", "Deleted for good", "Item shared with you"]
const FROM = ["Vault", "Backups", "Telemetry", "Notifications", "Recently Deleted"]
const TEXTS = [
  "Your network was backed up to the family bucket.",
  "A Pixel 9 signed in as you in Brisbane.",
  "Andre asks for the Google client.",
  "3 errors, 12 slow calls, AI cost $0.42 this week.",
  "18 items were deleted for good after 30 days.",
  "Anna shared Wi-Fi with the family.",
]

export function start(onUpdate) {
  const [query, setQuery] = createSignal("")
  const [read, setRead] = createSignal(new Set())
  const all = Array.from({ length: 30 }, (_, i) => ({
    id: i,
    app: APPS[i % APPS.length],
    title: TITLES[i % TITLES.length],
    from: FROM[i % FROM.length],
    text: TEXTS[(i * 5) % TEXTS.length],
    ago: i < 5 ? `${i * 7 + 2}m` : `${i - 3}h`,
  }))
  const shown = createMemo(() => {
    const q = query().toLowerCase()
    return q ? all.filter((n) => (n.title + n.text + n.from).toLowerCase().includes(q)) : all
  })

  const Row = (n) =>
    h(
      "box",
      {
        role: "listitem",
        "aria-label": () => `${read().has(n.id) ? "" : "Unread. "}${n.title}, ${n.from}, ${n.ago} ago`,
        onClick: () => setRead((s) => new Set(s).add(n.id)),
        style: { flexDirection: "row", alignItems: "center", gap: 12, paddingBlock: 12, borderBottomWidth: 1, borderColor: "border" },
      },
      h("image", { src: `icons/${n.app}.png`, alt: n.from, style: { width: 40, height: 40, borderRadius: "md" } }),
      h(
        "box",
        { style: { flexGrow: 1, flexShrink: 1, gap: 2 } },
        h(
          "box",
          { style: { flexDirection: "row", justifyContent: "space-between", alignItems: "baseline", gap: 8 } },
          text({ style: () => ({ font: read().has(n.id) ? "body" : "label", color: "foreground", lineClamp: 1, flexShrink: 1 }) }, n.title),
          text({ style: { font: "caption-1", color: "muted-foreground", fontVariantNumeric: "tabular-nums" } }, n.ago),
        ),
        text({ style: { font: "footnote", color: "muted-foreground", lineClamp: 2 } }, `${n.from} · ${n.text}`),
      ),
    )

  return mount(
    () =>
      h(
        "box",
        { role: "main", style: { flexDirection: "column", padding: "gutter", gap: "gap", backgroundColor: "background", height: "100%" } },
        text({ role: "heading", "aria-level": 1, style: { font: "title", color: "foreground" } }, "Notifications"),
        h("input", {
          type: "search",
          placeholder: "Search",
          "aria-label": "Search notifications",
          enterkeyhint: "search",
          onInput: (value) => setQuery(value),
          style: { height: "control", paddingInline: 12, borderRadius: "md", backgroundColor: "muted", color: "foreground", font: "body" },
        }),
        h(
          "box",
          { role: "list", "aria-label": "Inbox", style: { flexDirection: "column", flexGrow: 1, overflow: "scroll" } },
          h(For, { get each() { return shown() }, children: (n) => Row(n) }),
          () => (shown().length ? null : text({ style: { font: "body", color: "muted-foreground" } }, "No notifications.")),
        ),
      ),
    onUpdate,
  )
}
