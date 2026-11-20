// Holds Your Key, on a device's page in Settings: whether that device holds its person's key, which it's given only
// by their passkey, with Unlock with Passkey there. One that holds it is switched off here, as `revoke` does.
import { createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { call, changed, List, Problem, Row, Skeleton, Switch, toast } from "ui"

/** A device, as `encryption_key` names it. */
interface Named {
  device: string
  name: string
}

/** What `encryption_key` answers. */
interface Key {
  devices: Named[]
}

export default function HoldsYourKey(props: { device: string }) {
  const key = createMemo(async () => {
    const that: Key = JSON.parse(await call("encryption_key", { device: props.device }))
    return { holds: that.devices.some((one) => one.device === props.device) }
  })
  changed(() => refresh(key))
  const revoke = async () => {
    await call("encryption_revoke", { device: props.device }).catch(toast)
    refresh(key)
  }
  const text = () => (key().holds ? "Opens what's encrypted for you on its own." : "Your passkey lets it hold your key: Unlock with Passkey, in its Vault.")
  return (
    <Errored fallback={(error) => <Problem heading="Can't say whether it holds your key" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={1} />}>
        <Show when={key()}>
          <List>
            <Row
              heading="Holds Your Key"
              text={text()}
              end={<Switch aria-label="Holds Your Key" value={key().holds} disabled={!key().holds} onValueChange={revoke} />}
            />
          </List>
        </Show>
      </Loading>
    </Errored>
  )
}
