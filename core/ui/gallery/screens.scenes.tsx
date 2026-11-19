import { Button, History, List, ListStyle, Menu, Row, type Scenes, Screen, Section, Split, styles, Text, View } from "ui"

const look = styles({ window: { height: 720 }, narrow: { height: 475 } })

const More = () => (
  <Menu.Root onSelect={() => {}}>
    <Menu.Trigger />
    <Menu.Positioner>
      <Menu.Content>
        <Menu.Item value="remove" destructive>Remove</Menu.Item>
      </Menu.Content>
    </Menu.Positioner>
  </Menu.Root>
)

const Devices = () => (
  <Screen heading="Devices" action={<Button>Invite</Button>}>
    <Section heading="Your Devices">
      <List>
        <Row heading="Alex's Laptop" text="This device" onPress={() => {}} />
        <Row heading="Sam's Phone" text="Seen 3 hours ago" end={<More />} />
        <Row heading="Pixel Tablet" text="Selected" current onPress={() => {}} />
      </List>
    </Section>
  </Screen>
)

// A window of grouped cards: a sidebar, and what it opens, which opens its own list's rows over that list.
const settings = () => (
  <ListStyle value="sidebar">
    <Split
      open
      detail={
        <ListStyle value="insetGrouped">
          <History value={{ back: () => {} }}>
            <Split detail={<Screen heading="Alex's Laptop" back><Text>Its page.</Text></Screen>}><Devices /></Split>
          </History>
        </ListStyle>
      }
    >
      <Screen heading="Settings">
        <List>
          <Row heading="Devices" current onPress={() => {}} />
          <Row heading="People & Groups" onPress={() => {}} />
        </List>
      </Screen>
    </Split>
  </ListStyle>
)

export default {
  "Screens/Settings": { alone: true, size: [1100, 720], render: () => <View style={look.window}>{settings()}</View> },
  "Screens/A large title": {
    alone: true,
    size: [750, 475],
    render: () => (
      <View style={look.narrow}>
        <Screen heading="Notes" back largeTitle>
          <Text role="heading" aria-level={2}>Notes</Text>
        </Screen>
      </View>
    ),
  },
} satisfies Scenes
