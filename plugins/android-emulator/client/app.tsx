// Android Emulator: your emulators on every machine, and one of them live, to use as a phone, start, stop or remove.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, confirm, Empty, leave, List, LiveScreen, Menu, Problem, Row, Screen, Skeleton, Split, toast } from "ui"

/** An emulator, as `android-emulator_list` answers it. */
interface Emulator {
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
const STATE: Record<string, string> = { running: "Running", starting: "Starting", stopping: "Stopping", stopped: "Stopped" }

const [open, setOpen] = createSignal<string>()

function Detail(props: { emulator: Emulator }) {
  const e = () => props.emulator
  const [busy, setBusy] = createSignal(false)
  const act = async ({ value }: { value: string }) => {
    if (value === "remove") {
      const yes = await confirm({ heading: `Remove ${e().name}?`, text: "Its apps and everything on it go with it, for good.", action: "Remove", destructive: true })
      if (!yes) return
    }
    setBusy(true)
    await call(`android-emulator_${value}`, { emulator: e().name })
      .then(() => value === "remove" && setOpen())
      .catch(toast)
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
        {(emulator) => <LiveScreen label={`${emulator}'s screen`} screen={(inputs) => call("android-emulator_screen", { emulator, ...inputs })} />}
      </Show>
    </Screen>
  )
}

function App() {
  const emulators = createMemo(() => call("android-emulator_list").then((text): Emulator[] => JSON.parse(text).emulators))
  changed(() => refresh(emulators))
  const opened = () => emulators().find((e) => e.name === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your emulators" text={error() as Error} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={emulators().length}><Empty>Pick an emulator to use it.</Empty></Show>}><Detail emulator={opened()!} /></Show>}>
          <Screen heading="Android Emulator" back onBack={leave}>
            <Show when={emulators().length} fallback={<Empty>No emulators. Your agents create them as they need them.</Empty>}>
              <List>
                <For each={emulators()} keyed={(e) => e.name}>
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
