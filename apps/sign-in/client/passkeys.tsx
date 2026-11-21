// Sign In & Security: the passkeys whoever's looking signs in with. One is removed here, but never the last, which says so before
// it's pressed. Another is made in the device's own browser, which has passkeys where an app's web view doesn't,
// by a link that works once. Each says whether it unlocks their key on a new device: one whose PRF output derives a
// key, which it says the next time it's used, gets the key encrypted to it by a device that holds it, here, once its
// person says it may. Whoever uses it has their key from then on, so a passkey nobody here allowed never does. Above
// them are the servers waiting for a key that a passkey hands over, their person's own or a group's or the
// network's they own, each with Use Passkey, which opens the sign-in page to do it (`unlock_with_passkey`).
import { createEffect, createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { Button, call, changed, confirm, date, Empty, List, open, Problem, Row, Screen, Section, Skeleton, toast, visible } from "ui"
import { Remove } from "./mine.tsx"

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
/** What `encryption_key` says of a key: whose it is, a group's or the network's encrypted to the person's own, and the servers that should hold the keys of what it encrypts and don't. */
interface Held {
  owner: string
  wrapped: string | null
  access: { missing: { key: string; name: string | null }[] }[]
}

/// The servers waiting for a key a passkey of this person's hands them: their own, and those of the network and the
/// groups they own, whose keys are encrypted to theirs. One row each, by whose key it is.
const waitingFor = createMemo(async () => {
  const me: Held = JSON.parse(await call("encryption_key"))
  const groups: { group: string; owners: string[] }[] = JSON.parse(await call("permissions_groups")).groups
  const scopes = ["network", ...groups.filter((group) => group.owners.includes(me.owner)).map((group) => group.group)]
  const theirs = await Promise.all(scopes.map((subject) => call("encryption_key", {}, { subject }).then((text): Held => JSON.parse(text), () => null)))
  return [me, ...theirs.filter((held): held is Held => !!held?.wrapped)]
    .map((held) => ({ scope: held.owner === me.owner ? undefined : held.owner, servers: [...new Set(held.access.flatMap((artifact) => artifact.missing.map((server) => server.name ?? server.key.slice(0, 8))))] }))
    .filter((waiting) => waiting.servers.length)
})
const again = () => [mine, holds, waitingFor].forEach(refresh)
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
    text: `${passkey.name}, added ${date(passkey.added)} at ${when}. The sign-in page showed a code as you set it up: allow it only if that code is ${passkey.code}. Whoever uses it can open everything encrypted for you.`,
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

/// Whose key servers wait for, as a row says it.
const whose = (scope?: string) => (scope === undefined ? "your" : scope === "network" ? "the network's" : `${scope.split("/").pop()}'s`)
const listed = (names: string[]) => (names.length < 3 ? names.join(" and ") : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`)

/// The servers waiting for a key, in one row whose passkey hands each its keys at once (the sign-in page's Unlock with
/// Passkey, with no `scope`, hands over every key the person's opens).
function Waiting() {
  const servers = () => [...new Set(waitingFor()?.flatMap((waiting) => waiting.servers))]
  const keys = () => {
    const named = waitingFor()?.map((waiting) => whose(waiting.scope)) ?? []
    return named.length === 1 ? `${named[0]} key` : `${listed(named)} keys`
  }
  return (
    <Show when={waitingFor()?.length}>
      <Section heading="Waiting for Your Passkey">
        <List>
          <Row
            heading={`${listed(servers())} ${servers().length === 1 ? "is" : "are"} waiting for ${keys()}`}
            text={`Your passkey hands ${waitingFor()!.length === 1 ? "it" : "them"} over.`}
            action={<Button variant="secondary" onPress={opens("sign-in_unlock_with_passkey")}>Use Passkey</Button>}
          />
        </List>
      </Section>
    </Show>
  )
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
            <Waiting />
            <Show when={all().passkeys.length} fallback={<Empty>No passkeys yet.</Empty>}>
            <List each={all().passkeys} keyed={(passkey) => passkey.id}>
          {(passkey) => {
            const only = () => all().passkeys.length === 1
            // What it can do now, under its words on a phone; the trash after it there, and beside the words where it has none.
            const set = () => passkey().prf_key === null
            const allows = () => !set() && waiting(passkey()) && holds()
            const trash = () => <Remove name={passkey().name} disabled={only()} onPress={() => remove(passkey())} />
            return (
              <Row
                heading={passkey().name}
                text={`${unlocks(passkey())} · ${only() ? "Your only one" : `Made ${date(passkey().added)}`}`}
                end={set() || allows() ? undefined : trash()}
                action={
                  set() || allows() ? (
                    <>
                      {set() ? (
                        <Button variant="secondary" onPress={opens("sign-in_unlock_with_passkey")}>Set Up</Button>
                      ) : (
                        <Button variant="secondary" onPress={() => allow(passkey())}>Allow</Button>
                      )}
                      {trash()}
                    </>
                  ) : undefined
                }
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
