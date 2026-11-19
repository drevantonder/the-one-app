import { Button, Empty, Problem, type Scenes, Skeleton, Status, Steps, styles, tokens, View } from "ui"

const look = styles({ column: { gap: tokens.grid[4] } })

export default {
  "States/Every kind": () => (
    <View style={look.column}>
      <Empty>Nothing here yet.</Empty>
      <Problem heading="Couldn't load" text="The server didn't answer." action={<Button variant="outline">Try Again</Button>} />
      <Problem warn heading="Needs you" text="Sign in again." />
      <Status heading="Backing up" text="12 of 40 files" />
      <Steps steps={["Starting", "Signing in", "Ready"]} at={1} />
      <Skeleton rows={2} />
    </View>
  ),
} satisfies Scenes
