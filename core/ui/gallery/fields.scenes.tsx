import { createSignal } from "solid-js"
import { Checkbox, createListCollection, Field, Input, RadioGroup, type Scenes, Search, Select, styles, Switch, Text, Textarea, tokens, View } from "ui"

const look = styles({
  form: { gap: tokens.grid[4] },
  narrow: (width: number) => ({ gap: tokens.grid[2], width }),
})

const fruits = createListCollection({ items: [{ value: "apple", label: "Apple" }, { value: "pear", label: "Pear" }] })

const Fields = () => {
  const [code, setCode] = createSignal("")
  return (
    <View style={look.form}>
      <Field label="Name" hint="As people see it">
        <Input placeholder="Your name" />
      </Field>
      <Field label="Email" error="That isn't an email address">
        <Input value="andre@" aria-invalid="true" />
      </Field>
      <Field label="Code" hint="Digits only">
        <Input inputMode="numeric" value={code()} onChangeText={(text) => setCode(text.replace(/[^0-9]/g, ""))} />
      </Field>
      <Field label="Password">
        <Input secureTextEntry value="hunter2" autoComplete="current-password" />
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
    </View>
  )
}

/// A field and what's in it, as text, so a scene's tree says what was typed and where.
const Echoed = (props: { label: string; value?: string; multiline?: boolean }) => {
  const [value, setValue] = createSignal(props.value ?? "")
  return (
    <View style={look.narrow(240)}>
      <Field label={props.label}>
        {props.multiline ? <Textarea value={value()} onChangeText={setValue} /> : <Input value={value()} onChangeText={setValue} />}
      </Field>
      <Text>{`[${value()}]`}</Text>
    </View>
  )
}

export default {
  "Fields/Every kind": () => <Fields />,
  "Fields/A select open": { render: () => <Fields />, taps: [{ role: "combobox", name: "Fruit" }] },
  // The caret steps over an emoji, a flag and a letter with its accent whole, and Backspace takes an emoji whole:
  // End, then back past "é" and the flag, and Backspace deletes the family, leaving "a🇦🇺é".
  "Fields/Caret by grapheme": {
    render: () => <Echoed label="Emoji" value={"a👨‍👩‍👧🇦🇺e\u0301"} />,
    taps: [{ role: "textbox", name: "Emoji" }, { key: "End" }, { key: "ArrowLeft" }, { key: "ArrowLeft" }, { key: "Backspace" }],
  },
  // Undo takes back a word at a time, as macOS's and Flutter's fields do: "hello world" typed, then undone once,
  // leaves "hello ". (Chromium undoes the whole run, so the DOM's is empty.)
  "Fields/Undo a word": {
    render: () => <Echoed label="Note" />,
    taps: [{ role: "textbox", name: "Note" }, { type: "hello world" }, { key: "z", ctrl: true }],
  },
  // Where a line wraps after a hyphen, with no space to hang, End puts the caret at the end of its line, not the
  // start of the next (Flutter's TextAffinity.upstream), so Down goes on to the next line's end and "|" is typed
  // there: pressed in the middle, then Home, End, Down. (Chromium's Down goes to the next line's start, so the DOM's differs.)
  "Fields/Caret at a wrap": {
    render: () => <Echoed label="Note" multiline value="well-known-long-hyphen-ated-words-wrap-with-no-space" />,
    taps: [{ role: "textbox", name: "Note" }, { key: "Home" }, { key: "End" }, { key: "ArrowDown" }, { type: "|" }],
  },
} satisfies Scenes
