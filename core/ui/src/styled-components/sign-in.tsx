// The one screen someone signs in on, written once on the drawing primitives (#primitives): the same on the DOM, as
// the network's sign-in pages show it, and on a native host, as the app's own window does before it's in.
import { children, createSignal, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Image, Text, View } from "#primitives"
import { Button, Icon, Spinner } from "./button.tsx"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  // In the middle of the window, which it's the whole of.
  screen: {
    flexGrow: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: tokens.grid[6],
    paddingTop: tokens.length.safeTop,
    paddingBottom: tokens.length.safeBottom,
    paddingInline: tokens.length.gutter,
    textAlign: "center",
  },
  part: { maxWidth: 352, gap: tokens.grid[1], alignItems: "center" },
  // What else it asks, as wide as it may be: a field has no width of its own.
  asks: { width: "100%", maxWidth: 352, gap: tokens.grid[1], textAlign: "start" },
  mark: { width: 88, height: 88 },
  heading: { font: tokens.type.largeTitle },
  grey: { color: tokens.colour.secondaryLabel },
  shown: { font: tokens.type.largeTitle, fontFamily: tokens.font.mono },
  said: { flexDirection: "row", gap: tokens.grid[2.5], alignItems: "flex-start", color: tokens.colour.orange, textAlign: "start" },
  problem: { color: tokens.colour.red },
  symbol: { paddingTop: tokens.grid[0.5] },
  words: { flexShrink: 1 },
  status: { flexDirection: "row", gap: tokens.grid[2], alignItems: "center", color: tokens.colour.secondaryLabel },
  // As wide as its widest button, which the other is too.
  answers: { maxWidth: 352, gap: tokens.grid[2] },
  // `stop` first, then the button, side by side.
  beside: { flexDirection: "row-reverse" },
  half: { flexGrow: 1 },
})

/// `<SignIn mark={logo} heading="Sign in" button="Sign in" onSignIn={passkey} />`: the one screen someone signs in on,
/// wherever it's shown: the `mark` of whose it is (a picture's address, or what draws one), a `heading` and one
/// `button`. `onSignIn` asks them, and what it throws is shown above the button to try again, as a `problem` given
/// is. A `code` is shown big, under a grey `note` that says what to do with it. A `warning` says who will act as them
/// if they do, and stays until they have. `stop` is a second button, for saying no, which `onStop` does. `done` is
/// the one line that's left once they've answered. `status` is what's under way while there's nothing to press yet, a
/// spinner and one line where the buttons go; with `quiet` the button is one of two ways on while it's under way, an
/// outline, under the status, and not the one to press. With `beside`, `stop` goes beside the button, before it, as a
/// consent page's Cancel and Allow do. Return in a field it asks does what the button does. What else
/// it asks, such as a `Field` for their name, goes in it, above the button.
export function SignIn(props: {
  mark?: string | JSX.Element
  heading?: JSX.Element
  button?: JSX.Element
  quiet?: boolean
  onSignIn?: () => unknown
  problem?: JSX.Element
  code?: JSX.Element
  note?: JSX.Element
  warning?: JSX.Element
  stop?: JSX.Element
  onStop?: () => unknown
  beside?: boolean
  done?: JSX.Element
  status?: JSX.Element
  children?: JSX.Element
}) {
  const [working, setWorking] = createSignal<"sign-in" | "stop">()
  const [problem, setProblem] = createSignal<string>()
  // What else it asks, made once, and nothing where each part of it is left out (`{asks && <Field>…}`).
  const asked = children(() => props.children)

  const respond = (which: "sign-in" | "stop", does: () => unknown) => async () => {
    if (working()) return
    setProblem()
    setWorking(which)
    try {
      await does()
    } catch (error) {
      setProblem((error as Error).message)
    } finally {
      setWorking()
    }
  }

  return (
    <View
      role="main"
      style={look.screen}
      onKeyDown={(event) => {
        if (event.key !== "Enter" || !props.button || props.status || props.quiet || props.done) return
        event.preventDefault()
        respond("sign-in", () => props.onSignIn?.())()
      }}
    >
      <Show when={props.mark}>{(mark) => (typeof mark() === "string" ? <Image src={mark() as string} alt="" style={look.mark} /> : mark())}</Show>
      <Text role="heading" aria-level={1} style={look.heading}>
        {props.heading}
      </Text>
      <Show
        when={!props.done}
        fallback={
          <View style={look.part}>
            <Text style={look.grey}>{props.done}</Text>
          </View>
        }
      >
        <Show when={props.code}>
          <View style={look.part}>
            <Text style={look.grey}>{props.note}</Text>
            <Text style={look.shown}>{props.code}</Text>
          </View>
        </Show>
        <Show when={props.warning}>
          <View style={look.part}>
            <View style={look.said}>
              <View style={look.symbol}>
                <Icon name="warning-circle" />
              </View>
              <Text style={look.words}>{props.warning}</Text>
            </View>
          </View>
        </Show>
        <Show when={asked.toArray().some(Boolean)}>
          <View style={look.asks}>{asked()}</View>
        </Show>
        <Show when={problem() ?? props.problem}>
          <View style={look.part}>
            <View role="alert" style={[look.said, look.problem]}>
              <View style={look.symbol}>
                <Icon name="warning-circle" />
              </View>
              <Text style={look.words}>{problem() ?? props.problem}</Text>
            </View>
          </View>
        </Show>
        <Show when={props.status}>
          <View role="status" style={look.status}>
            <Spinner />
            <Text>{props.status}</Text>
          </View>
        </Show>
        <Show when={props.button && (!props.status || props.quiet)}>
          <View style={[look.answers, props.beside && look.beside]}>
            <Button variant={props.quiet ? "outline" : "default"} loading={working() === "sign-in"} disabled={!!working()} style={props.beside ? look.half : undefined} onPress={respond("sign-in", () => props.onSignIn?.())}>
              {props.button}
            </Button>
            <Show when={props.stop}>
              <Button variant="outline" loading={working() === "stop"} disabled={!!working()} style={props.beside ? look.half : undefined} onPress={respond("stop", () => props.onStop?.())}>
                {props.stop}
              </Button>
            </Show>
          </View>
        </Show>
      </Show>
    </View>
  )
}
