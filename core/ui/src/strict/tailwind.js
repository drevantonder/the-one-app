// The strict style vocabulary on the DOM (style.ts): `classes`, drawn as Tailwind draws them from the tokens
// (theme.css), each only on a strict primitive (`ui-strict`, primitives.tsx), so a page's own class of the same name,
// not in strict, keeps its own look. build.js draws the UI library's with it, and app.js a page's.
import { readFileSync } from "node:fs"
import { compile } from "tailwindcss"

const theme = readFileSync(new URL("theme.css", import.meta.url), "utf8")

/** The CSS of `classes`, every one in the vocabulary (`named`). @param {string[]} classes @returns {Promise<string>} */
export async function utilities(classes) {
  const css = (await compile(`${theme}\n@tailwind utilities;`, { base: "." })).build(classes)
  const drawn = new Set(classes)
  // Each class's selector, `.hover\:bg-accent`, as itself where it's on a strict primitive. `:where` adds nothing to how
  // much it weighs against another rule.
  return css.replace(/\.((?:[\w-]|\\.)+)/g, (selector, name) => (drawn.has(name.replace(/\\(.)/g, "$1")) ? `${selector}:where(.ui-strict)` : selector))
}
