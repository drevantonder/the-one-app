import { Box, Text, type Scenes } from "ui"

export default {
  "Text/Type styles": () => (
    <Box style={{ gap: 8 }}>
      <Text style={{ font: "title" }}>Title</Text>
      <Text style={{ font: "figure" }}>1,234</Text>
      <Text style={{ font: "label" }}>Label</Text>
      <Text>Body text, which runs on over more than one line where there isn't room for all of it in one.</Text>
      <Text style={{ font: "subheadline" }}>Subheadline</Text>
      <Text style={{ font: "footnote", color: "muted-foreground" }}>Footnote, muted</Text>
      <Text style={{ font: "caption-1" }}>Caption 1</Text>
      <Text style={{ font: "caption-2" }}>Caption 2</Text>
    </Box>
  ),
} satisfies Scenes
