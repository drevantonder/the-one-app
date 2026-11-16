import { Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Icon } from "./button.tsx"
import { said } from "./shared.ts"

/// `<Empty>No notes yet.</Empty>`: an empty screen or list: one grey line, and a button (`action`) only when
/// there's an obvious first step.
export function Empty(props: { children?: JSX.Element; action?: JSX.Element }) {
  return (
    <div class="ui-state">
      <p class="empty">{props.children}</p>
      {props.action}
    </div>
  )
}

/// `<Skeleton rows={3} />`: grey shapes the shape of the list that's coming, once it's taken longer than a blink,
/// so a quick load shows nothing in between. It's what `<Loading fallback>` shows.
export function Skeleton(props: { rows?: number }) {
  return (
    <div class="ui-state" role="status" aria-busy="true" aria-label={said.loading}>
      <div class="skeleton">
        {Array.from({ length: props.rows ?? 3 }, () => (
          <div>
            <i />
            <i />
          </div>
        ))}
      </div>
    </div>
  )
}

/// `<Problem heading="Couldn't load notes" text="The server didn't answer." action={<Button>Try again</Button>} />`:
/// a problem, shown on the thing it's about: a symbol, a short `heading`, one sentence of `text`, and the action
/// that fixes it. Red, for what needs the user now; `warn` for what can wait. It's what `<Errored fallback>` shows.
export function Problem(props: { heading: JSX.Element; text?: JSX.Element; action?: JSX.Element; warn?: boolean }) {
  return (
    <div class="ui-state" role="alert">
      <div class={["problem", { warn: props.warn }]}>
        <Icon name="warning-circle" />
        <strong>{props.heading}</strong>
        <Show when={props.text}>
          <p>{props.text}</p>
        </Show>
        <Show when={props.action}>
          <div class="action">{props.action}</div>
        </Show>
      </div>
    </div>
  )
}
