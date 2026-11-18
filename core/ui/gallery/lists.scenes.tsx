import { createSignal, For } from "solid-js"
import { Badge, Box, Button, Checkbox, Icon, List, Menu, Row, Section, Text, type Scenes } from "ui"

const Rows = () => {
  const [notes, setNotes] = createSignal(["First note", "Second note", "Third note"])
  return (
    <Box style={{ gap: 8 }}>
      <Box style={{ flexDirection: "row", gap: 8 }}>
        <Button variant="outline" onClick={() => setNotes([...notes().slice(0, 1), `Note ${notes().length + 1}`, ...notes().slice(1)])}>
          Add Row
        </Button>
        <Button variant="outline" onClick={() => setNotes(notes().slice(0, 1).concat(notes().slice(2)))}>
          Remove Row
        </Button>
      </Box>
      <List>
        <For each={notes()}>{(note) => <Row heading={note} />}</For>
      </List>
    </Box>
  )
}

const Sections = () => (
  <>
    <Section heading="Passwords & Keys">
      <List>
        <Row start={<Icon name="file" />} heading="Cloudflare API" text="api.cloudflare.com" onClick={() => {}} />
        <Row start={<Icon name="file" />} heading="GitHub" text="github.com and api.github.com" current onClick={() => {}} />
        <Row start={<Icon name="file" />} heading="OpenRouter" end={<Text>14 Nov</Text>} onClick={() => {}} />
      </List>
    </Section>
    <Section heading="Asked For">
      <List>
        <Row start={<Icon name="file" />} heading="Stripe secret key" end={<Badge variant="destructive">1</Badge>} onClick={() => {}} />
      </List>
    </Section>
  </>
)

export default {
  "Lists/Sections on a computer": { size: [750, 475], render: Sections },
  "Lists/Sections under a finger": { touch: true, render: Sections },
  "Lists/Rows": () => (
    <List>
      <Row heading="A row" text="Its grey line, which says up to two lines of what it says before it's cut off at the end" onClick={() => {}} />
      <Row heading="The current one" text="Open beside the list" current onClick={() => {}} />
      <Row heading="With an end" end={<Text style={{ color: "muted-foreground", fontVariantNumeric: "tabular-nums" }}>12:30</Text>} />
      <Row
        heading="With a control"
        control={<Checkbox aria-label="Done" checked />}
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
} satisfies Scenes
