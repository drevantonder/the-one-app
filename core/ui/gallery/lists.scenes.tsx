import { createSignal } from "solid-js"
import { Badge, Button, Checkbox, Icon, List, Menu, Metadata, Row, type Scenes, Section, styles, Text, tokens, View } from "ui"

const look = styles({
  column: { gap: tokens.grid[2] },
  row: { flexDirection: "row", gap: tokens.grid[2] },
  time: { color: tokens.colour.secondaryLabel },
})

const Rows = () => {
  const [notes, setNotes] = createSignal(["First note", "Second note", "Third note"])
  return (
    <View style={look.column}>
      <View style={look.row}>
        <Button variant="outline" onPress={() => setNotes([...notes().slice(0, 1), `Note ${notes().length + 1}`, ...notes().slice(1)])}>
          Add Row
        </Button>
        <Button variant="outline" onPress={() => setNotes(notes().slice(0, 1).concat(notes().slice(2)))}>
          Remove Row
        </Button>
      </View>
      <List each={notes()}>{(note) => <Row heading={note()} />}</List>
    </View>
  )
}

const Sections = () => (
  <>
    <Section heading="Passwords & Keys">
      <List>
        <Row start={<Icon name="file" />} heading="Cloudflare API" text="api.cloudflare.com" onPress={() => {}} />
        <Row start={<Icon name="file" />} heading="GitHub" text="github.com and api.github.com" current onPress={() => {}} />
        <Row start={<Icon name="file" />} heading="OpenRouter" end={<Text>14 Nov</Text>} onPress={() => {}} />
      </List>
    </Section>
    <Section heading="Asked For">
      <List>
        <Row start={<Icon name="file" />} heading="Stripe secret key" end={<Badge variant="destructive">1</Badge>} onPress={() => {}} />
      </List>
    </Section>
  </>
)

const Details = () => (
  <Section heading="Information">
    <Metadata>
      <Metadata.Label title="Username" text="drevan" />
      <Metadata.Label title="Token" text="•••••••••••••" />
      <Metadata.Label title="Your agents" text="Use it, never see it" />
      <Metadata.Label title="Added" text="14 Nov at 09:12" />
    </Metadata>
  </Section>
)

export default {
  "Lists/Metadata on a computer": { size: [750, 475], render: Details },
  "Lists/Metadata under a finger": { touch: true, render: Details },
  "Lists/Sections on a computer": {
    size: [750, 475],
    render: Sections,
    checks: {
      // Each list is one tab stop, the row open beside it first, and the arrows, Home, End and letters move in it.
      "The keyboard in a list": [
        { key: "Tab" },
        { expect: { focused: "GitHub github.com and api.github.com" } },
        { key: "ArrowDown" },
        { expect: { focused: "OpenRouter 14 Nov" } },
        { key: "ArrowDown" },
        { expect: { focused: "OpenRouter 14 Nov" } },
        { key: "Home" },
        { expect: { focused: "Cloudflare API api.cloudflare.com" } },
        { key: "o" },
        { expect: { focused: "OpenRouter 14 Nov" } },
        { key: "Tab" },
        { expect: { focused: "Stripe secret key 1" } },
        { key: "Tab", shift: true },
        { expect: { focused: "OpenRouter 14 Nov" } },
      ],
    },
  },
  "Lists/Sections under a finger": { touch: true, render: Sections },
  "Lists/Rows": () => (
    <List>
      <Row heading="A row" text="Its grey line, which says up to two lines of what it says before it's cut off at the end" onPress={() => {}} />
      <Row heading="The current one" text="Open beside the list" current onPress={() => {}} />
      <Row heading="With an end" end={<Text style={look.time}>12:30</Text>} />
      <Row
        heading="With a control"
        control={<Checkbox aria-label="Done" value />}
        action={
          <Menu.Root>
            <Menu.Trigger aria-label="More" />
            <Menu.Positioner>
              <Menu.Content>
                <Menu.Item value="a">Rename</Menu.Item>
                <Menu.Item value="b" destructive>
                  Delete
                </Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Menu.Root>
        }
      />
    </List>
  ),
  "Lists/Rows coming and going": () => <Rows />,
  "Lists/A row added": { render: () => <Rows />, taps: ["Add Row"] },
  "Lists/A long list": () => (
    <List each={Array.from({ length: 10000 }, (_, at) => `Note ${at + 1}`)}>{(note) => <Row heading={note()} text="19 Nov" />}</List>
  ),
} satisfies Scenes
