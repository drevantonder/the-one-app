import { createSignal } from "solid-js"
import { Box, Checkbox, createListCollection, Field, Input, RadioGroup, Search, Select, Switch, Textarea, type Scenes } from "ui"

const fruits = createListCollection({ items: [{ value: "apple", label: "Apple" }, { value: "pear", label: "Pear" }] })

const Fields = () => {
  const [code, setCode] = createSignal("")
  return (
    <Box style={{ gap: 16 }}>
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
    </Box>
  )
}

export default {
  "Fields/Every kind": () => <Fields />,
  "Fields/A select open": { render: () => <Fields />, taps: [{ role: "combobox", name: "Fruit" }] },
} satisfies Scenes
