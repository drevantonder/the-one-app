// How what someone sees before they're in moves, with Motion's small animate: Welcome's, and, built alone into
// dist/auth.json (src/screen/ui/build.js), auth's pages'. The logo never moves; onboarding.css has the browser's half.
import { animate } from "motion/mini"
import { moves, still } from "../screen/ui/src/lib/moves"

/// The screen's parts arrive, each slightly behind the last, as what was there finishes fading (onboarding.css). A
/// problem only fades in (`appear`), and what's hidden has no turn.
export function rise(stack: Element) {
  if (still()) return
  const parts = [...stack.children].filter((el) => !el.matches("[data-slot=welcome-logo],[data-slot=welcome-problem]") && el.getClientRects().length)
  parts.forEach((el, i) => animate(el, { opacity: [0, 1], translate: ["0 10px", "0 0"] }, { ...moves.settle, delay: moves.quick.duration + i * 0.04 }))
}

export function appear(el: Element) {
  if (!still()) animate(el, { opacity: [0, 1] }, moves.quick)
}

/// Goes from one screen to another: `update` draws the new one, a moment later, the old one fading over it.
export function transition(update: () => void) {
  if (still() || !document.startViewTransition) return update()
  // The browser rejects this when it drops the move (another began, the tab is hidden); `update` still runs.
  document.startViewTransition(update).ready.catch(() => {})
}
