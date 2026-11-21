import { createSignal } from "solid-js"
import { Button, type Scenes, ScrollView, styles, Text, tokens, View } from "ui"

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
  stack: { gap: 8, padding: 24 },
  row: { height: 100, justifyContent: "center", paddingStart: 60, alignItems: "flex-start" },
  moved: { translateX: 90, translateY: 6 },
  grown: { scale: 2, color: tokens.colour.red },
  turned: { rotate: 90, color: tokens.colour.blue },
  inside: { scale: 1.5, translateX: 40 },
  nested: { scale: 0.8, translateX: 30 },
  centred: { width: 240, textAlign: "center", scale: 1.3, translateX: 20 },
  half: { translateX: "100%", scale: 0.9 },
  scroller: { height: 60, width: 200, scale: 1.4, translateX: 30 },
  spins: (turn: number) => ({ rotate: turn, transitionProperty: ["rotate"], transition: tokens.motion.smooth }),
  flies: (x: number, y: number, scale: number) => ({ translateX: x, translateY: y, scale, transitionProperty: ["translateX", "translateY", "scale"], transition: tokens.motion.smooth }),
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

/// Texts drawn moved, grown and turned, by themselves and inside a view that is, which the inspector finds where they're drawn.
const Transformed = () => (
  <View style={look.stack}>
    <View style={look.row}><Text>Plain text</Text></View>
    <View style={look.row}><Text style={look.moved}>Moved text</Text></View>
    <View style={look.row}><View style={look.inside}><Text>Inside a view</Text></View></View>
    <View style={look.row}><Text style={look.grown}>Grown</Text></View>
    <View style={look.row}><Text style={look.turned}>Turned</Text></View>
    <View style={look.row}><View style={look.turned}><Text>View turned</Text></View></View>
    <View style={look.row}><View style={look.inside}><View style={look.nested}><Text>Nested</Text></View></View></View>
    <View style={look.row}><Text style={look.centred}>Centred</Text></View>
    <View style={look.row}><Text style={look.half}>Half over</Text></View>
    <View style={look.row}><ScrollView style={look.scroller}><View style={look.column}><Text>First line</Text><Text>Second line</Text><Text>Third line</Text></View></ScrollView></View>
    <View style={look.row}><View style={look.inside}><Text>Some <Text style={look.strong}>words</Text> in spans</Text></View></View>
  </View>
)

/// Texts that fly when Go is pressed, by themselves and inside a view that does.
const Flying = () => {
  const [away, setAway] = createSignal(false)
  return (
    <View style={look.stack}>
      <Button onPress={() => setAway(!away())}>Go</Button>
      <View style={look.row}><Text style={away() ? look.flies(120, 30, 1.6) : look.flies(0, 0, 1)}>Flies</Text></View>
      <View style={look.row}><View style={away() ? look.flies(120, 30, 1.6) : look.flies(0, 0, 1)}><Text>Rides</Text></View></View>
      <View style={look.row}><Text style={look.spins(away() ? 90 : 0)}>Spins</Text></View>
      <View style={look.row}><View style={look.spins(away() ? 90 : 0)}><Text>Turns</Text></View></View>
    </View>
  )
}

export default {
  "Text/Transformed": {
    render: () => <Transformed />,
    size: [390, 1400],
    checks: {
      "Every text's rect is where its glyphs are drawn": [{ expect: { located: ["Plain text", "Moved text", "Inside a view", "Grown", "Turned", "View turned", "Nested", "Centred", "Some words in spans", "Half over", "First line", "Second line"] } }],
    },
  },
  "Text/Flying": {
    render: () => <Flying />,
    size: [390, 700],
    motions: {
      "A text flies": { from: { role: "button", name: "Go" }, gesture: { moves: [{ ms: 90 }] }, follow: "Flies", located: true },
      "A text rides a view that flies": { from: { role: "button", name: "Go" }, gesture: { moves: [{ ms: 90 }] }, follow: "Rides", located: true },
      "A text spins": { from: { role: "button", name: "Go" }, gesture: { moves: [{ ms: 90 }] }, follow: "Spins", located: true },
      "A text rides a view that spins": { from: { role: "button", name: "Go" }, gesture: { moves: [{ ms: 90 }] }, follow: "Turns", located: true },
    },
  },
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
