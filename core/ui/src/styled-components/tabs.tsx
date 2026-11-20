// Tabs: a few views of one thing, one showing at a time, chosen in a segmented control. Written once on the drawing
// primitives, the same on the DOM and on a native host.
import { createContext, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, Text, View } from "#primitives"
import { createTabs, type TabsProps } from "../headless-components/tabs.ts"
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
  mark: { backgroundColor: tokens.colour.card, boxShadow: { default: tokens.shadow.edge, focused: tokens.shadow.focus } },
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

/** What the parts of one set of tabs share. */
const Shared = createContext<ReturnType<typeof createTabs>>()

/// `<Tabs.Root defaultValue="all">`, then `<Tabs.List>` around `<Tabs.Trigger value="all">All</Tabs.Trigger>`s and
/// a `<Tabs.Indicator />`, then a `<Tabs.Content value="all">...</Tabs.Content>` for each: a few views of one thing,
/// one showing at a time, chosen in a segmented control: `value` says which is showing, `onValueChange` is told the
/// value chosen, and with neither the first is.
/// The indicator slides to the tab chosen. The keyboard moves between the tabs as WAI-ARIA's tabs pattern has it: the
/// arrows along them (the other way right to left), Home and End to the first and last, each choosing as it goes.
export const Tabs = {
  Root: (props: TabsProps & { children?: JSX.Element }) => (
    <Shared value={createTabs(props)}>
      <View style={look.root}>{props.children}</View>
    </Shared>
  ),
  List: (props: { children?: JSX.Element }) => (
    <View {...useContext(Shared)!.list()} style={look.list}>
      {props.children}
    </View>
  ),
  Trigger: (props: { value: string; children?: JSX.Element }) => {
    const tabs = useContext(Shared)!
    onCleanup(tabs.add(props.value))
    const chosen = () => tabs.value() === props.value
    return (
      <Pressable {...tabs.trigger(props.value)} style={[look.trigger, chosen() && look.chosen, chosen() && !tabs.indicated() && look.mark]}>
        <Text style={look.label}>{props.children}</Text>
      </Pressable>
    )
  },
  Indicator: (_props: {}) => {
    const tabs = useContext(Shared)!
    onCleanup(tabs.indicate())
    return (
      <Show when={tabs.selected()}>
        {(at) => <View aria-hidden="true" style={[look.indicator(at().x, at().y, at().width, at().height), look.mark]} />}
      </Show>
    )
  },
  Content: (props: { value: string; children?: JSX.Element }) => {
    const content = useContext(Shared)!.content(props.value)
    return (
      <Show when={content.shown()}>
        <View role={content.role} style={look.content}>
          {props.children}
        </View>
      </Show>
    )
  },
}
