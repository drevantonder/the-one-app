// The UI library's components take what a page gives them (types/ui.d.ts): `bun run typecheck` fails here where one
// doesn't. It exports what a page may use and no more.
import type * as Library from "./index.ts"
import type * as Page from "./types/ui.d.ts"

declare const library: typeof Library
export const takes: Omit<typeof Page, never> = library
type None<T> = [T] extends [never] ? true : T
export const same: None<Exclude<keyof typeof Library, keyof typeof Page>> = true
