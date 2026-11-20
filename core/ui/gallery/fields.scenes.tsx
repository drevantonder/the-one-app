import { createSignal } from "solid-js"
import { Checkbox, Field, Input, RadioGroup, type Scenes, Search, Select, styles, Switch, Text, Textarea, tokens, View } from "ui"

const look = styles({
  form: { gap: tokens.grid[4] },
  top: { paddingTop: tokens.length.safeTop, paddingStart: tokens.grid[4], paddingEnd: tokens.grid[4] },
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
/// Radios, one of them off, and the field after them, with what's checked as text, so a check can say.
const Plans = () => {
  const [plan, setPlan] = createSignal<string>()
  const plans = [{ value: "free", label: "Free" }, { value: "pro", label: "Pro" }, { value: "team", label: "Team", disabled: true }, { value: "enterprise", label: "Enterprise" }]
  return (
    <View style={look.form}>
      <Field label="Plan">
        <RadioGroup value={plan()} options={plans} onValueChange={setPlan} />
      </Field>
      <Field label="Email">
        <Input />
      </Field>
      <Text>{`Plan: ${plan() ?? "none"}`}</Text>
    </View>
  )
}
const radio = (name: string) => ({ role: "radio", name })

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
/// A search at the top of a phone's screen, under its status bar, as the App Library's is, with words in it.
const Top = () => (
  <View style={look.top}>
    <Search value="Telemetry tools" aria-label="Search" />
  </View>
)
const search = { role: "searchbox", name: "Search" }
/// What a phone's system offers for a word selected, and at a caret.
const selected = { items: ["cut", "copy", "paste", "selectAll"] }

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
  // What people expect of radios, from the UI SDK's own tests (tests/specs/radio.spec.ts at the commit field.ts names).
  "Fields/Radios and what's checked": {
    render: () => <Plans />,
    checks: {
      "A press checks it": [radio("Pro"), { expect: { text: "Plan: pro" } }],
      "A press on one that's off does nothing": [radio("Team"), { expect: { text: "Plan: none" } }],
      "Tab comes to the first, checking nothing": [{ key: "Tab" }, { expect: { focused: radio("Free") } }, { expect: { text: "Plan: none" } }],
      "They're one Tab stop, which comes back to the one checked": [radio("Enterprise"), { key: "Tab" }, { expect: { focused: { role: "textbox", name: "Email" } } }, { key: "Tab", shift: true }, { expect: { focused: radio("Enterprise") } }],
      "The arrows move and check, past what's off, round the ends": [{ key: "Tab" }, { key: "ArrowDown" }, { expect: { focused: radio("Pro") } }, { expect: { text: "Plan: pro" } }, { key: "ArrowRight" }, { expect: { focused: radio("Enterprise") } }, { key: "ArrowDown" }, { expect: { focused: radio("Free") } }, { expect: { text: "Plan: free" } }, { key: "ArrowUp" }, { expect: { text: "Plan: enterprise" } }],
      "Space checks, Enter doesn't": [{ key: "Tab" }, { key: "Enter" }, { expect: { text: "Plan: none" } }, { key: " " }, { expect: { text: "Plan: free" } }],
    },
  },
  "Fields/Every kind": () => <Fields />,
  // A finger selects in a field as in a phone's own: held on a word, or tapping it twice, it selects it, with handles
  // at its ends and the system's menu over it, or under it where the status bar leaves no room; held, the system's
  // magnifier shows; a tap on the caret shows Paste.
  "Fields/Selected by a finger": {
    render: () => <Top />,
    touch: true,
    safeArea: [62, 0, 34, 0],
    // Each failed once, the menu looked for before the second tap's lift, which waited for the page, had come.
    seeds: [6, 9],
    checks: {
      "Held, a word is selected, magnified, and the menu shows as it lifts, under the status bar": [
        { down: search },
        { wait: 600 },
        { expect: { magnifier: true, handles: 2, textMenu: false } },
        { up: true },
        { expect: { textMenu: selected, handles: 2, magnifier: false } },
      ],
      "Two taps select a word, with its handles and the menu": [{ press: search, clicks: 2 }, { expect: { textMenu: selected, handles: 2 } }],
      "A tap on the caret shows Paste": [search, { wait: 500 }, search, { expect: { textMenu: { items: ["paste", "select", "selectAll"] }, handles: 0 } }],
    },
  },
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
