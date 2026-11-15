import { createEffect, createSignal, onSettled, Show } from "solid-js"
import { Icon } from "./button.jsx"
import { reduced, said } from "./shared.js"

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shown = false
addEventListener("popstate", (event) => {
  shown = !!event.hasUAVisualTransition
  setTimeout(() => (shown = false))
}, true)

let moving

/// `go(() => setOpen(note))`: changes which screen shows, as a move. `change` puts the new screen in place, and the
/// browser slides to it from what was there (its View Transitions): in over it, or with `back`, back out from over
/// it. Ask the server first: the page holds still until `change` is done. `part` is what moves, when it isn't the
/// whole page. Where the browser can't, or the user asked for no motion, it's a plain swap. From one page to
/// another the browser does this by itself (ui.css).
export function go(change, { back = false, part } = {}) {
  if (!globalThis.ViewTransition || !("types" in ViewTransition.prototype) || reduced.matches || shown) return void change()
  part?.style.setProperty("view-transition-name", "ui-screen")
  // What's still fading in what moves (a row let go of a moment ago) is finished first, so the move takes the screen
  // as it settles. Chromium's page crashed taking it mid-fade. What never ends, a spinner, keeps going.
  const fading = part ? part.getAnimations({ subtree: true }) : document.getAnimations()
  for (const fade of fading) if (fade.effect?.getComputedTiming().endTime !== Infinity) fade.finish()
  const move = (moving = document.startViewTransition({ update: change, types: [back ? "ui-back" : "ui-forward", part ? "ui-part" : "ui-page"] }))
  move.finished.finally(() => moving === move && part?.style.removeProperty("view-transition-name"))
}

// Back to a page left earlier is the device's to show. The browser would slide it in as if it were new.
addEventListener("pageswap", ({ activation, viewTransition }) => {
  if (activation?.navigationType === "traverse" && activation.entry.index < activation.from.index) viewTransition?.skipTransition()
})

/// `<Screen heading="Notes" action={<Button>Add</Button>}>...</Screen>`: a whole view of an app: a bar across its
/// top that stays as it scrolls, with its `heading` and what it does (`action`: its one main button, or a menu),
/// then what's in it, one part under another. With `back` the bar starts with a Back button, which calls `onBack`,
/// or goes back in the browser's history as the device's own back gesture does.
export function Screen(props) {
  return (
    <div class={["ui-screen", props.class]}>
      <header>
        <Show when={props.back}>
          <button class="back press ring" aria-label={said.back} onClick={() => (props.onBack ? props.onBack() : history.back())}>
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
export function Section(props) {
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
/// open the device's own back, and the detail's Back button, close it: it calls `onClose`, and the app sets
/// `open` false. Beside its list a detail has no Back button.
export function Split(props) {
  let split
  let panes
  // What's drawn as open, which follows `open` as a move.
  const [open, setOpen] = createSignal(false)
  let byDevice = false

  // The device's back, or forward: what's open is what the history says.
  const travelled = (event) => {
    byDevice = event.hasUAVisualTransition
    if (!history.state?.uiSplit && props.open) props.onClose?.()
  }
  onSettled(() => {
    addEventListener("popstate", travelled)
    // The page was loaded again with the detail open: what was in it is gone, so it starts shut.
    if (history.state?.uiSplit && !props.open) history.replaceState({ ...history.state, uiSplit: false }, "")
    return () => removeEventListener("popstate", travelled)
  })

  createEffect(() => !!props.open, (now, was) => {
    // It starts as it's written, and the device has already shown its own back gesture's move.
    const still = was === undefined || byDevice || getComputedStyle(panes).getPropertyValue("--beside")
    byDevice = false
    if (still) setOpen(now)
    else go(() => setOpen(now), { back: !now, part: split })
    // Opening it is one step in the history, so that the device's back is what closes it.
    if (now && !history.state?.uiSplit) history.pushState({ ...history.state, uiSplit: true }, "")
    if (!now && history.state?.uiSplit) history.back()
  })

  return (
    <div class={["ui-split", props.class]} ref={split}>
      <div class="panes" ref={panes} data-open={open() ? "" : undefined}>
        <div class="list">{props.children}</div>
        <div class="detail">{props.detail}</div>
      </div>
    </div>
  )
}
