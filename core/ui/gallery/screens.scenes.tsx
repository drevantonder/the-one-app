import { Button, History, List, ListStyle, Menu, Row, type Scenes, Screen, Section, Split, styles, Text, View } from "ui"

const look = styles({ window: { height: 720 }, narrow: { height: 475 }, phone: { height: 844 } })

const notes = Array.from({ length: 30 }, (_, at) => `Note ${at + 1}`)

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
  // A phone's first screen: its name large at the top of what scrolls, which goes under the bar as it scrolls, the
  // bar's small name taking its place and the bar its glass and hairline, as UINavigationBar's large titles do.
  "Screens/A phone's first screen": {
    alone: true,
    touch: true,
    render: () => (
      <View style={look.phone}>
        <Screen heading="Notes" action={<Button>Add</Button>}>
          <List>{notes.map((note) => <Row heading={note} />)}</List>
        </Screen>
      </View>
    ),
    checks: {
      "Scrolled, its large title goes under the bar, which takes its glass": [{ expect: { blur: false } }, { trace: "traces/finger-pans-up.json" }, { expect: { blur: true } }, { expect: { shown: { role: "heading", name: "Notes" } } }],
      // iOS's sizes at its usual text size (17 pt): the large title 34 pt (`.largeTitle`), the bar's 17 (`.headline`), each
      // with its lines' height and the room around them.
      "On a phone, the large title is 34 pt and the bar's title 17": [
        { expect: { tall: { of: { role: "heading", name: "Notes" }, height: 49.3 } } },
        { trace: "traces/finger-pans-up.json" },
        { expect: { tall: { of: { role: "heading", name: "Notes" }, height: 34 } } },
      ],
    },
    motions: { "Scrolled under its bar": { from: { x: 195, y: 600 }, gesture: { swipe: [0, -200], speed: 400 }, follow: { role: "heading", name: "Notes" } } },
  },
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
