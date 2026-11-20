// Android Emulator: your emulators on every machine, and one of them live, to use as a phone, start, stop or remove.
import { createMemo, createSignal, Errored, Loading, Match, refresh, Show, Switch } from "solid-js"
import { Button, call, changed, confirm, Empty, List, LiveScreen, Menu, Problem, render, Row, Screen, since, Skeleton, Split, Status, Steps, toast } from "ui"

/** An emulator, as `android-emulator_list` answers it. */
interface Emulator {
  name: string
  description: string
  state: State
  step?: "preparing" | "booting"
  why?: string
  machine: string
  last_used: number
}

/** An emulator's state, as the service keeps it (emulator.ex's `@steps`). */
type State = "stopped" | "starting" | "running" | "stopping" | "removing"
const SAYS: Record<State, string> = { stopped: "Stopped", starting: "Starting", running: "Running", stopping: "Stopping", removing: "Removing" }
/// What an emulator's row says of its state.
const says = (e: Emulator) => (e.state === "stopped" && e.why ? "Couldn't start" : SAYS[e.state])
/// What its menu offers in each state: none while it's being removed.
const ACTIONS: Record<State, ("start" | "stop" | "remove")[]> = { stopped: ["start", "remove"], starting: ["stop", "remove"], running: ["stop", "remove"], stopping: ["remove"], removing: [] }

const [open, setOpen] = createSignal<string>()

function Detail(props: { emulator: Emulator }) {
  const e = () => props.emulator
  const [busy, setBusy] = createSignal(false)
  const act = async (value: string) => {
    if (busy()) return
    if (value === "remove") {
      const yes = await confirm({ heading: `Remove ${e().name}?`, text: "Its apps and everything on it go with it, for good.", action: "Remove", destructive: true })
      if (!yes) return
    }
    setBusy(true)
    await call(`android-emulator_${value}`, { emulator: e().name })
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
          {(emulator) => <LiveScreen label={`${emulator}'s screen`} screen={(inputs) => call("android-emulator_screen", { emulator, ...inputs })} />}
        </Match>
        <Match when={e().state === "starting"}>
          <Steps
            steps={["Getting the emulator ready", "Starting Android"]}
            at={e().step === "booting" ? 1 : 0}
            text={e().step === "booting" ? undefined : "The first time on a machine takes a few minutes."}
          />
        </Match>
        <Match when={e().state === "stopping"}>
          <Status heading="Stopping…" text="It saves where it was, to carry on from there." />
        </Match>
        <Match when={e().state === "removing"}>
          <Status heading="Removing…" />
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
  const emulators = createMemo(() => call("android-emulator_list").then((text): Emulator[] => JSON.parse(text).emulators))
  changed(() => refresh(emulators))
  const opened = () => emulators().find((e) => e.name === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your emulators" text={error() as Error} action={<Button variant="outline" onPress={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={emulators().length}><Empty>Pick an emulator to use it.</Empty></Show>}><Detail emulator={opened()!} /></Show>}>
          <Screen heading="Android Emulator">
            <Show when={emulators().length} fallback={<Empty>No emulators. Your agents create them as they need them.</Empty>}>
              <List each={emulators()} keyed={(e) => e.name}>
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
