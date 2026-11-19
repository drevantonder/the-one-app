// How a screen on a computer gathers what can be done in it, as Raycast's action panel lists it: its bar's buttons
// and menus' items, and what the row chosen in it can do.
import { createContext, createEffect, useContext } from "solid-js"
import type { Action } from "./actions.tsx"

/** What a screen's bar gives what's in it on a computer: adds an action to the screen's, until what added it goes. */
export const Collect = createContext<((action: Action) => () => void) | null>(null)

/** What a screen gives its rows on a computer: says what the row chosen (under the pointer, or open beside the list)
 * can do (`set`, by the row, which is what it's known by), and that a row that's going can no longer (`clear`). */
export const Choose = createContext<{ set: (row: object, actions: (() => Action[]) | undefined) => void; clear: (row: object) => void } | null>(null)

/// Adds an action to the screen's while it's there, as it changes, where the screen's bar is gathering them: whether
/// one is, so what adds it draws nothing of its own.
export function collected(action: () => Action): boolean {
  const collect = useContext(Collect)
  if (collect) createEffect(action, (now) => collect(now))
  return !!collect
}
