// Every component of the UI library in each of its states, on one strict page, light and dark: what a change to how
// one looks is checked against, before and after, on the DOM and on a native host. `bun app.js gallery <out>` in
// core/ui builds it as an app's page is built; nothing here calls a tool.
import { createListCollection } from "ui"
import { createSignal, For } from "solid-js"
import { Alert, Badge, Box, Button, Sheet, Checkbox, Empty, Field, Icon, Input, List, Textarea, Menu, Problem, RadioGroup, render, Row, Screen, Search, Section, Select, Skeleton, Spinner, Status, Steps, Switch, Text } from "ui"

const [over, setOver] = createSignal<"sheet" | "alert">()
const [notes, setNotes] = createSignal(["First note", "Second note", "Third note"])
const [code, setCode] = createSignal("")
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
    <Section heading="Over the page">
      <Box style={{ flexDirection: "row", gap: 8 }}>
        <Button variant="outline" onClick={() => setOver("sheet")}>Open Sheet</Button>
        <Button variant="outline" onClick={() => setOver("alert")}>Open Alert</Button>
        <Menu.Root>
          <Menu.Trigger aria-label="Open Menu" />
          <Menu.Positioner>
            <Menu.Content>
              <Menu.Group label="Install for">
                <Menu.Item value="me">Just Me</Menu.Item>
                <Menu.Item value="family">Family</Menu.Item>
              </Menu.Group>
              <Menu.Separator />
              <Menu.Item value="rename">Rename</Menu.Item>
              <Menu.Item value="off" disabled>Turn Off</Menu.Item>
              <Menu.Item value="delete" destructive>Delete</Menu.Item>
            </Menu.Content>
          </Menu.Positioner>
        </Menu.Root>
      </Box>
      <Sheet heading="Add Note" open={over() === "sheet"} onClose={() => setOver()}>
        <Text>What a sheet holds, which scrolls where it's taller than the window allows.</Text>
        <Button>Save</Button>
      </Sheet>
      <Alert heading="Vault wants to use a key" text="Sent only to api.github.com." open={over() === "alert"} onClose={() => setOver()} action={<><Button variant="outline" onClick={() => setOver()}>Don't Allow</Button><Button onClick={() => setOver()}>Allow</Button></>} />
    </Section>
    <Section heading="List">
      <List>
        <Row heading="A row" text="Its grey line, which says up to two lines of what it says before it's cut off at the end" onClick={() => {}} />
        <Row heading="The current one" text="Open beside the list" current onClick={() => {}} />
        <Row heading="With an end" end={<Text style={{ color: "muted-foreground", fontVariantNumeric: "tabular-nums" }}>12:30</Text>} />
        <Row heading="With a control" control={<Checkbox aria-label="Done" checked />} action={<Menu.Root><Menu.Trigger aria-label="More" /><Menu.Positioner><Menu.Content><Menu.Item value="a">Rename</Menu.Item><Menu.Item value="b" destructive>Delete</Menu.Item></Menu.Content></Menu.Positioner></Menu.Root>} />
      </List>
    </Section>
    <Section heading="Rows coming and going">
      <Box style={{ flexDirection: "row", gap: 8 }}>
        <Button variant="outline" onClick={() => setNotes([...notes().slice(0, 1), `Note ${notes().length + 1}`, ...notes().slice(1)])}>Add Row</Button>
        <Button variant="outline" onClick={() => setNotes(notes().slice(0, 1).concat(notes().slice(2)))}>Remove Row</Button>
      </Box>
      <List>
        <For each={notes()}>{(note) => <Row heading={note} />}</For>
      </List>
    </Section>
    <Section heading="Fields">
      <Field label="Name" hint="As people see it">
        <Input placeholder="Your name" />
      </Field>
      <Field label="Email" error="That isn't an email address">
        <Input value="andre@" aria-invalid="true" />
      </Field>
      <Field label="Code" hint="Digits only">
        <Input inputmode="numeric" value={code()} onInput={(event) => setCode(event.currentTarget.value.replace(/[^0-9]/g, ""))} />
      </Field>
      <Field label="Password">
        <Input type="password" value="hunter2" autocomplete="current-password" />
      </Field>
      <Field label="Off" disabled>
        <Input value="Can't change this" />
      </Field>
      <Field label="Note">
        <Textarea placeholder="Anything else" />
      </Field>
      <Search placeholder="Search" aria-label="Search" />
      <Search placeholder="Search" aria-label="Search found" value="git" />
      <Switch label="Notifications" hint="On this device" checked />
      <Switch label="Off" />
      <Checkbox label="Remember me" />
      <Checkbox label="Ticked" checked />
      <Checkbox label="Must be ticked" error="Tick this to go on." />
      <Field label="How often">
        <RadioGroup value="a" options={[{ value: "a", label: "Daily" }, { value: "b", label: "Weekly" }, { value: "c", label: "Never", disabled: true }]} />
      </Field>
      <Field label="Fruit">
        <Select.Root collection={fruits} value={["apple"]} />
      </Field>
      <Field label="Nothing chosen">
        <Select.Root collection={fruits} placeholder="Pick one" />
      </Field>
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
