import { createSignal } from "solid-js"
import { Button, List, Row, styles, Text, tokens, View, type Scenes } from "ui"

const look = styles({
  card: { padding: tokens.grid[4], borderRadius: tokens.radius.large, backgroundColor: tokens.colour.card, boxShadow: tokens.shadow.lifted, gap: tokens.grid[1] },
  page: { padding: tokens.grid[4], gap: tokens.grid[4] },
  box: { width: 64, height: 64, borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.blue },
  smooth: (translateX: number) => ({ translateX, transitionProperty: ["translateX"], transition: tokens.motion.smooth }),
  snappy: (translateX: number) => ({ translateX, transitionProperty: ["translateX"], transition: tokens.motion.snappy }),
})

const Springs = () => {
  const [moved, setMoved] = createSignal(false)
  return (
    <View style={look.page}>
      <Button onPress={() => setMoved(!moved())}>Move</Button>
      <Text>Smooth</Text>
      <View aria-label="Smooth" style={[look.box, look.smooth(moved() ? 200 : 0)]} />
      <Text>Snappy</Text>
      <View aria-label="Snappy" style={[look.box, look.snappy(moved() ? 200 : 0)]} />
    </View>
  )
}

const notes = Array.from({ length: 200 }, (_, at) => `Note ${at + 1}`)

// What moves under a pointer or a finger, each with the gestures core/ui/motion.js plays on it, frame by frame.
export default {
  "Motion/Card dragged along": {
    touch: true,
    motions: {
      "Swiped and let go": { from: "Swipe me", follow: "Swipe me", gesture: { swipe: [160, 0], speed: 600 } },
      "Dragged with pauses": { from: "Swipe me", follow: "Swipe me", gesture: { drag: [200, 0], pauses: 2, speed: 400 } },
    },
    render: () => (
      <View style={look.page}>
        <View aria-label="Swipe me" drag="x" dragConstraints={{ left: 0, right: 0 }} style={look.card}>
          <Text>Swipe me</Text>
          <Text>It gives as a rubber band and springs back.</Text>
        </View>
      </View>
    ),
  },
  // A trackpad's swipe drags it as a finger does, the fingers' alone: the pointer, nudged as they go, doesn't move it.
  "Motion/Card swiped by a trackpad": {
    motions: { "Swiped, the pointer nudged": { follow: "Swipe me", gesture: { trace: "traces/trackpad-swipe-right.json" }, steady: true } },
    render: () => (
      <View style={look.page}>
        <View aria-label="Swipe me" drag="x" dragConstraints={{ left: 0, right: 0 }} style={look.card}>
          <Text>Swipe me</Text>
          <Text>It gives as a rubber band and springs back.</Text>
        </View>
      </View>
    ),
  },
  // The springs the tokens name, each moving a box 200 pt as Move is pressed: SwiftUI's `.smooth` and `.snappy`.
  "Motion/Springs": Springs,
  "Motion/Long list": {
    touch: true,
    motions: {
      Flung: { from: { x: 195, y: 600 }, gesture: { fling: [0, -300] } },
      "Pulled past its top": { from: { x: 195, y: 300 }, gesture: { swipe: [0, 240], speed: 800 } },
    },
    render: () => <List each={notes}>{(note) => <Row heading={note()} />}</List>,
  },
} satisfies Scenes
