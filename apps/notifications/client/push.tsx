// Where this phone or browser is with being woken by its notifications, which the inbox and Settings' Notifications
// both show: fine is silent, and a phone or browser that isn't woken says so, and why, with what turns them on.
import { Show } from "solid-js"
import { Button, client, List, Problem, Row, Spinner, Text, toast, View } from "ui"

/** Where this phone or browser is with being woken by its notifications, as `client()` says: nothing on a computer. */
export type Push = { state: "notAsked" | "denied" | "registering" | "registered" | "homeScreen" } | { state: "failed"; why: string }

/// Has this phone ask its person now, or try again, to be woken by its notifications, or this browser turn them on, as
/// its person taps to (a browser asks them only then), and shows where that left it.
async function tryPush(again: () => void) {
  try {
    await client({ push: true })
  } catch (error) {
    toast(error as Error)
  }
  again()
}

/// Opens this app's page in its phone's Settings, where its person turns notifications back on once they said no: a
/// phone asks only once.
async function openSettings() {
  try {
    await client({ settings: "app" })
  } catch (error) {
    toast(error as Error)
  }
}

/// Where this phone or browser is with being woken by its notifications, said only where it isn't: fine is silent,
/// and so is a computer, which shows them while the app runs. A phone asks its person by itself; a browser waits for
/// them to turn them on, and an iPhone's only from its Home Screen, as Safari has it.
export function PushState(props: { push: Push; settings: boolean; again: () => void }) {
  return (
    <>
      <Show when={props.push.state === "registering"}>
        <View role="status" style={{ flexDirection: "row", alignItems: "center", gap: 8, color: "muted-foreground" }}>
          <Spinner />
          <Text>Turning on notifications…</Text>
        </View>
      </Show>
      <Show when={props.push.state === "notAsked"}>
        <List>
          <Row heading="Notifications are off here" text="Turn them on to hear from your apps while this isn't open." action={<Button onClick={() => tryPush(props.again)}>Turn On</Button>} />
        </List>
      </Show>
      <Show when={props.push.state === "homeScreen"}>
        <List>
          <Row heading="Notifications need the Home Screen" text="In Safari, tap Share, then Add to Home Screen, and open it from there." />
        </List>
      </Show>
      <Show when={props.push.state === "denied"}>
        <Problem
          warn
          heading="Notifications are off here"
          text="Turn them on in Settings for this app or site."
          action={props.settings ? <Button variant="outline" onClick={openSettings}>Open Settings</Button> : undefined}
        />
      </Show>
      <Show when={props.push.state === "failed"}>
        <Problem warn heading="This device isn't woken by notifications" text={"why" in props.push ? props.push.why : undefined} action={<Button variant="outline" onClick={() => tryPush(props.again)}>Try Again</Button>} />
      </Show>
    </>
  )
}

