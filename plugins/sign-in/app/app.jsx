// Devices: everything that acts as whoever's looking. Each has one thing to do to it, which is to take it out,
// and this device signs out.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, client, confirm, Empty, leave, List, Problem, Row, Screen, Skeleton, toast } from "ui"
import { on, Remove } from "./mine.jsx"

const mine = createMemo(async () => {
  const [{ devices }, { device }] = await Promise.all([call("sign-in_devices").then(JSON.parse), client()])
  return { devices, here: device }
})
changed(() => refresh(mine))

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 minutes ago".
function seen(at) {
  const minutes = Math.round((new Date(at) - Date.now()) / 60_000)
  if (minutes > -60) return `Seen ${ago.format(Math.min(minutes, -1), "minute")}`
  if (minutes > -48 * 60) return `Seen ${ago.format(Math.round(minutes / 60), "hour")}`
  return `Seen ${ago.format(Math.round(minutes / (24 * 60)), "day")}`
}
/// What a device is, and whether it's on or since when it's been in, in one grey line. This device says only
/// that it's this one.
const about = {
  app: (device) => (device.seen === "on" ? "Online" : device.seen ? seen(device.seen) : `Let in ${on(device.added)}`),
  assistant: (device) => `An assistant, let in ${on(device.added)}`,
  browser: (device) => `A browser, signed in ${on(device.added)}`,
}

/// Takes a device out once its person has said so. Taking this one out is signing out, and the client starts
/// again where a device that isn't in does.
async function remove(device, here) {
  const question = here
    ? { heading: "Sign out of this device?", text: "It stops acting as you, and has to be let in again.", action: "Sign Out" }
    : { heading: `Remove ${device.name}?`, text: "It stops acting as you at once, and has to be let in again.", action: "Remove" }
  if (!(await confirm({ ...question, destructive: true }))) return
  try {
    await call("sign-in_remove_device", { device: device.id })
    if (here) return client({ leave: true })
  } catch (error) {
    toast(error.message)
  }
  refresh(mine)
}

render(
  () => (
    <Screen heading="Devices" back onBack={leave}>
      <Errored fallback={(error) => <Problem heading="Can't show your devices" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={3} />}>
          <Show when={mine().devices.length} fallback={<Empty>Nothing acts as you yet.</Empty>}>
            <List>
              <For each={mine().devices} keyed={(device) => device.id}>
                {(device) => (
                  <Row
                    heading={device().name}
                    text={device().id === mine().here ? "This device" : about[device().kind](device())}
                    end={
                      <Show when={device().id === mine().here} fallback={<Remove name={device().name} onClick={() => remove(device(), false)} />}>
                        <Button variant="outline" onClick={() => remove(device(), true)}>Sign Out</Button>
                      </Show>
                    }
                  />
                )}
              </For>
            </List>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
