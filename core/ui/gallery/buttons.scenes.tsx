import { Badge, Box, Button, Icon, Spinner, type Scenes } from "ui"

export default {
  "Buttons/Variants": () => (
    <Box style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
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
    </Box>
  ),
  "Buttons/Badges and spinner": () => (
    <Box style={{ flexDirection: "row", gap: 8, alignItems: "center" }}>
      <Badge>3</Badge>
      <Badge variant="secondary">New</Badge>
      <Badge variant="destructive">12</Badge>
      <Badge variant="outline">Beta</Badge>
      <Spinner />
    </Box>
  ),
} satisfies Scenes
