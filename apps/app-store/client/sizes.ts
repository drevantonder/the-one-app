import { defineVars } from "ui"

/** The App Store's own lengths: an app's icon and its tile as a Home Screen has them, its icon atop its page, and how
 * wide a category's shelf is at least. */
export const sizes = defineVars({
  icon: 60,
  tile: 68,
  largeIcon: 120,
  shelf: 320,
})
