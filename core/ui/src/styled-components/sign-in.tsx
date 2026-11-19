import { createSignal, Show } from "solid-js"
import { Button, Icon, Spinner } from "./button.tsx"
import type { JSX } from "@solidjs/web"

// `<SignIn mark={<img src={mark} alt="" />} heading="Sign in" button="Sign in" onSignIn={passkey} />`: the one
// screen someone signs in on, wherever it's shown: the `mark` of whose it is, a `heading` and one `button`.
// `onSignIn` asks them, and what it throws is shown above the button to try again, as a `problem` given is. A
// `code` is shown big, under a grey `note` that says what to do with it. A `warning` says who will act as them if
// they do, and stays until they have. `stop` is a second button, for saying no, which `onStop` does. `done` is
// the one line that's left once they've answered. With `href` the button is a link to where signing in happens,
// for a screen that can't ask itself. `status` is what's under way while there's nothing to press yet, a spinner
// and one line where the buttons go; with `quiet` the button is one of two ways on while it's under way, an outline,
// under the status, and not the one to press. Return in a field it asks does what the button does. What else it asks, such as a `Field` for their name, goes in it, above
// the button.
export function SignIn(props: {
  mark?: JSX.Element
  heading?: JSX.Element
  button?: JSX.Element
  quiet?: boolean
  onSignIn?: () => unknown
  href?: string
  problem?: JSX.Element
  code?: JSX.Element
  note?: JSX.Element
  warning?: JSX.Element
  stop?: JSX.Element
  onStop?: () => unknown
  done?: JSX.Element
  status?: JSX.Element
  children?: JSX.Element
}) {
  const [working, setWorking] = createSignal<"sign-in" | "stop">()
  const [problem, setProblem] = createSignal<string>()

  const respond = (which: "sign-in" | "stop", does: () => unknown) => async () => {
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
    <main
      class="ui-sign-in"
      onKeyDown={(event) => {
        if (event.key !== "Enter" || (event.target as HTMLElement).tagName !== "INPUT" || !props.button || props.status || props.quiet || working()) return
        event.preventDefault()
        if (props.href) location.assign(props.href)
        else respond("sign-in", () => props.onSignIn?.())()
      }}
    >
      <Show when={props.mark}>
        <div class="mark">{props.mark}</div>
      </Show>
      <h1>{props.heading}</h1>
      <Show when={!props.done} fallback={<p class="done">{props.done}</p>}>
        <Show when={props.code}>
          <div class="code">
            <p>{props.note}</p>
            <p class="shown">{props.code}</p>
          </div>
        </Show>
        <Show when={props.warning}>
          <p class="warning">
            <Icon name="warning-circle" />
            <span>{props.warning}</span>
          </p>
        </Show>
        {props.children}
        <Show when={problem() ?? props.problem}>
          <p class="problem" role="alert">
            <Icon name="warning-circle" />
            <span>{problem() ?? props.problem}</span>
          </p>
        </Show>
        <Show when={props.status}>
          <p class="status" role="status">
            <Spinner />
            <span>{props.status}</span>
          </p>
        </Show>
        <Show when={props.button && (!props.status || props.quiet)}>
          <div class="answers">
            <Button
              variant={props.quiet ? "outline" : "default"}
              loading={working() === "sign-in"}
              disabled={!!working()}
              onPress={props.href ? () => location.assign(props.href!) : respond("sign-in", () => props.onSignIn?.())}
            >
              {props.button}
            </Button>
            <Show when={props.stop}>
              <Button variant="outline" loading={working() === "stop"} disabled={!!working()} onPress={respond("stop", () => props.onStop?.())}>
                {props.stop}
              </Button>
            </Show>
          </div>
        </Show>
      </Show>
    </main>
  )
}
