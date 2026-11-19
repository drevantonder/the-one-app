// Tabs, with their state and no look: which is selected, the keyboard between them, and where the selected one is
// drawn, which an indicator slides to. From Vercel's UI SDK (Apache-2.0),
// packages/core/src/components/tabs/tabs.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7: a controlled or uncontrolled
// `value`, the first tab until one is chosen; the arrows along the tabs, wrapping (`loop`), each selecting as it goes
// (`activation: "automatic"`); the tab stop on the selected tab; and the indicator measured from the list
// (foundation/selection/indicator.ts), on our host's layout (`onLayout`).
import { createSignal, merge } from "solid-js"
import { createControllableSignal } from "../behavior-primitives/controllable.ts"
import { createRovingFocus } from "../behavior-primitives/roving-focus.ts"
import type { Keyed, Laid } from "../types/ui.d.ts"

/** Where something is drawn. */
type At = { x: number; y: number; width: number; height: number }

const rectOf = (event: Laid): At => {
  const { x, y, width, height } = event.currentTarget.getBoundingClientRect()
  return { x, y, width, height }
}

/** A `Tabs.Root`'s props, as Ark UI's tabs name them. */
export interface TabsProps {
  value?: string
  defaultValue?: string
  onValueChange?: (details: { value: string | null }) => void
}

/** `const tabs = createTabs(props)`: the state of one set of tabs, and the props of each of its parts. */
export function createTabs(props: TabsProps) {
  const [tabs, setTabs] = createSignal<{ value: string; at?: At }[]>([])
  const [list, setList] = createSignal<At>()
  const [indicators, setIndicators] = createSignal(0)
  const [chosen, choose] = createControllableSignal<string>({ value: () => props.value, defaultValue: () => props.defaultValue, onChange: (value) => props.onValueChange?.({ value }) })
  const value = () => chosen() ?? tabs()[0]?.value
  const roving = createRovingFocus({ items: () => tabs().map((tab) => ({ key: tab.value, preferred: tab.value === value() })), orientation: "horizontal", loop: true, remember: false })
  /** Where the selected tab is drawn, from the list's corner, once both are laid out. */
  const selected = () => {
    const from = list()
    const tab = tabs().find((one) => one.value === value())?.at
    return from && tab ? { x: tab.x - from.x, y: tab.y - from.y, width: tab.width, height: tab.height } : undefined
  }
  return {
    value,
    selected,
    /** Whether an indicator marks the selected tab, so the tab doesn't mark itself. */
    indicated: () => indicators() > 0 && !!selected(),
    /** Says an indicator is drawn. → what says it's gone. */
    indicate: () => (setIndicators((now) => now + 1), () => setIndicators((now) => now - 1)),
    /** Adds a tab, in order. → what takes it out. */
    add: (tab: string) => (setTabs((now) => [...now, { value: tab }]), () => setTabs((now) => now.filter((one) => one.value !== tab))),
    list: () => ({
      role: "tablist" as const,
      "aria-orientation": "horizontal" as const,
      onKeyDown: (event: Keyed) => {
        const to = roving.keydown(event)
        if (to !== undefined) choose(to)
      },
      onLayout: (event: Laid) => setList(rectOf(event)),
    }),
    trigger: (tab: string) =>
      merge(roving.item(tab), {
        role: "tab" as const,
        get "aria-selected"() {
          return value() === tab ? ("true" as const) : ("false" as const)
        },
        onPress: () => choose(tab),
        onLayout: (event: Laid) => setTabs((now) => now.map((one) => (one.value === tab ? { ...one, at: rectOf(event) } : one))),
      }),
    content: (tab: string) => ({ shown: () => value() === tab, role: "tabpanel" as const }),
  }
}
