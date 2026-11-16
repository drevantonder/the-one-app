import { createSignal, onSettled, Show } from "solid-js"
import { Button, Icon } from "./button.tsx"
import { said, touch } from "./shared.ts"
import { Problem } from "./states.tsx"

// Keys as the browser names them, as xdotool does, which is how a screen is told them.
const KEYS: Record<string, string> = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Prior", PageDown: "Next" }
// The sizes a screen is resized to: no narrower than Chromium draws its window (500), and no bigger than Computer
// Use's screens take.
const SIZES = { width: [500, 7680], height: [240, 4320] }
const mac = /Mac|iPhone|iPad/.test(navigator.platform)
// WebKitGTK, the Linux app's web view, leaves full screen on F, as a shortcut of its own, before the page hears it.
const webKitGtk = /Linux/.test(navigator.userAgent) && !/Chrome|Firefox|Android/.test(navigator.userAgent)

/** What's sent to a screen: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>

/// `<LiveScreen label="The browser" screen={(inputs) => call("chrome_screen", { session, ...inputs })} />`: a screen
/// as it is now, as a remote desktop app shows one, which you click, scroll and type on: `screen` is called with
/// Computer Use's actions (`screenshot`, which answers with the picture, `click`, `scroll`, `type`, `key`, `resize`),
/// in order. It's as wide as its place and as tall as fits, and the screen is resized to match it, so its text is
/// as big as the page's. Full Screen (F11, or ⌃⌘F on a Mac) fills the display with it, and on a phone a tap is
/// what opens it. `watch` only looks: nothing is sent to it, and it keeps its size.
export function LiveScreen(props: { label: string; screen: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean }) {
  // What it's of, for good: what's still on its way once it's gone goes to it.
  const screen = props.screen
  const watch = props.watch
  const [picture, setPicture] = createSignal<string>()
  const [problem, setProblem] = createSignal<string>()
  // Full screen: the browser's ("display"), or filling the window where a page can't have that.
  const [full, setFull] = createSignal<"display" | "window">()
  const [size, setSize] = createSignal<[number, number]>()
  let view!: HTMLDivElement
  let img!: HTMLImageElement
  let field!: HTMLInputElement
  let queue: Promise<unknown> = Promise.resolve()
  let last: ScreenInputs | undefined

  const look = () =>
    screen({ action: "screenshot" })
      .then((data) => (setPicture(data as string), setProblem()))
      .catch((error) => setProblem(error.message))

  /// Sends what's done to the screen after what's already on its way, and takes it again after. `merge` folds it into
  /// the last one still waiting, when it can (typing, scrolling the same way).
  const act = (inputs: ScreenInputs, merge?: (was: ScreenInputs, now: ScreenInputs) => boolean) => {
    if (merge && last?.action === inputs.action && merge(last, inputs)) return
    const job = (last = { ...inputs })
    queue = queue
      .then(() => {
        if (last === job) last = undefined
        return screen(job)
      })
      .then(look)
      .catch((error) => setProblem(error.message))
  }

  // Where the picture is drawn in the view: as big as fits, in the middle.
  const at = (event: MouseEvent) => {
    if (!size()) return
    const [width, height] = size()!
    const box = img.getBoundingClientRect()
    const scale = Math.min(box.width / width, box.height / height)
    const x = (event.clientX - box.left - (box.width - width * scale) / 2) / scale
    const y = (event.clientY - box.top - (box.height - height * scale) / 2) / scale
    return { x: Math.round(Math.min(Math.max(x, 0), width - 1)), y: Math.round(Math.min(Math.max(y, 0), height - 1)) }
  }

  // The screen is resized to the view, in CSS pixels: Chromium on it draws at one pixel each and doesn't take a new
  // scale while it runs, so a screen of the device's pixels would draw its text smaller than the page's.
  let resizing: ReturnType<typeof setTimeout>
  let fixed = false
  const fit = () => {
    clearTimeout(resizing)
    resizing = setTimeout(() => {
      // A view smaller than the least a screen takes keeps its shape, and is drawn a little smaller.
      const up = Math.max(1, SIZES.width[0] / view.clientWidth, SIZES.height[0] / view.clientHeight)
      const width = Math.min(Math.round(view.clientWidth * up), SIZES.width[1])
      const height = Math.min(Math.round(view.clientHeight * up), SIZES.height[1])
      if (fixed || !size() || (width === size()![0] && height === size()![1])) return
      // A screen whose display can't be resized (a desktop container made before it could) is drawn scaled instead.
      queue = queue.then(() => screen({ action: "resize", width, height }).then(look, () => (fixed = true)))
    }, 250)
  }

  onSettled(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    const again = async () => {
      if (document.visibilityState === "visible") await look()
      if (!stopped) timer = setTimeout(again, 1000)
    }
    again()
    const sized = new ResizeObserver(fit)
    if (!watch) sized.observe(view)
    const left = () => setFull(document.fullscreenElement === view ? "display" : undefined)
    document.addEventListener("fullscreenchange", left)
    return () => ((stopped = true), clearTimeout(timer), sized.disconnect(), document.removeEventListener("fullscreenchange", left))
  })

  // Full Screen is the browser's, on the view. Where a page can't have it (an iPhone's Safari, a web view that
  // doesn't allow it, or WebKitGTK, where typing F would leave it) the view fills the window instead.
  const toggle = async () => {
    if (document.fullscreenElement) return document.exitFullscreen()
    if (full()) return setFull()
    if (webKitGtk) setFull("window")
    else await view.requestFullscreen({ navigationUI: "hide" }).catch(() => setFull("window"))
    if (!watch) field.focus()
  }

  const type = (text: string) => text && act({ action: "type", text }, (was, now) => ((was.text += now.text as string), true))
  const key = (event: KeyboardEvent) => {
    if (event.key === "F11" || (mac && event.ctrlKey && event.metaKey && event.key.toLowerCase() === "f")) return event.preventDefault(), toggle()
    const named = KEYS[event.key] ?? (event.key.length === 1 ? event.key.toLowerCase() : undefined)
    const modified = event.ctrlKey || event.metaKey || event.altKey
    // Pasting is the field's own, and comes as what's typed.
    if (!named || (!modified && event.key.length === 1) || (modified && event.key.toLowerCase() === "v")) return
    event.preventDefault()
    const held = [event.ctrlKey && "ctrl", event.altKey && "alt", event.shiftKey && "shift", event.metaKey && "super"].filter(Boolean)
    act({ action: "key", keys: [...held, named].join("+") })
  }
  const click = (event: MouseEvent, button = "left") => {
    // On a phone the view is small: a tap opens it full screen, where taps are clicks.
    if (touch() && !full()) return toggle()
    const point = at(event)
    if (watch || !point) return
    act({ action: "click", ...point, button })
    field.focus()
  }

  return (
    <div class="ui-live-screen">
      <div
        ref={view}
        class={["view", { full: full() === "window", watch }]}
        style={{ "aspect-ratio": watch && size() ? `${size()![0]} / ${size()![1]}` : undefined }}
        onClick={(event) => click(event)}
        onContextMenu={(event) => !watch && (event.preventDefault(), click(event, "right"))}
        onWheel={(event) => {
          const point = at(event)
          if (watch || !event.deltaY || !point) return
          event.preventDefault()
          act({ action: "scroll", ...point, direction: event.deltaY > 0 ? "down" : "up", amount: 1 }, (was, now) => was.direction === now.direction && ((was.amount = Math.min(50, (was.amount as number) + 1)), true))
        }}
      >
        <Show when={picture()}>
          <img ref={img} src={picture()} alt={props.label} draggable={false} onLoad={() => (setSize([img.naturalWidth, img.naturalHeight]), fit())} />
        </Show>
        <Show when={problem()}>
          <Problem heading="Couldn't reach this screen" text={problem()} />
        </Show>
        <Show when={!watch}>
          <input
            ref={field}
            class="keys"
            aria-label={`Type in ${props.label}`}
            autocomplete="off"
            autocapitalize="off"
            spellcheck={false}
            onKeyDown={key}
            onInput={(event) => (type(event.target.value), (event.target.value = ""))}
          />
        </Show>
        {/* Full screen in the window, or under a finger, has no Escape: it has its own way out. */}
        <Show when={full() === "window" || (full() && touch())}>
          <Button class="leave" variant="secondary" size="icon" aria-label={said.exitFullScreen} onClick={(event) => (event.stopPropagation(), toggle())}>
            <Icon name="corners-in" />
          </Button>
        </Show>
      </div>
      <div class="bar">
        <Button variant="ghost" size="icon" aria-label={said.fullScreen} title={`${said.fullScreen} (${mac ? "⌃⌘F" : "F11"})`} onClick={toggle}>
          <Icon name="corners-out" />
        </Button>
      </div>
    </div>
  )
}
