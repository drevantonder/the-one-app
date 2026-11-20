import { createSignal } from "solid-js"
import { Button, List, Row, Screen, Sheet, Split, styles, Text, tokens, View, type Scenes } from "ui"

const look = styles({
  card: { padding: tokens.grid[4], borderRadius: tokens.radius.large, backgroundColor: tokens.colour.card, boxShadow: tokens.shadow.lifted, gap: tokens.grid[1] },
  page: { padding: tokens.grid[4], gap: tokens.grid[4] },
  phone: { height: 844 },
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

/// A list whose rows say which was chosen, as a finger takes them.
const Chosen = () => {
  const [chosen, setChosen] = createSignal("none")
  return (
    <Screen heading={`Chosen: ${chosen()}`}>
      <List each={notes.slice(0, 40)}>{(note) => <Row heading={note()} onPress={() => setChosen(note())} />}</List>
    </Screen>
  )
}

/// A sheet of notes, which a finger pulls down from its list scrolled to its top to close.
const Pulled = () => {
  const [open, setOpen] = createSignal(true)
  return (
    <View>
      <Button onPress={() => setOpen(true)}>Show Notes</Button>
      <Sheet heading="Notes" open={open()} onClose={() => setOpen(false)}>
        <List each={notes.slice(0, 40)}>{(note) => <Row heading={note()} />}</List>
      </Sheet>
    </View>
  )
}

/// A list whose row opens its page over it, as a phone's does, which a finger in from the left edge goes back from.
const Pushed = () => {
  const [open, setOpen] = createSignal(false)
  return (
    <View style={look.phone}>
      <Split open={open()} onClose={() => setOpen(false)} detail={<Screen heading="Alex's Laptop" back><Text>Its page.</Text></Screen>}>
        <Screen heading="Devices">
          <List>
            <Row heading="Alex's Laptop" onPress={() => setOpen(true)} />
            <Row heading="Sam's Phone" onPress={() => setOpen(true)} />
          </List>
        </Screen>
      </Split>
    </View>
  )
}

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
  // A finger as a phone's screen gives it, which every phone's host takes alike (the renderer's `view::touch`).
  "Motion/Rows under a finger": {
    touch: true,
    render: () => <Chosen />,
    checks: {
      "A tap presses a row": ["Note 3", { expect: { text: "Chosen: Note 3" } }],
      "A finger that pans the list presses nothing": [{ trace: "traces/finger-pans-up.json" }, { expect: { text: "Chosen: none" } }],
    },
  },
  "Motion/A sheet's list under a finger": {
    touch: true,
    render: () => <Pulled />,
    checks: {
      "Pulled down from its list's top, the sheet takes the finger and closes": [{ wait: 400 }, { trace: "traces/finger-pulls-down.json" }, { expect: { gone: { role: "dialog" } } }],
    },
  },
  "Motion/A page pushed, under a finger": {
    touch: true,
    render: () => <Pushed />,
    checks: {
      "In from the left edge, it goes back a page": ["Alex's Laptop", { wait: 400 }, { expect: { text: "Its page." } }, { trace: "traces/finger-from-left-edge.json" }, { expect: { gone: { text: "Its page." } } }],
    },
  },
} satisfies Scenes
