import { Button, LiveScreen, type Scenes, type ScreenInputs, Steps, styles, tokens, View } from "ui"

const look = styles({ column: { gap: tokens.grid[4] } })

// A screen's picture, as Computer Use's `screenshot` answers it: a window on a desktop.
const picture = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAKAAAABkCAIAAACO1KzYAAABAUlEQVR42u3ZoQ2AMBRF0Q5WgewMTMKIJLhqDAmODRoEA6AwpZST3An+ce+HOCR1XHACwAIswAIswAIswIAFWIDVAPC2H9Uq5VTlAAMGDBgwYMCABViABRgwYMCAAQMGrH8Bj9Oqe4ABAwYMGDBgwIABAwYswIAtWQIMGDBgwIABAxZgARZgwIABAwYMGLAAqy/gecl6GmDAgAEDBgwYMGDAAqzPAsuSJcACDBgwYMCAAQOWf7AsWYABAwYMGDBgwIABA5Z/sCxZAgwYMGDAgAEDFmD5Bxs6AAMGDBgwYMCAAQMWYMCGDkMHYMCAAQMGLMACLMAydBg6AAMGDBjwm8AXsFsEUUQssX8AAAAASUVORK5CYII="

/** A screen with no live video, as a browser without WebRTC or a screen without a stream has: its pictures. */
const pictures = async ({ action }: ScreenInputs) => {
  if (action === "stream") throw new Error("This screen has no live video")
  return action === "screenshot" ? picture : { done: action }
}

/** A screen that answers nothing but its resizing. */
const gone = async ({ action }: ScreenInputs) => {
  if (action === "resize") return { done: action }
  throw new Error("The desktop isn't running")
}

// A press of Wait a Second moves a native host's clock on, which stands still while a scene is drawn, as the DOM's
// clock goes on by itself: its picture comes, and a screen that gives none is out of reach after a few seconds.
export default {
  "Live screen/Pictures": {
    taps: ["Wait a Second"],
    errors: ["Live video didn't connect"],
    render: () => (
      <View style={look.column}>
        <LiveScreen label="The desktop" screen={pictures} />
        <Button variant="outline">Wait a Second</Button>
      </View>
    ),
  },
  "Live screen/Starting": () => (
    <LiveScreen label="The browser">
      <Steps steps={["Getting a browser ready", "Opening Work"]} at={0} />
    </LiveScreen>
  ),
  "Live screen/Can't reach it": {
    errors: ["Live video didn't connect", "The screen gave no picture either"],
    taps: Array.from({ length: 6 }, () => "Wait a Second"),
    render: () => (
      <View style={look.column}>
        <LiveScreen label="The desktop" screen={gone} />
        <Button variant="outline">Wait a Second</Button>
      </View>
    ),
  },
} satisfies Scenes
