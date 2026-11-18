// The UI library's components take what a strict page gives them (types/ui.d.ts), the DOM's and a native host's:
// `bun run typecheck` fails here where one doesn't, so a strict page runs unchanged on both.
import type * as Dom from "../index.ts"
import type * as Native from "./native/index.tsx"
import type * as Strict from "./types/ui.d.ts"

declare const dom: typeof Dom
declare const native: typeof Native
export const strict: Omit<typeof Strict, never>[] = [dom, native]
