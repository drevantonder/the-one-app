// Containers: yours on every machine, each with what it's for and whether it's running, to stop or remove.
import { createMemo, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, leave, List, Menu, Problem, Row, Screen, Skeleton, toast } from "ui"

/** A container, as `containers_list` answers it. */
interface Listed {
  name: string
  description: string
  state: string
  machine: string
  last_used: number
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 hours ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -60) return ago.format(Math.min(minutes, -1), "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const STATE: Record<string, string> = { running: "Running", exited: "Stopped", created: "Created", paused: "Paused" }

function Container(props: { container: Listed }) {
  const c = () => props.container
  const act = async ({ value }: { value: string }) => {
    if (value === "remove") {
      const yes = await confirm({ heading: `Remove ${c().name}?`, text: "Its files go with it, for good.", action: "Remove", destructive: true })
      if (!yes) return
    }
    await call(`containers_${value}`, { container: c().name }).catch((error) => toast(error.message))
  }
  return (
    <Row
      heading={c().name}
      text={`${c().description} · ${STATE[c().state] ?? c().state} · ${c().machine} · used ${since(c().last_used)}`}
      end={
        <Menu.Root onSelect={act}>
          <Menu.Trigger aria-label={`More for ${c().name}`} />
          <Menu.Positioner>
            <Menu.Content>
              <Show when={c().state === "running"} fallback={<Menu.Item value="start">Start</Menu.Item>}>
                <Menu.Item value="stop">Stop</Menu.Item>
              </Show>
              <Menu.Separator />
              <Menu.Item value="remove" destructive>Remove</Menu.Item>
            </Menu.Content>
          </Menu.Positioner>
        </Menu.Root>
      }
    />
  )
}

function App() {
  const containers = createMemo(() => call("containers_list").then((text): Listed[] => JSON.parse(text)))
  changed(() => refresh(containers))
  return (
    <Screen heading="Containers" back onBack={leave}>
      <Errored fallback={(error, reset) => <Problem heading="Couldn't list your containers" text={(error() as Error).message} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
        <Loading fallback={<Skeleton rows={3} />}>
          <Show when={containers().length} fallback={<Empty>No containers. Your agents create them as they need them.</Empty>}>
            <List>
              <For each={containers()} keyed={(c) => `${c.machine}/${c.name}`}>
                {(c) => <Container container={c()} />}
              </For>
            </List>
          </Show>
        </Loading>
      </Errored>
    </Screen>
  )
}

render(() => <App />, document.body)
