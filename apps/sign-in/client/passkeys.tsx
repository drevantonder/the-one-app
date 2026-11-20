// Sign In & Security: the passkeys whoever's looking signs in with. One is removed here, but never the last, which says so before
// it's pressed. Another is made in the device's own browser, which has passkeys where an app's web view doesn't,
// by a link that works once. Each says whether it unlocks their key on a new device: one whose PRF output derives a
// key, which it says the next time it's used, gets the key encrypted to it by a device that holds it, here, once its
// person says it may. Whoever uses it has their key from then on, so a passkey nobody here allowed never does.
import { createEffect, createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { Button, call, changed, confirm, Empty, List, open, Problem, Row, Screen, Skeleton, toast, visible } from "ui"
import { on, Remove } from "./mine.tsx"

/** A passkey, as `sign-in_devices` answers it. */
interface Passkey {
  id: string
  name: string
  added: string
  prf_key: string | null
  unlocks: boolean
  code: string | null
}

const mine = createMemo(() => call("sign-in_devices").then((text): { passkeys: Passkey[] } => JSON.parse(text)))
/// Whether this device holds its person's key, which it encrypts to their passkeys.
const holds = createMemo(() => call("encryption_key").then((text): boolean => JSON.parse(text).slot != null))
const again = () => [mine, holds].forEach(refresh)
changed(again)
// A passkey made or used in the browser says its key there, which nothing here hears of: it's looked at again as
// this screen comes back.
createEffect(visible, (now, was) => void (now && was === false && again()))

/// Whether a passkey could unlock the key, and waits for its person to say it may.
const waiting = (passkey: Passkey) => !!passkey.prf_key && !passkey.unlocks

/// What a passkey does for a new device: unlocks the key there, can't, waits to be allowed, or hasn't said since it
/// was made.
const unlocks = (passkey: Passkey) =>
  passkey.unlocks
    ? "Unlocks your key on a new device"
    : passkey.prf_key === ""
      ? "Can't unlock your key on a new device"
      : passkey.prf_key
        ? "Waiting for you to allow it"
        : "Set it up to unlock your key on a new device"

/// Asks whether a new passkey may unlock the key, then encrypts the key to it from this device, which holds it.
async function allow(passkey: Passkey) {
  const when = new Date(passkey.added).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const question = {
    heading: "Let it unlock your encrypted data?",
    text: `${passkey.name}, added ${on(passkey.added)} at ${when}. The sign-in page showed a code as you set it up: allow it only if that code is ${passkey.code}. Whoever uses it can open everything encrypted for you.`,
    action: "Allow",
  }
  if (!(await confirm(question))) return
  await call("sign-in_encrypt_for_passkeys", { passkey: passkey.id, code: passkey.code }).catch(toast)
  refresh(mine)
}

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

// A button of its own for each screen that has it: one made here would be shared by all, outside any of them.
const opening = () => <Button onPress={opens("sign-in_add_passkey")}>Add Passkey</Button>

// The devices are read by `<Show>`, outside the `<Screen>`, which keeps what's thrown inside it to itself and draws nothing.
/// Sign In & Security, as Settings draws it: a section of its own, which Back leaves on a phone.
export default function SignInAndSecurity() {
  return (
  <Errored
    fallback={(error, retry) => (
      <Screen heading="Sign In & Security" back action={opening()}>
        <Problem
          heading="Can't show your passkeys"
          text={error() as Error}
          action={<Button onPress={() => (refresh(mine), retry())}>Try Again</Button>}
        />
      </Screen>
    )}
  >
    <Loading fallback={<Screen heading="Sign In & Security" back action={opening()}><Skeleton rows={2} /></Screen>}>
      <Show when={mine()}>
        {(all) => (
          <Screen heading="Sign In & Security" back action={opening()}>
            <Show when={all().passkeys.length} fallback={<Empty>No passkeys yet.</Empty>}>
            <List each={all().passkeys} keyed={(passkey) => passkey.id}>
          {(passkey) => {
            const only = () => all().passkeys.length === 1
            return (
              <Row
                heading={passkey().name}
                text={`${unlocks(passkey())} · ${only() ? "Your only one" : `Made ${on(passkey().added)}`}`}
                action={
                  passkey().prf_key === null ? (
                    <Button variant="secondary" onPress={opens("sign-in_unlock_with_passkey")}>Set Up</Button>
                  ) : waiting(passkey()) && holds() ? (
                    <Button variant="secondary" onPress={() => allow(passkey())}>Allow</Button>
                  ) : undefined
                }
                end={<Remove name={passkey().name} disabled={only()} onPress={() => remove(passkey())} />}
              />
            )
          }}
            </List>
            </Show>
          </Screen>
        )}
      </Show>
    </Loading>
  </Errored>
)
}
