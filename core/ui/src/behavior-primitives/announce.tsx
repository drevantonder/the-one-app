// What a screen reader reads out without its focus moving: a toast, something done in the background. From Vercel's
// UI SDK (Apache-2.0), packages/core/src/foundation/platform/announce.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7:
// a polite and an assertive live region, made once, the first time something's said, and each text set a moment after
// the region's emptied, so the same text twice is read twice. On our host a region is a node whose role is a live
// one (`status`, `alert`), over the page in a `Portal`, so it's read whatever's modal.
import { createRoot, createSignal } from "solid-js"
import { Portal, Text, View } from "#primitives"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

/** How urgently a screen reader reads an announcement: `polite` waits for a pause, `assertive` interrupts. */
export type AnnouncePriority = "polite" | "assertive"

// Read, never seen.
const look = styles({ unseen: { position: "absolute", width: tokens.length.hairline, height: tokens.length.hairline, overflow: "hidden", opacity: 0 } })

let said: Record<AnnouncePriority, (text: string) => void> | undefined
const pending: Partial<Record<AnnouncePriority, ReturnType<typeof setTimeout>>> = {}

function regions() {
  const [polite, setPolite] = createSignal("")
  const [assertive, setAssertive] = createSignal("")
  createRoot(() => (
    <Portal>
      <View role="status" style={look.unseen}>
        <Text>{polite()}</Text>
      </View>
      <View role="alert" style={look.unseen}>
        <Text>{assertive()}</Text>
      </View>
    </Portal>
  ))
  return { polite: setPolite, assertive: setAssertive }
}

/** `announce("Deleted")`: has screen readers read `text` once they're idle, or at once with `priority: "assertive"`.
 * The same text twice is read twice. */
export function announce(text: string, { priority = "polite" }: { priority?: AnnouncePriority } = {}) {
  if (!text) return
  said ??= regions()
  const set = said[priority]
  set("")
  // Screen readers only notice text that changes after the region exists, and the same text twice only after it was
  // cleared. A newer text replaces one still waiting at the same priority.
  clearTimeout(pending[priority])
  pending[priority] = setTimeout(() => set(text), 100)
}
