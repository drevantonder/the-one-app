import { createSignal } from "solid-js"
import { Badge, Button, Icon, type Scenes, Spinner, styles, Text, tokens, View } from "ui"

const look = styles({
  column: { gap: tokens.grid[2], alignItems: "flex-start" },
  moved: (translateX: number) => ({ translateX }),
  wrap: { flexDirection: "row", flexWrap: "wrap", gap: tokens.grid[2] },
  row: { flexDirection: "row", gap: tokens.grid[2], alignItems: "center" },
})

/// A button moved from where it's laid out, pressed where it's drawn: a press goes to what's on screen, as GPUI's
/// hitboxes are from what was painted, not to where its layout is.
const Moved = () => {
  const [pressed, setPressed] = createSignal(0)
  return (
    <View style={look.column}>
      <View style={look.moved(160)}>
        <Button onPress={() => setPressed(pressed() + 1)}>Moved</Button>
      </View>
      <Text>{`Pressed ${pressed()} times`}</Text>
    </View>
  )
}

/// Buttons, and how often each was pressed, as text.
const Counted = () => {
  const [count, setCount] = createSignal(0)
  return (
    <View style={look.column}>
      <Button onPress={() => setCount(count() + 1)}>Save</Button>
      <Button disabled onPress={() => setCount(count() + 100)}>Delete</Button>
      <Text>{`Pressed ${count()} times`}</Text>
    </View>
  )
}

export default {
  // What people expect of a button, from the UI SDK's own tests (tests/specs/button.spec.ts at the commit button.ts
  // names).
  "Buttons/Pressed": {
    render: () => <Counted />,
    checks: {
      "A press presses it": ["Save", { expect: { text: "Pressed 1 times" } }],
      "Enter and Space press it": [{ key: "Tab" }, { key: "Enter" }, { key: " " }, { expect: { text: "Pressed 2 times" } }],
      "One that's off does nothing": [{ role: "button", name: "Delete" }, { expect: { text: "Pressed 0 times" } }],
      // Its pressed look is drawn on the first frame of the press, as a Mac's buttons' is, not faded in.
      "Held down, it looks pressed at once": [{ down: "Save" }, { expect: { drawn: { of: { role: "button", name: "Save" }, opacity: 0.7 } }, within: 17 }, { up: true }, { expect: { text: "Pressed 1 times" } }],
    },
  },
  "Buttons/Variants": () => (
    <View style={look.wrap}>
      <Button>Default</Button>
      <Button variant="secondary">Secondary</Button>
      <Button variant="outline">Outline</Button>
      <Button variant="ghost">Ghost</Button>
      <Button variant="destructive">Delete</Button>
      <Button loading>Saving</Button>
      <Button disabled>Disabled</Button>
      <Button variant="outline" size="icon" aria-label="More">
        <Icon name="dots-three" />
      </Button>
    </View>
  ),
  "Buttons/Badges and spinner": () => (
    <View style={look.row}>
      <Badge>3</Badge>
      <Badge variant="secondary">New</Badge>
      <Badge variant="destructive">12</Badge>
      <Badge variant="outline">Beta</Badge>
      <Spinner />
    </View>
  ),
  "Buttons/Pressed where it's drawn": { render: () => <Moved />, taps: [{ role: "button", name: "Moved" }] },
} satisfies Scenes
