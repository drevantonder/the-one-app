import { createSignal, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Button, Icon, Spinner } from "./button.tsx"
import { messages, readable, whole } from "./shared.ts"

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
    <div class="ui-state" role="status" aria-busy="true" aria-label={messages.loading}>
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
/// A `text` that's an error, as `call` throws it (`<Errored fallback={(error) => <Problem text={error()} />}>`), or a
/// tool's error as text, shows what a person reads of it (`readable`), and the rest under Show Details, to copy for an
/// agent or for support.
export function Problem(props: { heading: JSX.Element; text?: JSX.Element | Error; action?: JSX.Element; warn?: boolean }) {
  const all = () => (typeof props.text === "string" || props.text instanceof Error ? whole(props.text) : undefined)
  const [open, setOpen] = createSignal(false)
  return (
    <div class="ui-state" role="alert">
      <div class={["problem", { warn: props.warn }]}>
        <Icon name="warning-circle" />
        <strong>{props.heading}</strong>
        <Show when={props.text}>
          <p>{all() === undefined ? (props.text as JSX.Element) : readable(all()!)}</p>
        </Show>
        <Show when={all() && all() !== readable(all()!)}>
          <details onToggle={(event) => setOpen(event.currentTarget.open)}>
            <summary class="ring">
              <Icon name="caret-right" along />
              {open() ? messages.hideDetails : messages.showDetails}
            </summary>
            <Details text={all()!} />
          </details>
        </Show>
        <Show when={props.action}>
          <div class="action">{props.action}</div>
        </Show>
      </div>
    </div>
  )
}

/// What follows the first paragraph of a tool's error, which is for an agent, and Copy, which takes all of it to
/// paste to an agent or to support. Under a problem's Show Details, and in what a toast's Details opens.
export function Details(props: { text: string }) {
  const [copied, setCopied] = createSignal(false)
  return (
    <div class="ui-details">
      <pre>{props.text.slice(readable(props.text).length).trim()}</pre>
      <button type="button" class="ui-button press ring" data-variant="ghost" data-size="icon" aria-label={copied() ? messages.copied : messages.copy} title={messages.copy} onClick={() => navigator.clipboard.writeText(props.text).then(() => setCopied(true))}>
        <Icon name={copied() ? "check" : "copy"} />
      </button>
    </div>
  )
}

/// `<Status heading="Connecting…" />`: something under way where what it's about will be: a spinner, a short
/// `heading` and, only if it helps, one sentence of `text`, in the middle of its place. With `over`, it's a card on
/// what's there now, which it dims: a live view reconnecting, something saving. Its place is laid out by the UI
/// library, or `position: relative`.
export function Status(props: { heading: JSX.Element; text?: JSX.Element; over?: boolean }) {
  return (
    <div class={["ui-state", "middle", { over: props.over }]} role="status">
      <div class="status">
        <Spinner />
        <strong>{props.heading}</strong>
        <Show when={props.text}>
          <p>{props.text}</p>
        </Show>
      </div>
    </div>
  )
}

/// `<Steps steps={["Getting a browser ready", "Opening Personal"]} at={0} text="The first time takes a few minutes." />`:
/// something starting, as the steps it takes: those before `at` done, `at` under way with, only if it helps, one
/// sentence of `text`, and the rest to come. In the middle of its place, as `Status` is.
export function Steps(props: { steps: JSX.Element[]; at: number; text?: JSX.Element }) {
  return (
    <div class="ui-state middle" role="status">
      <ol class="steps">
        {props.steps.map((step, i) => (
          <li class={{ done: i < props.at, now: i === props.at }}>
            {i < props.at ? <Icon name="check" /> : i === props.at ? <Spinner /> : <span class="next" />}
            <span>
              {step}
              <Show when={i === props.at && props.text}>
                <small>{props.text}</small>
              </Show>
            </span>
          </li>
        ))}
      </ol>
    </div>
  )
}
