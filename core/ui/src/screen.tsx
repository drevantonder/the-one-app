import { createContext, createEffect, createSignal, flush, onSettled, Show, useContext } from "solid-js"
import { covers } from "./bridge.ts"
import { Icon } from "./button.tsx"
import { reduced, messages } from "./shared.ts"
import type { JSX } from "@solidjs/web"

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shown = false
addEventListener("popstate", (event) => {
  shown = event.hasUAVisualTransition
  setTimeout(() => (shown = false))
}, true)

let moving: ViewTransition | undefined

/// `go(() => setOpen(note))`: changes which screen shows, as a move. `change` puts the new screen in place, and the
/// browser slides to it from what was there (its View Transitions): in over it, or with `back`, back out from over
/// it. The browser takes its picture of the new screen once it's drawn, with what it was still loading, as the View
/// Transitions guide has it: its picture of the old one stays on screen until then. `part` is what moves, when it
/// isn't the whole page. For whoever asked for less motion one fades into the other, and where the browser can't,
/// it's a plain swap. From one page to another the browser does this by itself (ui.css).
export function go(change: () => void, { back = false, part }: { back?: boolean; part?: HTMLElement } = {}): void {
  if (!globalThis.ViewTransition || !("types" in ViewTransition.prototype) || shown) return void change()
  part?.style.setProperty("view-transition-name", "ui-screen")
  // What's still fading in what moves (a row let go of a moment ago) is finished first, so the move takes the screen
  // as it settles. Chromium's page crashed taking it mid-fade. What never ends, a spinner, keeps going.
  const fading = part ? part.getAnimations({ subtree: true }) : document.getAnimations()
  for (const fade of fading) if (fade.effect?.getComputedTiming().endTime !== Infinity) fade.finish()
  const update = () => new Promise<void>((settled) => (change(), onSettled(settled), flush()))
  const move = (moving = document.startViewTransition({ update, types: reduced.matches ? [] : [back ? "ui-back" : "ui-forward", part ? "ui-part" : "ui-page"] }))
  move.finished.finally(() => moving === move && part?.style.removeProperty("view-transition-name"))
}

// Back to a page left earlier is the device's to show. The browser would slide it in as if it were new.
addEventListener("pageswap", ({ activation, viewTransition }) => {
  if (activation?.navigationType === "traverse" && activation.from && activation.entry.index < activation.from.index) viewTransition?.skipTransition()
})

// The page was loaded again with a split's detail open: what was in it is gone, so each starts shut.
if (history.state?.uiSplit) history.replaceState({ ...history.state, uiSplit: 0 }, "")

/// What closes what a split has open beside its list, for the list's Back button, while something is.
const Opened = createContext<() => (() => void) | undefined>(() => undefined)

/// `<Screen heading="Notes" action={<Button>Add</Button>}>...</Screen>`: a whole view of an app: a bar across its
/// top that stays as it scrolls, with its `heading` and what it does (`action`: its one main button, or a menu),
/// then what's in it, one part under another. With `back` the bar starts with a Back button, which calls `onBack`,
/// or goes back in the browser's history as the device's own back gesture does. A split's list closes what's open
/// beside it first. An app's first screen has no Back: the window's rail, its title bar's Home on a computer, and the
/// device's back leave the app. Where the window's breadcrumb names the app, its first screen's heading gives way to it
/// (ui.css).
export function Screen(props: { heading?: JSX.Element; action?: JSX.Element; back?: boolean; onBack?: () => void; class?: JSX.ClassValue; children?: JSX.Element }) {
  const opened = useContext(Opened)
  const back = () => {
    const close = opened()
    if (close) close()
    else if (props.onBack) props.onBack()
    else history.back()
  }
  return (
    <div class={["ui-screen", props.class]}>
      <header>
        <Show when={props.back}>
          <button class="back press ring" aria-label={messages.back} onClick={back}>
            <Icon name="caret-left" along />
          </button>
        </Show>
        <h1>{props.heading}</h1>
        {props.action}
      </header>
      <main>{props.children}</main>
    </div>
  )
}

/// `<Section heading="This week">...</Section>`: a part of a screen under a small grey heading.
export function Section(props: { heading?: JSX.Element; children?: JSX.Element }) {
  return (
    <section class="ui-section">
      <Show when={props.heading}>
        <h2>{props.heading}</h2>
      </Show>
      {props.children}
    </section>
  )
}

/// `<Split open={!!note()} onClose={() => setNote()} detail={<Screen heading={note().text} back>...</Screen>}><Screen heading="Notes">...</Screen></Split>`:
/// a list and what it opens (`detail`): beside it where there's room, pushed over it where there isn't, while
/// `open`. Each side scrolls by itself, and the split is as tall as the window unless the page says. While it's
/// open the device's own back, Escape, and the detail's Back button close it, or beside its list, where a detail has
/// none, the list's: it calls `onClose`, and the app sets `open` false.
export function Split(props: { open?: boolean; onClose?: () => void; detail?: JSX.Element; class?: JSX.ClassValue; children?: JSX.Element }) {
  let split!: HTMLDivElement
  let panes!: HTMLDivElement
  // What's drawn as open, which follows `open` as a move.
  const [open, setOpen] = createSignal(false)
  // Whether the detail is beside the list, which the room it has says (ui.css).
  const [beside, setBeside] = createSignal(false)
  let byDevice = false
  // How deep in the history its opening is, among the splits open in one another (a sidebar's, then a list's):
  // each is one step, so the device's back closes the innermost first.
  let depth = 0
  const deep = () => Number(history.state?.uiSplit ?? 0)
  const close = () => props.onClose?.()

  // The device's back, or forward: what's open is what the history says.
  const travelled = (event: PopStateEvent) => {
    byDevice = event.hasUAVisualTransition
    if (props.open && deep() < depth) props.onClose?.()
  }
  onSettled(() => {
    addEventListener("popstate", travelled)
    const resized = new ResizeObserver(() => setBeside(!!getComputedStyle(panes).getPropertyValue("--beside")))
    resized.observe(split)
    // Escape closes it, unless it's for a sheet or a menu over it, which closes that, or a split open in it, which
    // closes first.
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.defaultPrevented && props.open && deep() <= depth && !document.querySelector("dialog[open], :popover-open")) close()
    }
    addEventListener("keydown", escape)
    return () => (removeEventListener("popstate", travelled), removeEventListener("keydown", escape), resized.disconnect())
  })

  // Beside its list the device's back closes it as it does a sheet, not as a step back through the history, which a
  // device's gesture shows as the whole page going back. Pushed over the list, it's that step.
  createEffect(() => !!props.open && beside(), (now) => (now ? covers(close) : undefined))

  createEffect(() => !!props.open, (now, was) => {
    // It starts as it's written, and the device has already shown its own back gesture's move.
    const still = was === undefined || byDevice || getComputedStyle(panes).getPropertyValue("--beside")
    byDevice = false
    if (still) setOpen(now)
    else go(() => setOpen(now), { back: !now, part: split })
    // Opening it is one step in the history, so that the device's back is what closes it.
    if (now && !depth) history.pushState({ ...history.state, uiSplit: (depth = deep() + 1) }, "")
    if (!now && depth) {
      if (deep() >= depth) history.back()
      depth = 0
    }
  })

  return (
    <div class={["ui-split", props.class]} ref={split}>
      <div class="panes" ref={panes} data-open={open() ? "" : undefined}>
        <div class="list">
          <Opened value={() => (props.open ? close : undefined)}>{props.children}</Opened>
        </div>
        <div class="detail">{props.detail}</div>
      </div>
    </div>
  )
}
