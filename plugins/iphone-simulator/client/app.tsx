// iPhone Simulator: your simulators on every Mac, and one of them live, to use as an iPhone, start, stop or remove.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, leave, List, LiveScreen, Menu, Problem, Row, Screen, Skeleton, Split, toast } from "ui"

/** A simulator, as `iphone-simulator_list` answers it. */
interface Simulator {
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
const STATE: Record<string, string> = { running: "Running", starting: "Starting", stopping: "Stopping", stopped: "Stopped", gone: "Gone from Xcode" }

const [open, setOpen] = createSignal<string>()

function Detail(props: { simulator: Simulator }) {
  const e = () => props.simulator
  const [busy, setBusy] = createSignal(false)
  const act = async ({ value }: { value: string }) => {
    if (value === "remove") {
      const yes = await confirm({ heading: `Remove ${e().name}?`, text: "Its apps and everything on it go with it, for good.", action: "Remove", destructive: true })
      if (!yes) return
    }
    setBusy(true)
    await call(`iphone-simulator_${value}`, { simulator: e().name })
      .then(() => value === "remove" && setOpen())
      .catch((error) => toast(error.message))
      .finally(() => setBusy(false))
  }
  return (
    <Screen
      heading={e().name}
      back
      onBack={() => setOpen()}
      action={
        <Menu.Root onSelect={act}>
          <Menu.Trigger aria-label={`More for ${e().name}`} />
          <Menu.Positioner>
            <Menu.Content>
              <Show when={e().state === "stopped"} fallback={<Menu.Item value="stop">Stop</Menu.Item>}>
                <Menu.Item value="start">Start</Menu.Item>
              </Show>
              <Menu.Separator />
              <Menu.Item value="remove" destructive>Remove</Menu.Item>
            </Menu.Content>
          </Menu.Positioner>
        </Menu.Root>
      }
    >
      <Show
        when={e().state === "running" && e().name}
        keyed
        fallback={<Empty>{busy() || e().state === "starting" ? "Starting…" : `${STATE[e().state] ?? e().state}. Start it to use it.`}</Empty>}
      >
        {(name) => <LiveScreen label={`${name}'s screen`} screen={(inputs) => call("iphone-simulator_screen", { simulator: name, ...inputs })} />}
      </Show>
    </Screen>
  )
}

function App() {
  const simulators = createMemo(() => call("iphone-simulator_list").then((text): Simulator[] => JSON.parse(text)))
  changed(() => refresh(simulators))
  const opened = () => simulators().find((e) => e.name === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your simulators" text={(error() as Error).message} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={simulators().length}><Empty>Pick a simulator to use it.</Empty></Show>}><Detail simulator={opened()!} /></Show>}>
          <Screen heading="iPhone Simulator" back onBack={leave}>
            <Show when={simulators().length} fallback={<Empty>No simulators. Your agents create them as they need them.</Empty>}>
              <List>
                <For each={simulators()} keyed={(e) => e.name}>
                  {(e) => <Row heading={e().name} text={`${e().description} · ${STATE[e().state] ?? e().state} · ${e().machine} · used ${since(e().last_used)}`} current={open() === e().name} onClick={() => setOpen(e().name)} />}
                </For>
              </List>
            </Show>
          </Screen>
        </Split>
      </Loading>
    </Errored>
  )
}

render(() => <App />, document.body)
