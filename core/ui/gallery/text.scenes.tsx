import { createSignal } from "solid-js"
import { Box, Text, type Scenes } from "ui"

/// A link in a sentence's words, which says how often it's been pressed.
function InlineLink() {
  const [pressed, setPressed] = createSignal(0)
  return (
    <Box style={{ gap: 8 }}>
      <Text>
        Read the{" "}
        <Text role="link" tabindex={0} onClick={() => setPressed(pressed() + 1)} style={{ color: "primary", font: "label" }}>
          terms of use
        </Text>{" "}
        before you go on.
      </Text>
      <Text style={{ font: "footnote", color: "muted-foreground" }}>Pressed {pressed()} times</Text>
    </Box>
  )
}

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
  "Text/Styled spans": () => (
    <Box style={{ gap: 8 }}>
      <Text style={{ color: "muted-foreground" }}>
        12 calls · <Text style={{ color: "destructive" }}>3 failed</Text>
      </Text>
      <Text>
        Body, <Text style={{ font: "label" }}>label weight</Text> and <Text style={{ font: "caption-1", color: "muted-foreground" }}>a caption</Text> beside{" "}
        <Text style={{ backgroundColor: "muted" }}>a highlight</Text> here
      </Text>
      <Text style={{ font: "title" }}>
        Title with <Text style={{ color: "primary" }}>a coloured word</Text>
      </Text>
    </Box>
  ),
  "Text/Inline link": () => <InlineLink />,
  "Text/Inline link, pressed": { render: () => <InlineLink />, taps: ["terms of use"] },
  "Text/Runs wrapping": () => (
    <Box style={{ width: 220 }}>
      <Text>
        A sentence that wraps, with <Text style={{ color: "destructive" }}>a red stretch that runs on over the end of a line</Text> and{" "}
        <Text style={{ font: "label", backgroundColor: "muted" }}>a highlighted bold one that wraps too</Text> then plain words to its end
      </Text>
    </Box>
  ),
} satisfies Scenes
