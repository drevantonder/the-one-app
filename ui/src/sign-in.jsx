import { createSignal, Show } from "solid-js"
import { Button, Icon } from "./button.jsx"

/// `<SignIn mark={<img src="mark.png" alt="" />} heading="Sign in" button="Sign in" onSignIn={passkey} />`: the one
/// screen someone signs in on, wherever it's shown: the `mark` of whose it is, a `heading` and one `button`.
/// `onSignIn` asks them, and what it throws is shown above the button to try again. A `warning` says who will act
/// as them if they do, and stays until they have. `done` is the one line that's left once they have. With `href`
/// the button is a link to where signing in happens, for a screen that can't ask itself.
export function SignIn(props) {
  const [working, setWorking] = createSignal(false)
  const [problem, setProblem] = createSignal()

  const signIn = async () => {
    setProblem()
    setWorking(true)
    try {
      await props.onSignIn()
    } catch (error) {
      setProblem(error.message)
    } finally {
      setWorking(false)
    }
  }

  return (
    <main class="ui-sign-in">
      <Show when={props.mark}>
        <div class="mark">{props.mark}</div>
      </Show>
      <h1>{props.heading}</h1>
      <Show when={!props.done} fallback={<p class="done">{props.done}</p>}>
        <Show when={props.warning}>
          <p class="warning">
            <Icon name="warning-circle" />
            <span>{props.warning}</span>
          </p>
        </Show>
        <Show when={problem()}>
          <p class="problem" role="alert">
            <Icon name="warning-circle" />
            <span>{problem()}</span>
          </p>
        </Show>
        <Show when={props.href} fallback={<Button loading={working()} onClick={signIn}>{props.button}</Button>}>
          <a class="ui-button press ring" data-variant="default" href={props.href}>
            {props.button}
          </a>
        </Show>
      </Show>
    </main>
  )
}
