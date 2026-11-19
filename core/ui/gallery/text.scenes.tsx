import { createSignal } from "solid-js"
import { type Scenes, styles, Text, tokens, View } from "ui"

const look = styles({
  column: { gap: tokens.grid[2] },
  largeTitle: { font: tokens.type.largeTitle },
  title: { font: tokens.type.title },
  figure: { font: tokens.type.figure },
  barTitle: { font: tokens.type.barTitle },
  large: { font: tokens.type.large },
  small: { font: tokens.type.small },
  caption: { font: tokens.type.caption },
  strong: { fontWeight: tokens.weight.medium },
  quiet: { color: tokens.colour.secondaryLabel },
  wrong: { color: tokens.colour.red },
  accent: { color: tokens.colour.blue },
  highlight: { backgroundColor: tokens.colour.quinaryLabel },
  width: (width: number) => ({ width }),
})

/// A link in a sentence's words, which says how often it's been pressed.
function InlineLink() {
  const [pressed, setPressed] = createSignal(0)
  return (
    <View style={look.column}>
      <Text>
        Read the{" "}
        <Text role="link" tabIndex={0} onPress={() => setPressed(pressed() + 1)} style={[look.accent, look.strong]}>
          terms of use
        </Text>{" "}
        before you go on.
      </Text>
      <Text style={[look.caption, look.quiet]}>Pressed {pressed()} times</Text>
    </View>
  )
}

export default {
  "Text/Type styles": () => (
    <View style={look.column}>
      <Text style={look.largeTitle}>Large Title</Text>
      <Text style={look.title}>Title</Text>
      <Text style={look.figure}>1,234</Text>
      <Text style={look.barTitle}>Bar Title</Text>
      <Text style={look.large}>Large</Text>
      <Text>Medium, the body's, which runs on over more than one line where there isn't room for all of it in one.</Text>
      <Text style={look.strong}>Medium, its weight medium</Text>
      <Text style={[look.caption, look.quiet]}>Caption, quiet</Text>
      <Text style={look.small}>Small</Text>
    </View>
  ),
  "Text/Styled spans": () => (
    <View style={look.column}>
      <Text style={look.quiet}>
        12 calls · <Text style={look.wrong}>3 failed</Text>
      </Text>
      <Text>
        Body, <Text style={look.strong}>medium weight</Text> and <Text style={[look.caption, look.quiet]}>a caption</Text> beside{" "}
        <Text style={look.highlight}>a highlight</Text> here
      </Text>
      <Text style={look.title}>
        Title with <Text style={look.accent}>a coloured word</Text>
      </Text>
    </View>
  ),
  "Text/Inline link": () => <InlineLink />,
  "Text/Inline link, pressed": { render: () => <InlineLink />, taps: ["terms of use"] },
  "Text/Runs wrapping": () => (
    <View style={look.width(220)}>
      <Text>
        A sentence that wraps, with <Text style={look.wrong}>a red stretch that runs on over the end of a line</Text> and{" "}
        <Text style={[look.strong, look.highlight]}>a highlighted bold one that wraps too</Text> then plain words to its end
      </Text>
    </View>
  ),
} satisfies Scenes
