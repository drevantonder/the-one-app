// General, as Settings draws it: what the app does on this computer by itself, which a phone has none of.
import { createSignal, Show } from "solid-js"
import { client, List, Row, Screen, Section, Switch, toast } from "ui"

const [login, setLogin] = createSignal<boolean | null>()
client().then((now) => setLogin(now.login ?? null))
const choose = (on: boolean) =>
  client({ login: on }).then(
    () => setLogin(on),
    (error: Error) => toast(error.message),
  )

export default function General() {
  return (
    <Screen heading="General" back>
      <Show when={login() !== null && login() !== undefined}>
        <Section>
          <List>
            <Row heading="Open at Login" end={<Switch aria-label="Open at Login" value={login() === true} onValueChange={choose} />} />
          </List>
        </Section>
      </Show>
    </Screen>
  )
}
