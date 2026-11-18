// Every component of the UI library in each of its states, on one strict page, light and dark: what a change to how
// one looks is checked against, before and after, on the DOM and on a native host. `bun app.js gallery <out>` in
// core/ui builds it as an app's page is built; nothing here calls a tool.
import { createListCollection } from "ui"
import { Badge, Box, Button, Checkbox, Empty, Field, Icon, Input, List, Menu, Problem, RadioGroup, render, Row, Screen, Search, Section, Select, Skeleton, Spinner, Status, Steps, Switch, Text } from "ui"

const fruits = createListCollection({ items: [{ value: "apple", label: "Apple" }, { value: "pear", label: "Pear" }] })

render(() => (
  <Screen heading="Gallery" action={<Button variant="ghost" size="icon" aria-label="Add"><Icon name="plus" /></Button>}>
    <Section heading="Text">
      <Text style={{ font: "title" }}>Title</Text>
      <Text style={{ font: "figure" }}>1,234</Text>
      <Text style={{ font: "label" }}>Label</Text>
      <Text>Body text, which runs on over more than one line where there isn't room for all of it in one.</Text>
      <Text style={{ font: "subheadline" }}>Subheadline</Text>
      <Text style={{ font: "footnote", color: "muted-foreground" }}>Footnote, muted</Text>
      <Text style={{ font: "caption-1" }}>Caption 1</Text>
      <Text style={{ font: "caption-2" }}>Caption 2</Text>
    </Section>
    <Section heading="Buttons">
      <Box style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
        <Button>Default</Button>
        <Button variant="secondary">Secondary</Button>
        <Button variant="outline">Outline</Button>
        <Button variant="ghost">Ghost</Button>
        <Button variant="destructive">Delete</Button>
        <Button loading>Saving</Button>
        <Button disabled>Disabled</Button>
        <Button variant="outline" size="icon" aria-label="More"><Icon name="dots-three" /></Button>
      </Box>
      <Box style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
        <Badge>3</Badge>
        <Badge variant="secondary">New</Badge>
        <Badge variant="destructive">12</Badge>
        <Badge variant="outline">Beta</Badge>
        <Spinner />
      </Box>
    </Section>
    <Section heading="List">
      <List>
        <Row heading="A row" text="Its grey line, which says up to two lines of what it says before it's cut off at the end" onClick={() => {}} />
        <Row heading="The current one" text="Open beside the list" current onClick={() => {}} />
        <Row heading="With an end" end={<Text style={{ color: "muted-foreground", fontVariantNumeric: "tabular-nums" }}>12:30</Text>} />
        <Row heading="With a control" control={<Checkbox aria-label="Done" checked />} action={<Menu.Root><Menu.Trigger aria-label="More" /><Menu.Positioner><Menu.Content><Menu.Item value="a">Rename</Menu.Item><Menu.Item value="b" destructive>Delete</Menu.Item></Menu.Content></Menu.Positioner></Menu.Root>} />
      </List>
    </Section>
    <Section heading="Fields">
      <Field label="Name" hint="As people see it">
        <Input placeholder="Your name" />
      </Field>
      <Field label="Email" error="That isn't an email address">
        <Input value="andre@" aria-invalid="true" />
      </Field>
      <Search placeholder="Search" aria-label="Search" />
      <Switch label="Notifications" hint="On this device" checked />
      <Checkbox label="Remember me" />
      <RadioGroup value="a" options={[{ value: "a", label: "Daily" }, { value: "b", label: "Weekly" }]} />
      <Select.Root collection={fruits} value={["apple"]} />
    </Section>
    <Section heading="States">
      <Empty>Nothing here yet.</Empty>
      <Problem heading="Couldn't load" text="The server didn't answer." action={<Button variant="outline">Try Again</Button>} />
      <Problem warn heading="Needs you" text="Sign in again." />
      <Status heading="Backing up" text="12 of 40 files" />
      <Steps steps={["Starting", "Signing in", "Ready"]} at={1} />
      <Skeleton rows={2} />
    </Section>
  </Screen>
))
