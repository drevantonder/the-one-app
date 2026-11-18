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
  "Tabs/The first chosen": views,
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
