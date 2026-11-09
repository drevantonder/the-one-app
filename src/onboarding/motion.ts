// How what someone sees before they're in moves, with Motion's small animate: Welcome's, and, built alone into
// dist/auth.json (src/screen/ui/build.js), auth's pages'. The logo never moves; onboarding.css has the browser's half.
import { animate } from "motion/mini"

const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches

/// The screen's parts arrive, each slightly behind the last, as what was there finishes fading (onboarding.css). A
/// problem only fades in (`appear`), and what's hidden has no turn.
export function rise(stack: Element) {
  if (still()) return
  const parts = [...stack.children].filter((el) => !el.matches("[data-slot=welcome-logo],[data-slot=welcome-problem]") && el.getClientRects().length)
  parts.forEach((el, i) => animate(el, { opacity: [0, 1], translate: ["0 10px", "0 0"] }, { duration: 0.3, delay: 0.1 + i * 0.04, ease: [0.22, 1, 0.36, 1] }))
}

export function appear(el: Element) {
  if (!still()) animate(el, { opacity: [0, 1] }, { duration: 0.2 })
}

/// Goes from one screen to another: `update` draws the new one, a moment later, the old one fading over it.
export function transition(update: () => void) {
  if (still() || !document.startViewTransition) update()
  else document.startViewTransition(update)
}
