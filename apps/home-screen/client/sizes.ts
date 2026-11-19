import { defineVars } from "ui"

/** The Home Screen's own lengths, as an iPhone's Home Screen measures them: the gap under an app's icon to its name,
 * between the small icons in a category's fourth place, and beside the owner's name in its pill; a menu's row; an
 * app found in the launcher's search, and the owner's name at its widest; and the launcher's search's inset. */
export const sizes = defineVars({
  iconGap: 7,
  folderGap: 5,
  ownerGap: 5,
  menuRow: 44,
  found: 76,
  foundHeight: 88,
  ownerName: 144,
  searchInline: 18,
})
