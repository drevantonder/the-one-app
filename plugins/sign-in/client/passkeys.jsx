// Passkeys: what whoever's looking signs in with. One is removed here, but never the last, which says so before
// it's pressed. Another is made in the device's own browser, which has passkeys where an app's web view doesn't,
// by a link that works once.
import { createMemo, Errored, For, Loading, refresh } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, leave, List, open, Problem, Row, Screen, Skeleton, toast } from "ui"
import { on, Remove } from "./mine.jsx"

const mine = createMemo(() => call("sign-in_devices").then(JSON.parse))
changed(() => refresh(mine))

async function remove(passkey) {
  if (!(await confirm({ heading: `Remove ${passkey.name}?`, text: "You won't be able to sign in with it again.", action: "Remove", destructive: true }))) return
  await call("sign-in_remove_passkey", { passkey: passkey.id }).catch((error) => toast(error.message))
  refresh(mine)
}

async function add() {
  try {
    await open(await call("sign-in_add_passkey"))
  } catch (error) {
    toast(error.message)
  }
}

render(
  () => (
    <Screen heading="Passkeys" back onBack={leave} action={<Button onClick={add}>Add Passkey</Button>}>
      <Errored fallback={(error) => <Problem heading="Can't show your passkeys" text={String(error().message ?? error())} />}>
        <Loading fallback={<Skeleton rows={2} />}>
          <List>
            <For each={mine().passkeys} keyed={(passkey) => passkey.id}>
              {(passkey) => {
                const only = () => mine().passkeys.length === 1
                return <Row heading={passkey().name} text={only() ? "Your only one. Add another to remove it." : `Made ${on(passkey().added)}`} end={<Remove name={passkey().name} disabled={only()} onClick={() => remove(passkey())} />} />
              }}
            </For>
          </List>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
