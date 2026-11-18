// The UI library's DOM components take what a strict app gives them (types/ui.d.ts): `bun run typecheck` fails here
// where one doesn't, so a strict app runs unchanged on the DOM.
import type * as Dom from "../index.ts"
import type * as Strict from "./types/ui.d.ts"

declare const dom: typeof Dom
export const strict: Omit<typeof Strict, never> = dom
