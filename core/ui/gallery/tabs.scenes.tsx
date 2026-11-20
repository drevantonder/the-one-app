import { Tabs, Text, type Scenes } from "ui"

const views = () => (
  <Tabs.Root defaultValue="code">
    <Tabs.List>
      <Tabs.Trigger value="code">Code</Tabs.Trigger>
      <Tabs.Trigger value="history">History</Tabs.Trigger>
      <Tabs.Trigger value="pull_requests">Pull Requests</Tabs.Trigger>
      <Tabs.Indicator />
    </Tabs.List>
    <Tabs.Content value="code">
      <Text>Three files, last changed an hour ago.</Text>
    </Tabs.Content>
    <Tabs.Content value="history">
      <Text>Twelve commits on main.</Text>
    </Tabs.Content>
    <Tabs.Content value="pull_requests">
      <Text>No pull requests.</Text>
    </Tabs.Content>
  </Tabs.Root>
)

export default {
  // What people expect of tabs, from the UI SDK's own tests (tests/specs/tabs.spec.ts at the commit tabs.ts names).
  "Tabs/The first chosen": {
    render: views,
    checks: {
      "A press chooses a tab and shows its view": [{ role: "tab", name: "History" }, { expect: { states: { of: { role: "tab", name: "History" }, is: { selected: true } } } }, { expect: { text: "Twelve commits on main." } }, { expect: { gone: "Three files, last changed an hour ago." } }],
      "The arrows choose the next tab, wrapping": [{ key: "Tab" }, { expect: { focused: { role: "tab", name: "Code" } } }, { key: "ArrowRight" }, { expect: { states: { of: { role: "tab", name: "History" }, is: { selected: true } } } }, { key: "ArrowLeft" }, { key: "ArrowLeft" }, { expect: { focused: { role: "tab", name: "Pull Requests" } } }, { expect: { text: "No pull requests." } }],
      "Home and End choose its ends": [{ key: "Tab" }, { key: "End" }, { expect: { states: { of: { role: "tab", name: "Pull Requests" }, is: { selected: true } } } }, { key: "Home" }, { expect: { states: { of: { role: "tab", name: "Code" }, is: { selected: true } } } }],
      "Up and down do nothing to tabs along": [{ key: "Tab" }, { key: "ArrowDown" }, { expect: { focused: { role: "tab", name: "Code" } } }, { expect: { states: { of: { role: "tab", name: "Code" }, is: { selected: true } } } }],
    },
  },
  "Tabs/Another chosen": { render: views, taps: ["History"] },
  "Tabs/Without an indicator": () => (
    <Tabs.Root defaultValue="day">
      <Tabs.List>
        <Tabs.Trigger value="hour">Hour</Tabs.Trigger>
        <Tabs.Trigger value="day">Day</Tabs.Trigger>
        <Tabs.Trigger value="week">Week</Tabs.Trigger>
      </Tabs.List>
    </Tabs.Root>
  ),
} satisfies Scenes
