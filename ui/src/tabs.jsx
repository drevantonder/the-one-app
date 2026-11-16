import { machine } from "./zag.jsx"

// Until its machine is here, the tab that's chosen shows, and is marked as chosen.
const still = (root) => {
  const chosen = (value) => value === (root.value ?? root.defaultValue)
  return {
    getTriggerProps: (props) => ({ "data-tabs-trigger": "", role: "tab", type: "button", "data-selected": chosen(props.value) ? "" : undefined }),
    getContentProps: (props) => ({ "data-tabs-content": "", role: "tabpanel", hidden: !chosen(props.value) }),
    getIndicatorProps: () => ({ "data-tabs-indicator": "", hidden: true }),
  }
}

const { Ark } = machine("tabs", () => import("./zag/tabs.ts"), ["div", "getRootProps"], {
  List: ["div", "getListProps"],
  Trigger: ["button", "getTriggerProps", (zag) => zag.triggerProps],
  Content: ["div", "getContentProps", (zag) => zag.contentProps],
  Indicator: ["div", "getIndicatorProps"],
}, { still })

/// `<Tabs.Root defaultValue="all">`, then `<Tabs.List>` around `<Tabs.Trigger value="all">All</Tabs.Trigger>`s and
/// a `<Tabs.Indicator />`, then a `<Tabs.Content value="all">...</Tabs.Content>` for each: a few views of one thing,
/// one showing at a time. It's Ark UI's tabs (ark-ui.com/docs/components/tabs), with its parts and props: `value`
/// and `onValueChange` say which is showing.
export const Tabs = {
  ...Ark,
  Trigger: (props) => <Ark.Trigger {...props} class={["press ring reach", props.class]} />,
}
