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

export default {
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
  "Buttons/Pressed where it's drawn": { render: () => <Moved />, taps: [{ x: 210, y: 36 }] },
} satisfies Scenes
