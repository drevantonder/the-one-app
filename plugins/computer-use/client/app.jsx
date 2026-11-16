// Computer Use: the screens your agents can use, and one of them as it is now, with who last used it and for what.
import { createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render } from "@solidjs/web"
import { Button, call, changed, Empty, leave, List, Problem, Row, Screen, Section, Skeleton, Split } from "ui"
import "./app.css"

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// How long ago `at` was, in the largest unit that's whole: "2 minutes ago".
function since(at) {
  const minutes = Math.round((at - Date.now()) / 60_000)
  if (minutes > -60) return ago.format(Math.min(minutes, -1), "minute")
  if (minutes > -48 * 60) return ago.format(Math.round(minutes / 60), "hour")
  return ago.format(Math.round(minutes / (24 * 60)), "day")
}
const address = (screen) => `${screen.plugin}/${screen.name}`
const name = (who) => who?.split("/").pop()
const usage = (used) => (used ? `${used.doing ?? "Used"} · ${since(used.at)}` : "Not used yet")

const [open, setOpen] = createSignal()

/// The screen as it is now: a screenshot, taken again a second and a half after the last came, while it's in view.
function Live(props) {
  const [picture, setPicture] = createSignal()
  const [problem, setProblem] = createSignal()
  onSettled(() => {
    let timer
    let stopped = false
    const take = async () => {
      if (document.visibilityState === "visible") {
        const screen = props.screen
        await call("computer-use_screenshot", { screen: address(screen) })
          .then((data) => (setPicture({ of: address(screen), data }), setProblem()))
          .catch((error) => setProblem(error.message))
      }
      if (!stopped) timer = setTimeout(take, 1500)
    }
    take()
    return () => ((stopped = true), clearTimeout(timer))
  })
  const shown = () => (picture()?.of === address(props.screen) ? picture().data : undefined)
  return (
    <Show when={!problem()} fallback={<Problem heading="Couldn't see this screen" text={problem()} />}>
      <div class="live" style={{ "aspect-ratio": `${props.screen.width} / ${props.screen.height}` }}>
        <Show when={shown()}>
          <img src={shown()} alt={`${props.screen.name} as it is now`} />
        </Show>
      </div>
    </Show>
  )
}

function Detail(props) {
  const s = () => props.screen
  return (
    <Screen heading={s().name} back onBack={() => setOpen()}>
      <Live screen={s()} />
      <Section heading="Last Used">
        <List>
          <Row heading={s().used?.doing ?? "Not used yet"} text={s().used ? `${name(s().used.by)}'s agent · ${since(s().used.at)}` : undefined} />
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

function App() {
  const screens = createMemo(() => call("computer-use_screens").then(JSON.parse))
  changed(() => refresh(screens))
  const opened = () => screens().find((screen) => address(screen) === open())
  return (
    <Errored fallback={(error, reset) => <Problem heading="Couldn't list your screens" text={error().message} action={<Button variant="outline" onClick={reset}>Try Again</Button>} />}>
      <Loading fallback={<Skeleton rows={3} />}>
        <Split open={!!opened()} onClose={() => setOpen()} detail={<Show when={opened()} fallback={<Show when={screens().length}><Empty>Pick a screen to watch it.</Empty></Show>}><Detail screen={opened()} /></Show>}>
          <Screen heading="Computer Use" back onBack={leave}>
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
