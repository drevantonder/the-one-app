// Holds Your Key, on a device's page in Settings: whether that device holds its person's key, so it opens what's
// encrypted for them without asking another device. A device asks for it from itself; one that asked is let hold it
// here by a device that holds it, as `approve` does, and one that holds it is switched off, as `revoke` does.
import { createMemo, Errored, Loading, refresh, Show } from "solid-js"
import { call, changed, client, List, Problem, Row, Skeleton, Switch, toast } from "ui"

/** A device, as `encryption_key` names it. */
interface Named {
  device: string
  name: string
}

/** What `encryption_key` answers. */
interface Key {
  slot: string | null
  devices: Named[]
  asking: Named[]
}

export default function HoldsYourKey(props: { device: string }) {
  const key = createMemo(async () => {
    const { device: here } = await client()
    const [that, mine]: Key[] = await Promise.all([
      call("encryption_key", { device: props.device }).then(JSON.parse),
      call("encryption_key", { device: here }).then(JSON.parse),
    ])
    return { holds: that.devices.some((one) => one.device === props.device), asking: that.asking.find((one) => one.device === props.device), approves: mine.slot != null }
  })
  changed(() => refresh(key))
  const set = async (on: boolean) => {
    try {
      if (on) await call("encryption_approve", { device: props.device, name: key().asking!.name })
      else await call("encryption_revoke", { device: props.device })
    } catch (error) {
      toast(error as Error)
    }
    refresh(key)
  }
  const text = () =>
    key().holds || (key().asking && key().approves)
      ? "Opens what's encrypted for you without asking another device."
      : key().asking
        ? "It asked to. Let it from a device that holds your key."
        : "It asks for your key from its own Accounts."
  return (
    <Errored fallback={(error) => <Problem heading="Can't say whether it holds your key" text={error() as Error} />}>
      <Loading fallback={<Skeleton rows={1} />}>
        <Show when={key()}>
          <List>
            <Row
              heading="Holds Your Key"
              text={text()}
              end={<Switch aria-label="Holds Your Key" checked={key().holds} disabled={!key().holds && !(key().asking && key().approves)} onChange={(event) => set(event.currentTarget.checked)} />}
            />
          </List>
        </Show>
      </Loading>
    </Errored>
  )
}
