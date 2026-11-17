// iPhone Simulator: your simulators on every Mac, and one of them live, to use as an iPhone, start, stop or remove.
import { createMemo, createSignal, Errored, For, Loading, Match, refresh, Show, Switch } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, leave, List, LiveScreen, Menu, Problem, Row, Screen, Skeleton, Split, Status, Steps, toast } from "ui"

/** A simulator, as `iphone-simulator_list` answers it. */
interface Simulator {
  name: string
  description: string
  state: State
  step?: "booting" | "connecting"
  why?: string
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
/** A simulator's state, as the service keeps it (simulator.ex's `@steps`). */
type State = "stopped" | "starting" | "running" | "stopping" | "removing" | "gone"
const SAYS: Record<State, string> = { stopped: "Stopped", starting: "Starting", running: "Running", stopping: "Stopping", removing: "Removing", gone: "Gone from Xcode" }
/// What a simulator's row says of its state.
const says = (e: Simulator) => (e.state === "stopped" && e.why ? "Couldn't start" : SAYS[e.state])
/// What its menu offers in each state: none while it's being removed.
const ACTIONS: Record<State, ("start" | "stop" | "remove")[]> = { stopped: ["start", "remove"], starting: ["stop", "remove"], running: ["stop", "remove"], stopping: ["remove"], removing: [], gone: ["remove"] }

const [open, setOpen] = createSignal<string>()

function Detail(props: { simulator: Simulator }) {
  const e = () => props.simulator
  const [busy, setBusy] = createSignal(false)
  const act = async (value: string) => {
    if (busy()) return
    if (value === "remove") {
      const yes = await confirm({ heading: `Remove ${e().name}?`, text: "Its apps and everything on it go with it, for good.", action: "Remove", destructive: true })
      if (!yes) return
    }
    setBusy(true)
    await call(`iphone-simulator_${value}`, { simulator: e().name })
      .then(() => value === "remove" && setOpen())
      // A start that fails says why where it would have shown, as the problem it's stopped with.
      .catch((error) => value !== "start" && toast(error))
      .finally(() => setBusy(false))
  }
  const actions = () => ACTIONS[e().state]
  return (
    <Screen
      heading={e().name}
      back
      onBack={() => setOpen()}
      action={
        <Show when={actions().length}>
          <Menu.Root onSelect={({ value }) => act(value)}>
            <Menu.Trigger aria-label={`More for ${e().name}`} />
            <Menu.Positioner>
              <Menu.Content>
                <Show when={actions().includes("start")}>
                  <Menu.Item value="start">Start</Menu.Item>
                </Show>
                <Show when={actions().includes("stop")}>
                  <Menu.Item value="stop">Stop</Menu.Item>
                </Show>
                <Show when={actions().length > 1}>
                  <Menu.Separator />
                </Show>
                <Menu.Item value="remove" destructive>Remove</Menu.Item>
              </Menu.Content>
            </Menu.Positioner>
          </Menu.Root>
        </Show>
      }
    >
      <Switch>
        <Match when={e().state === "running" && e().name} keyed>
          {(name) => <LiveScreen label={`${name}'s screen`} screen={(inputs) => call("iphone-simulator_screen", { simulator: name, ...inputs })} />}
        </Match>
        <Match when={e().state === "starting"}>
          <Steps
            steps={["Starting iOS", "Connecting to its screen"]}
            at={e().step === "connecting" ? 1 : 0}
            text={e().step === "connecting" ? "The first time takes a few seconds more." : undefined}
          />
        </Match>
        <Match when={e().state === "stopping"}>
          <Status heading="Stopping…" />
        </Match>
        <Match when={e().state === "removing"}>
          <Status heading="Removing…" />
        </Match>
        <Match when={e().state === "gone"}>
          <Problem warn heading="Gone from Xcode" text="It was deleted outside The One App." action={<Button variant="outline" loading={busy()} onClick={() => act("remove")}>Remove</Button>} />
        </Match>
        <Match when={e().why}>
          <Problem heading={`Couldn't start ${e().name}`} text={e().why} action={<Button variant="outline" loading={busy()} onClick={() => act("start")}>Try Again</Button>} />
        </Match>
        <Match when={e().state === "stopped"}>
          <Empty action={<Button loading={busy()} onClick={() => act("start")}>Start</Button>}>Stopped.</Empty>
        </Match>
      </Switch>
    </Screen>
  )
}

function App() {
  const simulators = createMemo(() => call("iphone-simulator_list").then((text): Simulator[] => JSON.parse(text).simulators))
  changed(() => refresh(simulators))
  const opened = () => simulators().find((e) => e.name === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your simulators" text={error() as Error} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={simulators().length}><Empty>Pick a simulator to use it.</Empty></Show>}><Detail simulator={opened()!} /></Show>}>
          <Screen heading="iPhone Simulator" back onBack={leave}>
            <Show when={simulators().length} fallback={<Empty>No simulators. Your agents create them as they need them.</Empty>}>
              <List>
                <For each={simulators()} keyed={(e) => e.name}>
                  {(e) => <Row heading={e().name} text={`${e().description} · ${says(e())} · ${e().machine} · used ${since(e().last_used)}`} current={open() === e().name} onClick={() => setOpen(e().name)} />}
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
