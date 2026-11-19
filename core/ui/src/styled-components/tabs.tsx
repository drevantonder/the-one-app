// Tabs: a few views of one thing, one showing at a time, chosen in a segmented control. Written once on the drawing
// primitives, the same on the DOM and on a native host.
import { createContext, createSignal, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, Text, View } from "#primitives"
import { direction } from "#bridge"
import type { Keyed, Laid } from "../types/ui.d.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  root: { gap: tokens.length.gutter, minWidth: 0 },
  list: {
    position: "relative",
    flexDirection: "row",
    alignSelf: "flex-start",
    maxWidth: "100%",
    height: tokens.length.control,
    padding: tokens.grid[0.5],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.quinaryLabel,
  },
  trigger: {
    position: "relative",
    zIndex: 1,
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.small,
    color: tokens.colour.secondaryLabel,
    font: tokens.type.medium,
    fontWeight: tokens.weight.medium,
    cursor: "pointer",
    userSelect: "none",
    boxShadow: { focused: tokens.shadow.focus },
  },
  chosen: { color: tokens.colour.label },
  // What marks the chosen tab: the tab itself, until the indicator knows where the tabs are, and then the indicator,
  // which slides to the one chosen.
  mark: { backgroundColor: tokens.colour.card, boxShadow: tokens.shadow.edge },
  indicator: (translateX: number, translateY: number, width: number, height: number) => ({
    position: "absolute",
    top: 0,
    left: 0,
    zIndex: 0,
    borderRadius: tokens.radius.small,
    pointerEvents: "none",
    transitionProperty: ["translateX", "translateY", "width", "height"],
    transition: tokens.motion.settle,
    translateX,
    translateY,
    width,
    height,
  }),
  label: { lineClamp: 1 },
  content: { gap: tokens.length.gutter },
})

/** Where a tab is drawn, from the list's corner. */
type At = { x: number; y: number; width: number; height: number }

/** What the parts of one set of tabs share. */
const Shared = createContext<{
  value: () => string | undefined
  choose: (value: string) => void
  /** The tabs, in order, each with where it's drawn once it's laid out. */
  tabs: () => { value: string; at?: At }[]
  add: (value: string) => () => void
  place: (value: string, at: At) => void
  /** Where the list is drawn, which a tab's place is measured from. */
  setList: (at: At) => void
  /** Where the chosen tab is drawn, from the list's corner, once both are laid out. */
  chosen: () => At | undefined
  /** Whether an indicator marks the chosen tab, so the tab doesn't; and what says there is one, till it's gone. */
  indicated: () => boolean
  indicate: () => () => void
  /** The tab the keyboard was last moved to, which takes it. */
  focused: () => string | undefined
  setFocused: (value: string | undefined) => void
}>()

const rectOf = (event: Laid): At => {
  const { x, y, width, height } = event.currentTarget.getBoundingClientRect()
  return { x, y, width, height }
}

/// `<Tabs.Root defaultValue="all">`, then `<Tabs.List>` around `<Tabs.Trigger value="all">All</Tabs.Trigger>`s and
/// a `<Tabs.Indicator />`, then a `<Tabs.Content value="all">...</Tabs.Content>` for each: a few views of one thing,
/// one showing at a time, chosen in a segmented control, with Ark UI's tabs' parts and props
/// (ark-ui.com/docs/components/tabs): `value` and `onValueChange` say which is showing, and with neither the first is.
/// The indicator slides to the tab chosen. The keyboard moves between the tabs as WAI-ARIA's tabs pattern has it: the
/// arrows along them (the other way right to left), Home and End to the first and last, each choosing as it goes.
export const Tabs = {
  Root: (props: { value?: string; defaultValue?: string; onValueChange?: (details: { value: string | null }) => void; children?: JSX.Element }) => {
    const [own, setOwn] = createSignal(props.defaultValue)
    const [tabs, setTabs] = createSignal<{ value: string; at?: At }[]>([])
    const [list, setList] = createSignal<At>()
    const [indicators, setIndicators] = createSignal(0)
    const [focused, setFocused] = createSignal<string>()
    const value = () => props.value ?? own() ?? tabs()[0]?.value
    const choose = (to: string) => {
      if (to === value()) return
      setOwn(to)
      props.onValueChange?.({ value: to })
    }
    const add = (tab: string) => {
      setTabs((now) => [...now, { value: tab }])
      return () => setTabs((now) => now.filter((one) => one.value !== tab))
    }
    const place = (tab: string, at: At) => setTabs((now) => now.map((one) => (one.value === tab ? { ...one, at } : one)))
    const chosen = () => {
      const from = list()
      const tab = tabs().find((one) => one.value === value())?.at
      return from && tab ? { x: tab.x - from.x, y: tab.y - from.y, width: tab.width, height: tab.height } : undefined
    }
    const indicated = () => indicators() > 0 && !!chosen()
    const indicate = () => (setIndicators((now) => now + 1), () => setIndicators((now) => now - 1))
    return (
      <Shared value={{ value, choose, tabs, add, place, setList, chosen, indicated, indicate, focused, setFocused }}>
        <View style={look.root}>{props.children}</View>
      </Shared>
    )
  },
  List: (props: { children?: JSX.Element }) => {
    const tabs = useContext(Shared)!
    const keyed = (event: Keyed) => {
      const all = tabs.tabs()
      const at = all.findIndex((one) => one.value === (tabs.focused() ?? tabs.value()))
      const back = direction() === "rtl" ? "ArrowRight" : "ArrowLeft"
      const on = direction() === "rtl" ? "ArrowLeft" : "ArrowRight"
      const to = event.key === back ? (at - 1 + all.length) % all.length : event.key === on ? (at + 1) % all.length : event.key === "Home" ? 0 : event.key === "End" ? all.length - 1 : -1
      if (to < 0 || !all[to]) return
      event.preventDefault()
      tabs.setFocused(all[to].value)
      tabs.choose(all[to].value)
    }
    return (
      <View role="tablist" aria-orientation="horizontal" onKeyDown={keyed} onLayout={(event) => tabs.setList(rectOf(event))} style={look.list}>
        {props.children}
      </View>
    )
  },
  Trigger: (props: { value: string; children?: JSX.Element }) => {
    const tabs = useContext(Shared)!
    onCleanup(tabs.add(props.value))
    const chosen = () => tabs.value() === props.value
    return (
      <Pressable
        role="tab"
        aria-selected={chosen() ? "true" : "false"}
        tabIndex={chosen() ? 0 : -1}
        autoFocus={tabs.focused() === props.value}
        onPress={() => tabs.choose(props.value)}
        onBlur={() => tabs.focused() === props.value && tabs.setFocused(undefined)}
        onLayout={(event) => tabs.place(props.value, rectOf(event))}
        style={[look.trigger, chosen() && look.chosen, chosen() && !tabs.indicated() && look.mark]}
      >
        <Text style={look.label}>{props.children}</Text>
      </Pressable>
    )
  },
  Indicator: (_props: {}) => {
    const tabs = useContext(Shared)!
    onCleanup(tabs.indicate())
    return (
      <Show when={tabs.chosen()}>
        {(at) => <View aria-hidden="true" style={[look.indicator(at().x, at().y, at().width, at().height), look.mark]} />}
      </Show>
    )
  },
  Content: (props: { value: string; children?: JSX.Element }) => {
    const tabs = useContext(Shared)!
    return (
      <Show when={tabs.value() === props.value}>
        <View role="tabpanel" style={look.content}>
          {props.children}
        </View>
      </Show>
    )
  },
}
