import { Button, Empty, Problem, type Scenes, Skeleton, Status, Steps, View } from "ui"

export default {
  "States/Every kind": () => (
    <View style={{ gap: 16 }}>
      <Empty>Nothing here yet.</Empty>
      <Problem heading="Couldn't load" text="The server didn't answer." action={<Button variant="outline">Try Again</Button>} />
      <Problem warn heading="Needs you" text="Sign in again." />
      <Status heading="Backing up" text="12 of 40 files" />
      <Steps steps={["Starting", "Signing in", "Ready"]} at={1} />
      <Skeleton rows={2} />
    </View>
  ),
} satisfies Scenes
