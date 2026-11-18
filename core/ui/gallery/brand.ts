import { createTheme, defineVars, tokens } from "ui"

/** The gallery's own tokens: a brand colour, light and dark, and a length. */
export const brand = defineVars({
  accent: { default: "#d6336c", dark: "#ff6b9d" },
  tile: 56,
})

// And a colour of ours it names new values for, which only the scenes of "Styling" use: its tint, which is a part of its
// opacity, follows.
createTheme(tokens.colour, { magenta: { default: "#6f42c1", dark: "#b197fc" } })
