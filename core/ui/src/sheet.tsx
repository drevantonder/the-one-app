import { createEffect, createSignal, onSettled, Show } from "solid-js"
import { render } from "@solidjs/web"
import { covers } from "./bridge.ts"
import { Button, Icon } from "./button.tsx"
import { messages, readable, reduced } from "./shared.ts"
import { Details } from "./states.tsx"
import { hold } from "./toast.tsx"
import type { JSX } from "@solidjs/web"

/// The sheets and alerts that are open, the last opened last.
const open: HTMLDialogElement[] = []

/// Where a sheet is in the middle of the window, not at its foot (ui.css).
const wide = matchMedia("(min-width: 40rem)")

/// The browser's own `<dialog>`, shown over the page while `props.open`. The browser keeps focus inside it and
/// closes it on Escape and on the device's back gesture; a press outside closes it too. It calls `onClose` once
/// it's shut, however that came about.
function Dialog(props: {
  open?: boolean
  onClose?: () => void
  class?: string
  role?: "alertdialog"
  labelledby?: string
  describedby?: string
  ref?: (dialog: HTMLDialogElement) => void
  children?: JSX.Element
}) {
  let dialog!: HTMLDialogElement
  let pressed: EventTarget | null = null
  let shut: (() => void) | undefined

  /// An open dialog makes everything outside it inert, a toast too. So toasts show from inside the one that's on
  /// top, and from the page again once none is open. And an open dialog is what a device's own back closes.
  const topmost = (is: boolean) => {
    if (open.includes(dialog)) open.splice(open.indexOf(dialog), 1)
    if (is) open.push(dialog)
    hold(open.at(-1))
    shut?.()
    shut = is ? covers(() => dialog.close()) : undefined
  }

  createEffect(() => !!props.open, (now) => {
    if (now === dialog.open) return
    if (!now) return dialog.close()
    dialog.showModal()
    topmost(true)
    // The keyboard doesn't come up on a phone unless the app asked for it, with `autofocus` on a field.
    ;(dialog.querySelector<HTMLElement>("[autofocus]") ?? dialog).focus()
  })
  onSettled(() => () => topmost(false))

  /// A press that both starts and ends on the dialog itself was on what's around it.
  const outside = (event: PointerEvent | MouseEvent) => {
    if (event.type === "pointerdown") pressed = event.target
    else if (event.target === dialog && pressed === dialog) dialog.close()
  }

  const closed = () => {
    // Opened again before the browser said it had closed: it's open, and that's not news.
    if (dialog.open) return
    topmost(false)
    props.onClose?.()
  }

  return (
    <dialog
      ref={[(element: HTMLDialogElement) => (dialog = element), props.ref]}
      class={props.class}
      role={props.role}
      aria-labelledby={props.labelledby}
      aria-describedby={props.describedby}
      onPointerDown={outside}
      onClick={outside}
      onClose={closed}
    >
      {props.children}
    </dialog>
  )
}

/// `<Sheet heading="Add note" open={adding()} onClose={() => setAdding(false)}>...</Sheet>`: something to fill in
/// or look at, over the page: up from the foot of a narrow window, in the middle of a wide one, while `open`.
/// Escape, the device's back, a press outside and a pull down close it too: each calls `onClose`, which sets
/// `open` false. `autofocus` on a field in it brings up a phone's keyboard as it opens.
export function Sheet(props: { heading?: JSX.Element; open?: boolean; onClose?: () => void; children?: JSX.Element }) {
  let dialog!: HTMLDialogElement
  let pulling: { x: number; y: number; down: number; moves: { time: number; value: number }[]; started?: number; fade?: Animation } | null = null
  const id = `ui-sheet-${Math.random().toString(36).slice(2)}`

  /// A sheet at the foot of the window goes down with the finger that pulls it, and on letting go closes if it was
  /// pulled far or flicked, and comes back if not. A finger that's scrolling what's in the sheet isn't pulling it:
  /// only one that goes down with nothing above left to scroll to. Only the sheet's own `translate` changes as it
  /// goes, and the backdrop's fade is held at how far down it is: nothing inside the sheet is restyled.
  const pull = (event: TouchEvent) => {
    const finger = event.touches[0] ?? event.changedTouches[0]
    if (event.type === "touchstart") {
      const under = event.composedPath()
      const scrolled = (under.slice(0, under.indexOf(dialog)) as Element[]).some((element) => element.scrollTop > 0)
      pulling = wide.matches || scrolled || event.touches.length > 1 ? null : { x: finger.clientX, y: finger.clientY, down: 0, moves: [] }
      return
    }
    const was = pulling
    if (!was) return
    const down = finger.clientY - was.y
    if (event.type === "touchmove") {
      if (was.started === undefined) {
        const across = Math.abs(finger.clientX - was.x)
        // Not far enough yet to tell which way it's going.
        if (Math.hypot(across, down) < 4) return
        // Going up or sideways, or the device has already started scrolling: it's not a pull.
        if (down < across || !event.cancelable) return void (pulling = null)
        was.started = event.timeStamp
        dialog.setAttribute("data-pulled", "")
        was.fade = dialog.animate([{ opacity: 1 }, { opacity: 0 }], { pseudoElement: "::backdrop", duration: 1000, fill: "both" })
        was.fade.pause()
      }
      event.preventDefault()
      was.down = down
      was.moves = [...was.moves.slice(-31), { time: event.timeStamp, value: down }]
      dialog.style.translate = `0 ${Math.max(0, down)}px`
      was.fade!.currentTime = 1000 * Math.min(1, Math.max(0, down) / dialog.offsetHeight)
      return
    }
    pulling = null
    if (was.started === undefined) return
    was.fade!.cancel()
    dialog.removeAttribute("data-pulled")
    // How fast it was going down as it was let go, in px a millisecond, from the last move at least 80 ms before the
    // last, as a slow device's moves come further apart: none if it had stopped.
    const last = was.moves.at(-1)
    const from = was.moves.findLast((move) => last!.time - move.time >= 80) ?? was.moves[0]
    const speed = !last || event.timeStamp - last.time > 80 ? 0 : (last.value - from.value) / Math.max(1, last.time - from.time)
    // Let go a third of the way down, or flicked: still going down, and fast enough on average since it was taken.
    const flicked = speed > 0 && was.down / Math.max(1, event.timeStamp - was.started) > 0.11
    if (event.type !== "touchend" || !(was.down > dialog.offsetHeight / 3 || flicked)) return void dialog.style.removeProperty("translate")
    // For whoever asked for less motion it fades where it was let go. Otherwise it goes on down, no slower than the
    // finger took it.
    if (!reduced.matches) {
      dialog.style.removeProperty("translate")
      if (flicked) dialog.style.transitionDuration = `${Math.max(120, Math.min(200, (dialog.offsetHeight - was.down) / speed))}ms`
    }
    dialog.close()
    // Reading the transitions starts them, at that pace; what was set for them goes once they've ended.
    Promise.allSettled(dialog.getAnimations().map((move) => move.finished)).then(() => {
      dialog.style.removeProperty("translate")
      dialog.style.removeProperty("transition-duration")
    })
  }
  const touched = (element: HTMLDialogElement) => {
    for (const type of ["touchstart", "touchmove", "touchend", "touchcancel"] as const) element.addEventListener(type, pull, { passive: false })
  }

  return (
    <Dialog class="ui-dialog sheet" open={props.open} onClose={props.onClose} labelledby={id} ref={(element) => touched((dialog = element))}>
      <div class="panel">
        <header>
          <h2 id={id}>{props.heading}</h2>
          <button class="press ring" aria-label={messages.close} onClick={() => dialog.close()}>
            <Icon name="x" />
          </button>
        </header>
        <div class="body">{props.children}</div>
      </div>
    </Dialog>
  )
}

