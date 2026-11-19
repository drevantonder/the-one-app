import { Badge, Button, Icon, type Scenes, Spinner, View } from "ui"

export default {
  "Buttons/Variants": () => (
    <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
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
    <View style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
      <Badge>3</Badge>
      <Badge variant="secondary">New</Badge>
      <Badge variant="destructive">12</Badge>
      <Badge variant="outline">Beta</Badge>
      <Spinner />
    </View>
  ),
} satisfies Scenes
