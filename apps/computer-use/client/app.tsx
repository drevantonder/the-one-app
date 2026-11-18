// Computer Use: the screens your agents can use, and one of them as it is now, with who last used it and for what.
import { createMemo, createSignal, Errored, For, Loading, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, client, Empty, List, LiveScreen, Problem, Row, Screen, Section, Skeleton, Split } from "ui"

/** A screen agents can use, as `computer-use_screens` answers it. */
interface AgentScreen {
  app: string
  name: string
  description: string
  width: number
  height: number
  used?: { doing?: string; by?: string; at: number }
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 minutes ago".
function since(at: number) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -60) return ago.format(Math.min(minutes, -1), "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const address = (screen: AgentScreen) => `${screen.app}/${screen.name}`
const name = (who?: string) => who?.split("/").pop()
const usage = (used: AgentScreen["used"]) => (used ? `${used.doing ?? "Used"} · ${since(used.at)}` : "Not used yet")

const [open, setOpen] = createSignal<string>()

function Detail(props: { screen: AgentScreen }) {
  const s = () => props.screen
  return (
    <Screen heading={s().name} back onBack={() => setOpen()}>
      <Show when={address(s())} keyed>
        {(screen) => (
          <LiveScreen
            label={`${s().name} as it is now`}
            watch
            screen={({ action, ...inputs }) => call(action === "stream" ? "computer-use_stream" : "computer-use_screenshot", { screen, ...inputs })}
          />
        )}
      </Show>
      <Section heading="Last Used">
        <List>
          <Row heading={s().used?.doing ?? "Not used yet"} text={s().used ? `${name(s().used!.by)}'s agent · ${since(s().used!.at)}` : undefined} />
        </List>
      </Section>
      <Section heading="Screen">
        <List>
          <Row heading={s().description} text={`${s().width} × ${s().height}`} />
        </List>
      </Section>
    </Screen>
  )
}

/// On a phone whose person hasn't turned Computer Use on, how they do, in Android's own Settings: the One App in
/// Accessibility, after allowing restricted settings in its App Info on Android 13 and later, for an app that didn't
/// come from a store. Nothing once it's on, or on a device that has none.
function ThisPhone() {
  const phone = createMemo(() => client().then(({ computerUse }) => computerUse))
  // Its person comes back from Settings to here.
  const back = () => document.visibilityState === "visible" && refresh(phone)
  document.addEventListener("visibilitychange", back)
  return (
    <Loading>
      <Show when={phone() && !phone()!.on}>
        <Problem
          warn
          heading="Your AI can't use this phone yet"
          text="Turn on the One App in Accessibility settings."
          action={<Button onClick={() => client({ settings: "accessibility" })}>Open Settings</Button>}
        />
        <List>
          <Row
            text="Restricted setting? Allow it under ⋮ in App Info."
            action={<Button variant="ghost" onClick={() => client({ settings: "app" })}>App Info</Button>}
          />
        </List>
      </Show>
    </Loading>
  )
}

function App() {
  const screens = createMemo(() => call("computer-use_screens").then((text): AgentScreen[] => JSON.parse(text).screens))
  changed(() => refresh(screens))
  const opened = () => screens().find((screen) => address(screen) === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your screens" text={error() as Error} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={screens().length}><Empty>Pick a screen to watch it.</Empty></Show>}><Detail screen={opened()!} /></Show>}>
          <Screen heading="Computer Use">
            <ThisPhone />
            <Show when={screens().length} fallback={<Empty>No screens. Your agents' desktops show here once they make one.</Empty>}>
              <List>
                <For each={screens()} keyed={address}>
                  {(screen) => <Row heading={screen().name} text={usage(screen().used)} current={open() === address(screen())} onClick={() => setOpen(address(screen()))} />}
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
