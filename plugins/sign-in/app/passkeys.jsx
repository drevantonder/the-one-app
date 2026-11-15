// Passkeys: what whoever's looking signs in with. One is removed here, but never the last.
import { createMemo, Errored, For, Loading, refresh } from "solid-js"
import { render } from "@solidjs/web"
import { call, changed, confirm, leave, List, Problem, Row, Screen, Skeleton, toast } from "ui"
import { on, Remove } from "./mine.jsx"

const mine = createMemo(() => call("sign-in_devices").then(JSON.parse))
changed(() => refresh(mine))

async function remove(passkey) {
  if (!(await confirm({ heading: `Remove ${passkey.name}?`, text: "You won't be able to sign in with it again.", action: "Remove", destructive: true }))) return
  await call("sign-in_remove_passkey", { passkey: passkey.id }).catch((error) => toast(error.message))
  refresh(mine)
}

render(
  () => (
    <Screen heading="Passkeys" back onBack={leave}>
      <Errored fallback={(error) => <Problem heading="Can't show your passkeys" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={2} />}>
          <List>
            <For each={mine().passkeys} keyed={(passkey) => passkey.id}>
              {(passkey) => <Row heading={passkey().name} text={`Made ${on(passkey().added)}`} end={<Remove name={passkey().name} onClick={() => remove(passkey())} />} />}
            </For>
          </List>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
