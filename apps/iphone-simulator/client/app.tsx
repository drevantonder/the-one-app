// iPhone Simulator: your simulators on every Mac, and one of them live, to use as an iPhone, start, stop or remove.
import { createMemo, createSignal, Errored, Loading, Match, refresh, Show, Switch } from "solid-js"
import { Button, call, changed, confirm, Empty, List, LiveScreen, Menu, Problem, render, Row, Screen, since, Skeleton, Split, Status, Steps, toast } from "ui"

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
      .catch((error) => {
        if (value !== "start") toast(error)
      })
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
          <Menu.Root onSelect={(value) => act(value)}>
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
          <Problem warn heading="Gone from Xcode" text="It was deleted outside the One App." action={<Button variant="outline" loading={busy()} onPress={() => act("remove")}>Remove</Button>} />
        </Match>
        <Match when={e().why}>
          <Problem heading={`Couldn't start ${e().name}`} text={e().why} action={<Button variant="outline" loading={busy()} onPress={() => act("start")}>Try Again</Button>} />
        </Match>
        <Match when={e().state === "stopped"}>
          <Empty action={<Button loading={busy()} onPress={() => act("start")}>Start</Button>}>Stopped.</Empty>
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
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your simulators" text={error() as Error} action={<Button variant="outline" onPress={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} pick="Pick a simulator to use it." detail={<Show when={opened()}><Detail simulator={opened()!} /></Show>}>
          <Screen heading="iPhone Simulator">
            <Show when={simulators().length} fallback={<Empty>No simulators. Your agents create them as they need them.</Empty>}>
              <List each={simulators()} keyed={(e) => e.name}>
                {(e) => <Row heading={e().name} text={[e().description, says(e()), e().machine, e().last_used && `used ${since(e().last_used)}`].filter(Boolean).join(" · ")} current={open() === e().name} onPress={() => setOpen(e().name)} />}
              </List>
            </Show>
          </Screen>
        </Split>
      </Loading>
    </Errored>
  )
}

render(() => <App />)