/// A question that stops everything until it's answered: the one the page has, asked through `confirm`. It keeps
/// showing the question as it fades out. Its two buttons sit side by side while both fit, and otherwise one over the
/// other, the action above Cancel, as iOS lays them out (ui.css).
function Alert(props: { question?: Question; open: boolean; onResult: (yes: boolean, reset: false) => void }) {
  let confirmed = false
  return (
    <Dialog class="ui-dialog alert" role="alertdialog" labelledby="ui-alert-heading" describedby="ui-alert-text" open={props.open} onClose={() => props.onResult(confirmed, (confirmed = false))}>
      <div class="panel">
        <h2 id="ui-alert-heading">{props.question?.heading}</h2>
        <Show when={props.question?.text}>
          <p id="ui-alert-text">{props.question?.text}</p>
        </Show>
        <footer>
          <Button variant="outline" autofocus onClick={(event) => ((confirmed = false), event.currentTarget.closest("dialog")!.close())}>
            {messages.cancel}
          </Button>
          <Button variant={props.question?.destructive ? "destructive" : "default"} onClick={(event) => ((confirmed = true), event.currentTarget.closest("dialog")!.close())}>
            {props.question?.action || messages.ok}
          </Button>
        </footer>
      </div>
    </Dialog>
  )
}

/** What `confirm` asks. */
export interface Question {
  heading: string
  text?: string
  action?: string
  destructive?: boolean
}

let ask: ((question: Question) => Promise<boolean>) | undefined

/// `await confirm({ heading: "Delete this note?", text: "It can't be brought back.", action: "Delete", destructive: true })`:
/// asks before doing what can't be undone. → whether `action` (OK unless said) was chosen; Cancel, Escape and a
/// press outside are no.
export function confirm(question: Question): Promise<boolean> {
  if (!ask) {
    const [question, setQuestion] = createSignal<Question>()
    const [open, setOpen] = createSignal(false)
    let resolve: (yes: boolean) => void
    const host = document.body.appendChild(document.createElement("div"))
    render(() => <Alert question={question()} open={open()} onResult={(yes) => (setOpen(false), resolve(yes))} />, host)
    ask = (question) => new Promise<boolean>((done) => ((resolve = done), setQuestion(question), setOpen(true)))
  }
  return ask(question)
}

let detail: ((text: string) => void) | undefined

/// Shows all of a tool's error in a sheet: what a person reads of it, then the rest and Copy. What a toast's Details
/// opens.
export function details(text: string): void {
  if (!detail) {
    const [shown, setShown] = createSignal<string>()
    const [open, setOpen] = createSignal(false)
    const host = document.body.appendChild(document.createElement("div"))
    render(
      () => (
        <Sheet heading={messages.details} open={open()} onClose={() => setOpen(false)}>
          <Show when={shown()} keyed>
            {(text) => (
              <>
                <p>{readable(text)}</p>
                <Details text={text} />
              </>
            )}
          </Show>
        </Sheet>
      ),
      host,
    )
    detail = (text) => (setShown(text), setOpen(true))
  }
  detail(text)
}
