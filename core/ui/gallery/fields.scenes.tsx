import { createSignal } from "solid-js"
import { Checkbox, Field, Input, RadioGroup, type Scenes, Search, Select, styles, Switch, Text, Textarea, tokens, View } from "ui"

const look = styles({
  form: { gap: tokens.grid[4] },
  narrow: (width: number) => ({ gap: tokens.grid[2], width }),
})

const fruits = [{ value: "apple", label: "Apple" }, { value: "pear", label: "Pear" }]

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
      <Switch label="Notifications" hint="On this device" value />
      <Switch label="Off" />
      <Checkbox label="Remember me" />
      <Checkbox label="Ticked" value />
      <Checkbox label="Must be ticked" error="Tick this to go on." />
      <Field label="How often">
        <RadioGroup value="a" options={[{ value: "a", label: "Daily" }, { value: "b", label: "Weekly" }, { value: "c", label: "Never", disabled: true }]} />
      </Field>
      <Field label="Fruit">
        <Select.Root items={fruits} value="apple" />
      </Field>
      <Field label="Nothing chosen">
        <Select.Root items={fruits} placeholder="Pick one" />
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

/// A select alone, and what's chosen, as text.
const Picked = () => {
  const [fruit, setFruit] = createSignal("apple")
  const items = [...fruits, { value: "grape", label: "Grape", disabled: true }, { value: "lemon", label: "Lemon" }]
  return (
    <View style={look.form}>
      <Field label="Fruit">
        <Select.Root items={items} value={fruit()} onValueChange={setFruit} />
      </Field>
      <Text>{`Chosen: ${fruit()}`}</Text>
    </View>
  )
}

const face = { role: "combobox", name: "Fruit" }

/// A field of each kind a phone's keyboard treats differently, as iOS's and Android's own fields do.
const Keyboards = () => (
  <View style={look.form}>
    <Field label="Name">
      <Input />
    </Field>
    <Search placeholder="Search" aria-label="Search" />
    <Field label="Email">
      <Input inputMode="email" />
    </Field>
    <Field label="Link">
      <Input inputMode="url" />
    </Field>
    <Field label="Username">
      <Input autoComplete="username" />
    </Field>
    <Field label="Code">
      <Input inputMode="numeric" autoComplete="one-time-code" />
    </Field>
    <Field label="Note">
      <Textarea />
    </Field>
  </View>
)
/// What a field's keyboard is asked for, as Flutter's TextInputConfiguration says it.
const kept = { textCapitalization: "TextCapitalization.none", autocorrect: false }
const sentences = { textCapitalization: "TextCapitalization.sentences", autocorrect: true }

export default {
  // What people expect of a select, from the UI SDK's own tests (tests/specs/select.spec.ts at the commit select.ts
  // names).
  "Fields/A select chosen by its keys": { render: () => <Picked />, taps: [{ key: "Tab" }, { key: "Enter" }, { key: "ArrowDown" }, { key: "Enter" }] },
  "Fields/A select and what's chosen": {
    render: () => <Picked />,
    checks: {
      "A press opens its list, drawn whole": [face, { expect: { shown: { role: "listbox" } } }, { expect: { states: { of: face, is: { expanded: true } } } }],
      "A press on its face again closes it": [face, { wait: 300 }, face, { expect: { gone: { role: "listbox" } } }],
      "A press on an option chooses it and closes it": [face, { wait: 300 }, { role: "option", name: "Pear" }, { expect: { text: "Chosen: pear" } }, { expect: { gone: { role: "listbox" } } }],
      "A press on one that's off does nothing": [face, { wait: 300 }, { role: "option", name: "Grape" }, { expect: { text: "Chosen: apple" } }, { expect: { shown: { role: "listbox" } } }],
      "Enter opens it, the arrows move past what's off, Enter chooses": [{ key: "Tab" }, { key: "Enter" }, { key: "ArrowDown" }, { key: "ArrowDown" }, { key: "Enter" }, { expect: { text: "Chosen: lemon" } }, { expect: { gone: { role: "listbox" } } }, { expect: { focused: face } }],
      "Escape closes it, choosing nothing": [{ key: "Tab" }, { key: "Enter" }, { key: "ArrowDown" }, { key: "Escape" }, { expect: { text: "Chosen: apple" } }, { expect: { gone: { role: "listbox" } } }, { expect: { focused: face } }],
      "Closed, the arrows choose": [{ key: "Tab" }, { key: "ArrowDown" }, { expect: { text: "Chosen: pear" } }, { expect: { gone: { role: "listbox" } } }],
      "A press outside closes it": [face, { wait: 300 }, { x: 380, y: 830 }, { expect: { gone: { role: "listbox" } } }, { expect: { text: "Chosen: apple" } }],
    },
  },
  "Fields/Every kind": () => <Fields />,
  // What each field asks a phone's keyboard for: a search, an address, a username or a code taken as it's typed, as
  // UISearchBar and Android's SearchView are; text capitalised as sentences and corrected.
  "Fields/What each asks the keyboard for": {
    render: () => <Keyboards />,
    touch: true,
    checks: {
      "A name is capitalised as sentences": [{ role: "textbox", name: "Name" }, { expect: { keyboard: sentences } }],
      "A search isn't capitalised or corrected": [{ role: "searchbox", name: "Search" }, { expect: { keyboard: kept } }],
      "An email isn't": [{ role: "textbox", name: "Email" }, { expect: { keyboard: kept } }],
      "A link isn't": [{ role: "textbox", name: "Link" }, { expect: { keyboard: kept } }],
      "A username isn't": [{ role: "textbox", name: "Username" }, { expect: { keyboard: kept } }],
      "A code isn't": [{ role: "textbox", name: "Code" }, { expect: { keyboard: kept } }],
      "A note is capitalised as sentences": [{ role: "textbox", name: "Note" }, { expect: { keyboard: sentences } }],
    },
  },
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
