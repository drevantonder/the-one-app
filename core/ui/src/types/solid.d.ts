// What Solid 2.0 has at runtime that its own public types leave out.
import type { Accessor } from "solid-js"

declare module "solid-js" {
  /** Runs `fn`, or `fallback` with the error it threw and what retries it, as `<Errored>` does: an error boundary as
   * a function. */
  export const createErrorBoundary: <T, U>(fn: () => T, fallback: (error: Accessor<unknown>, reset: () => void) => U) => Accessor<T | U>
}
