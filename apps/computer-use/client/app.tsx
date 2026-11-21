// Computer Use: the screens your agents can use, and one of them as it is now, with who last used it and for what.
import { createEffect, createMemo, createSignal, Errored, Loading, refresh, Show } from "solid-js"
import { Button, call, changed, client, Empty, List, LiveScreen, Problem, render, Row, Screen, Section, since, Skeleton, Split, styles, Text, tokens, visible } from "ui"

/** A screen agents can use, as `computer-use_screens` answers it. */
interface AgentScreen {
  app: string
  name: string
  description: string
  width: number
  height: number
  used?: { doing?: string; by?: string; at: number }
}

const look = styles({
  // A link in a sentence's words, in the accent, as the gallery's Text/Inline link is.
  link: { color: tokens.colour.blue, fontWeight: tokens.weight.medium },
})

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
          <Row heading={s().used?.doing ?? "Not used yet"} text={s().used ? `${s().used!.by ? `${name(s().used!.by)}'s agent` : "An agent"} · ${since(s().used!.at)}` : undefined} />
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
  createEffect(visible, (now, was) => {
    if (now && was === false) refresh(phone)
  })
  return (
    <Loading>
      <Show when={phone() && !phone()!.on}>
        <Problem
          warn
          heading="Your AI can't use this phone yet"
          text="Turn on the One App in Accessibility settings."
          action={<Button onPress={() => client({ settings: "accessibility" })}>Open Settings</Button>}
        />
        <List>
          <Row
            text={
              <>
                Restricted setting? Allow it under ⋮ in{" "}
                <Text role="link" tabIndex={0} onPress={() => client({ settings: "app" })} style={look.link}>
                  App Info
                </Text>
                .
              </>
            }
          />
        </List>
      </Show>
    </Loading>
  )
}

const screens = createMemo(() => call("computer-use_screens").then((text): AgentScreen[] => JSON.parse(text).screens))
changed(() => refresh(screens))

function App() {
  const opened = () => screens().find((screen) => address(screen) === open())
  return (
    <Show
      when={screens().length}
      fallback={
        <Screen heading="Computer Use">
          <ThisPhone />
          <Empty>No screens. Your agents' desktops show here once they make one.</Empty>
        </Screen>
      }
    >
      <Split open={!!opened()} onClose={() => setOpen()} pick="Pick a screen to watch it." detail={<Show when={opened()}><Detail screen={opened()!} /></Show>}>
        <Screen heading="Computer Use">
          <ThisPhone />
          <List each={screens()} keyed={address}>
            {(screen) => <Row heading={screen().name} text={usage(screen().used)} current={open() === address(screen())} onPress={() => setOpen(address(screen()))} />}
          </List>
        </Screen>
      </Split>
    </Show>
  )
}

render(() => (
  <Errored fallback={(error, reset) => <Screen heading="Computer Use"><Problem heading="Couldn't list your screens" text={error() as Error} action={<Button variant="outline" onPress={() => (refresh(screens), reset())}>Try Again</Button>} /></Screen>}>
    <Loading fallback={<Screen heading="Computer Use"><Skeleton rows={3} /></Screen>}>
      <App />
    </Loading>
  </Errored>
))
