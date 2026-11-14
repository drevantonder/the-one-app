// Devices: everything that acts as whoever's looking, and the passkeys they sign in with. Each has one thing to
// do to it, which is to take it out.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, Icon, leave, List, Problem, Row, Screen, Section, Skeleton, toast } from "ui"

const mine = createMemo(() => call("sign-in_devices").then(JSON.parse))
changed(() => refresh(mine))

const on = (added) => new Date(added).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })
/// What a device is, and since when, in one grey line.
const about = {
  app: (added) => `Let in ${on(added)}`,
  assistant: (added) => `An assistant, let in ${on(added)}`,
  browser: (added) => `A browser, signed in ${on(added)}`,
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
                      text={about[device.kind](device.added)}
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
