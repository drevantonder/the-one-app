// The UI library's components take what a page gives them (types/ui.d.ts), the DOM's and a native host's:
// `bun run typecheck` fails here where one doesn't, so a page runs unchanged on both. Each exports what a page may
// use and no more, so what one has the others have too.
import type * as Dom from "./index.ts"
import type * as Native from "./index.native.tsx"
import type * as Page from "./types/ui.d.ts"

declare const dom: typeof Dom
declare const native: typeof Native
export const both: Omit<typeof Page, never>[] = [dom, native]
type None<T> = [T] extends [never] ? true : T
export const same: [None<Exclude<keyof typeof Dom, keyof typeof Page>>, None<Exclude<keyof typeof Native, keyof typeof Page>>] = [true, true]
