// Sign-In & Security: the passkeys whoever's looking signs in with. One is removed here, but never the last, which says so before
// it's pressed. Another is made in the device's own browser, which has passkeys where an app's web view doesn't,
// by a link that works once. Each says whether it unlocks their key on a new device: one whose PRF output derives a
// key, which it says the next time it's used, gets the key encrypted to it by a device that holds it, here, as this
// screen opens there or comes back to the front.
import { createEffect, createMemo, Errored, For, Loading, refresh } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, List, open, Problem, Row, Screen, Skeleton, toast } from "ui"
import { on, Remove } from "./mine.tsx"

/** A passkey, as `sign-in_devices` answers it. */
interface Passkey {
  id: string
  name: string
  added: string
  prf_key: string | null
  unlocks: boolean
}

const mine = createMemo(() => call("sign-in_devices").then((text): { passkeys: Passkey[] } => JSON.parse(text)))
/// Whether this device holds its person's key, which it encrypts to their passkeys.
const holds = createMemo(() => call("encryption_key").then((text): boolean => JSON.parse(text).slot != null))
const again = () => [mine, holds].forEach(refresh)
changed(again)
// A passkey made or used in the browser says its key there, which nothing here hears of: it's looked at again as
// this screen comes back.
addEventListener("focus", again)
document.addEventListener("visibilitychange", () => document.visibilityState === "visible" && again())

createEffect(
  () => holds() && mine().passkeys.some((passkey) => passkey.prf_key && !passkey.unlocks),
  (needed) => void (needed && call("sign-in_encrypt_for_passkeys").then(() => refresh(mine), toast)),
)

/// What a passkey does for a new device: unlocks the key there, can't, or hasn't said since it was made.
const unlocks = (passkey: Passkey) =>
  passkey.unlocks ? "Unlocks your key on a new device" : passkey.prf_key === "" ? "Can't unlock your key on a new device" : passkey.prf_key ? "Unlocks your key once a device that holds it opens this" : "Set it up to unlock your key on a new device"

async function remove(passkey: Passkey) {
  if (!(await confirm({ heading: `Remove ${passkey.name}?`, text: "You won't be able to sign in with it again, or unlock your key with it.", action: "Remove", destructive: true }))) return
  await call("sign-in_remove_passkey", { passkey: passkey.id }).catch(toast)
  refresh(mine)
}

/// Opens a link in the device's browser, from a tool that makes it.
const opens = (tool: string, inputs?: object) => async () => {
  try {
    await open(JSON.parse(await call(tool, inputs)).link)
  } catch (error) {
    toast(error as Error)
  }
}

render(
  () => (
    <Screen heading="Sign-In & Security" action={<Button onClick={opens("sign-in_add_passkey")}>Add Passkey</Button>}>
      <Errored fallback={(error) => <Problem heading="Can't show your passkeys" text={error() as Error} />}>
        <Loading fallback={<Skeleton rows={2} />}>
          <List>
            <For each={mine().passkeys} keyed={(passkey) => passkey.id}>
              {(passkey) => {
                const only = () => mine().passkeys.length === 1
                return (
                  <Row
                    heading={passkey().name}
                    text={`${unlocks(passkey())} · ${only() ? "Your only one" : `Made ${on(passkey().added)}`}`}
                    action={passkey().prf_key === null ? <Button variant="secondary" onClick={opens("sign-in_unlock_with_passkey")}>Set Up</Button> : undefined}
                    end={<Remove name={passkey().name} disabled={only()} onClick={() => remove(passkey())} />}
                  />
                )
              }}
            </For>
          </List>
        </Loading>
      </Errored>
    </Screen>
  ),
  document.body,
)
