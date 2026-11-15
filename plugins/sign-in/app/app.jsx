// Devices: everything that acts as whoever's looking, and the passkeys they sign in with. Each has one thing to
// do to it, which is to take it out.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Icon, leave, List, Problem, Row, Screen, Section, Skeleton, toast } from "ui"

const mine = createMemo(() => call("sign-in_devices").then(JSON.parse))
changed(() => refresh(mine))

const on = (added) => new Date(added).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })
const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 minutes ago".
function seen(at) {
  const minutes = Math.round((new Date(at) - Date.now()) / 60_000)
  if (minutes > -60) return `Seen ${ago.format(Math.min(minutes, -1), "minute")}`
  if (minutes > -48 * 60) return `Seen ${ago.format(Math.round(minutes / 60), "hour")}`
  return `Seen ${ago.format(Math.round(minutes / (24 * 60)), "day")}`
}
/// What a device is, and whether it's on or since when it's been in, in one grey line.
const about = {
  app: (device) => (device.seen === "on" ? "Online" : device.seen ? seen(device.seen) : `Let in ${on(device.added)}`),
  assistant: (device) => `An assistant, let in ${on(device.added)}`,
  browser: (device) => `A browser, signed in ${on(device.added)}`,
}

/// Takes something out once its person has said so, and says what the server said if it wouldn't.
async function remove(tool, input, question) {
  if (!(await confirm({ ...question, action: "Remove", destructive: true }))) return
  try {
    await call(tool, input)
  } catch (error) {
    toast(error.message)
  }
  refresh(mine)
}

const Remove = (props) => (
  <Button variant="ghost" size="icon" aria-label={`Remove ${props.name}`} onClick={props.onClick}>
    <Icon name="trash" />
  </Button>
)

render(
  () => (
    <Screen heading="Devices" back onBack={leave}>
      <Errored fallback={(error) => <Problem heading="Can't show your devices" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={3} />}>
          <Section>
            <Show when={mine().devices.length} fallback={<Empty>Nothing acts as you yet.</Empty>}>
              <List>
                <For each={mine().devices}>
                  {(device) => (
                    <Row
                      heading={device.name}
                      text={about[device.kind](device)}
                      end={
                        <Remove
                          name={device.name}
                          onClick={() =>
                            remove("sign-in_remove_device", { device: device.id }, { heading: `Remove ${device.name}?`, text: "It stops acting as you at once, and has to be let in again." })
                          }
                        />
                      }
                    />
                  )}
                </For>
              </List>
            </Show>
          </Section>
          <Section heading="Passkeys">
            <List>
              <For each={mine().passkeys}>
                {(passkey) => (
                  <Row
                    heading={passkey.name}
                    text={`Made ${on(passkey.added)}`}
                    end={
                      <Remove
                        name={passkey.name}
                        onClick={() => remove("sign-in_remove_passkey", { passkey: passkey.id }, { heading: `Remove ${passkey.name}?`, text: "You won't be able to sign in with it again." })}
                      />
                    }
                  />
                )}
              </For>
            </List>
          </Section>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
