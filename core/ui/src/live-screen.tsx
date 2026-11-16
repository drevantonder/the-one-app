import { createSignal, onSettled, Show } from "solid-js"
import { Button, Icon } from "./button.tsx"
import { said, touch } from "./shared.ts"
import { Problem } from "./states.tsx"

// Keys as the browser names them, as X does, which is how a screen is told them.
const KEYS: Record<string, string> = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Prior", PageDown: "Next" }
// The sizes a screen is resized to: no narrower than Chrome draws its window (500), and no bigger than Computer Use's
// screens take.
const SIZES = { width: [500, 7680], height: [240, 4320] }
const mac = /Mac|iPhone|iPad/.test(navigator.platform)
// WebKitGTK, the Linux app's web view, leaves full screen on F, as a shortcut of its own, before the page hears it.
const webKitGtk = /Linux/.test(navigator.userAgent) && !/Chrome|Firefox|Android/.test(navigator.userAgent)
// How far a wheel turns for one of its steps, in pixels.
const STEP = 60

/** What's sent to a screen: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>

/** What the person does on a stream, as its data channel carries it to the screen. */
type Input = { t: "move"; x: number; y: number } | { t: "button"; b: number; down: boolean; mods: string[] } | { t: "key"; key: string; mods: string[] } | { t: "text"; text: string }

const json = (answer: unknown) => (typeof answer === "string" ? JSON.parse(answer) : answer)

/// `<LiveScreen label="The browser" screen={(inputs) => call("chrome_screen", { session, ...inputs })} />`: a screen
/// as it is now, as a remote desktop app shows one, which you click, scroll and type on. It's live video over WebRTC:
/// `screen` is called with `stream`, for the ICE servers and then with its offer, for the screen's answer, and what's
/// done on it goes to the screen over the stream's data channel. Where WebRTC can't connect it shows a picture each
/// second instead, and says so: `screen` is then called with Computer Use's actions (`screenshot`, which answers with
/// the picture, `click`, `scroll`, `type`, `key`), in order. It's as wide as its place and as tall as fits, and the
/// screen is resized to match it (`resize`), so its text is as big as the page's. Full Screen (F11, or ⌃⌘F on a Mac)
/// fills the display with it, and on a phone a tap is what opens it. `watch` only looks: nothing is sent to it, and it
/// keeps its size.
export function LiveScreen(props: { label: string; screen: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean }) {
  // What it's of, for good: what's still on its way once it's gone goes to it.
  const screen = props.screen
  const watch = props.watch
  const [live, setLive] = createSignal(false)
  const [picture, setPicture] = createSignal<string>()
  const [problem, setProblem] = createSignal<string>()
  // Why it shows pictures, not video.
  const [pictures, setPictures] = createSignal<string>()
  // Full screen: the browser's ("display"), or filling the window where a page can't have that.
  const [full, setFull] = createSignal<"display" | "window">()
  const [size, setSize] = createSignal<[number, number]>()
  let view!: HTMLDivElement
  let video!: HTMLVideoElement
  let img!: HTMLImageElement
  let field!: HTMLInputElement
  let queue: Promise<unknown> = Promise.resolve()
  let last: ScreenInputs | undefined
  let stopped = false
  let peer: RTCPeerConnection | undefined
  let channel: RTCDataChannel | undefined

  /// Connects to the screen's stream, again when it's lost, as it is when the screen is resized. One that can't
  /// connect at all has it show pictures.
  const connect = async () => {
    peer?.close()
    const pc = (peer = new RTCPeerConnection({ ...json(await screen({ action: "stream" })), iceTransportPolicy: "relay" }))
    pc.addTransceiver("video", { direction: "recvonly" })
    // Both ends make the data channel (id 0), so it opens with the connection, with nothing more to say.
    const data = (channel = pc.createDataChannel("input", { negotiated: true, id: 0 }))
    pc.ontrack = ({ track, receiver }) => {
      // As little as can be held back to play it smoothly: it's a screen you're using.
      receiver.jitterBufferTarget = 0
      video.srcObject = new MediaStream([track])
    }
    await pc.setLocalDescription()
    // The offer goes whole, with its candidates: once they're all gathered, or a moment after the first through
    // TURN, since each TURN server more is one more to wait on.
    await new Promise<void>((done) => {
      const timer = setTimeout(done, 5000)
      pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && done()
      pc.onicecandidate = ({ candidate }) => candidate?.type === "relay" && (clearTimeout(timer), setTimeout(done, 300))
    })
    const { answer } = json(await screen({ action: "stream", offer: pc.localDescription!.sdp }))
    await pc.setRemoteDescription({ type: "answer", sdp: answer })
    await new Promise<void>((connected, failed) => {
      const timer = setTimeout(() => failed(new Error("It didn't connect through TURN within 10 seconds")), 10_000)
      data.onopen = () => (clearTimeout(timer), connected())
    })
    if (pc !== peer) return
    setLive(true)
    setPictures()
    setProblem()
    const lost = () => pc === peer && !stopped && reconnect()
    data.onclose = lost
    pc.onconnectionstatechange = () => ["failed", "closed"].includes(pc.connectionState) && lost()
  }
  // Only the last try counts: one the screen's resizing cut short is let go.
  let tries = 0
  const reconnect = () => {
    const mine = ++tries
    return connect().catch((error) => mine === tries && !stopped && showPictures(error))
  }

  /// Shows a picture each second instead of video, saying why.
  const showPictures = (error: Error) => {
    peer?.close()
    peer = channel = undefined
    setLive(false)
    setPictures(error.message)
    look()
  }

  const send = (input: Input) => channel?.readyState === "open" && channel.send(JSON.stringify(input))

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

  // Where the screen is drawn in the view: as big as fits, in the middle.
  const at = (event: MouseEvent) => {
    if (!size()) return
    const [width, height] = size()!
    const box = (live() ? video : img).getBoundingClientRect()
    const scale = Math.min(box.width / width, box.height / height)
    const x = (event.clientX - box.left - (box.width - width * scale) / 2) / scale
    const y = (event.clientY - box.top - (box.height - height * scale) / 2) / scale
    return { x: Math.round(Math.min(Math.max(x, 0), width - 1)), y: Math.round(Math.min(Math.max(y, 0), height - 1)) }
  }

  // The screen is resized to the view, in CSS pixels: Chrome on it draws at one pixel each and doesn't take a new
  // scale while it runs, so a screen of the device's pixels would draw its text smaller than the page's. Its stream
  // ends as it's resized, and is connected to again at its new size.
  let resizing: ReturnType<typeof setTimeout>
  let fixed = false
  let requested = ""
  const resize = () => {
    // A view smaller than the least a screen takes keeps its shape, and is drawn a little smaller.
    const up = Math.max(1, SIZES.width[0] / view.clientWidth, SIZES.height[0] / view.clientHeight)
    const width = Math.min(Math.round(view.clientWidth * up), SIZES.width[1])
    const height = Math.min(Math.round(view.clientHeight * up), SIZES.height[1])
    if (fixed || requested === `${width}x${height}`) return
    requested = `${width}x${height}`
    // A screen whose display can't be resized (a desktop container made before it could) is drawn scaled instead.
    return screen({ action: "resize", width, height }).then(
      () => true,
      () => (fixed = true) && false,
    )
  }
  const fit = () => {
    clearTimeout(resizing)
    resizing = setTimeout(() => (queue = queue.then(async () => (await resize()) && (live() ? reconnect() : look()))), 250)
  }

  onSettled(() => {
    let timer: ReturnType<typeof setTimeout>
    // Pictures, each second, only while they're what's shown.
    const again = async () => {
      if (pictures() && document.visibilityState === "visible") await look()
      if (!stopped) timer = setTimeout(again, 1000)
    }
    if (!("RTCPeerConnection" in window)) showPictures(new Error("This device's browser has no WebRTC"))
    else if (watch) reconnect()
    // Its first size is the view's, so it isn't connected to twice.
    else queue = queue.then(resize).then(reconnect)
    timer = setTimeout(again, 1000)
    const sized = new ResizeObserver(fit)
    if (!watch) sized.observe(view)
    const left = () => setFull(document.fullscreenElement === view ? "display" : undefined)
    document.addEventListener("fullscreenchange", left)
    return () => {
      stopped = true
      peer?.close()
      clearTimeout(timer)
      sized.disconnect()
      document.removeEventListener("fullscreenchange", left)
    }
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

  // The modifiers held, as X names them: a Mac's ⌘ is Control, as Chrome's shortcuts on Linux take it.
  const mods = (event: KeyboardEvent | MouseEvent) =>
    [(event.ctrlKey || (mac && event.metaKey)) && "Control_L", event.altKey && "Alt_L", event.shiftKey && "Shift_L", !mac && event.metaKey && "Super_L"].filter(Boolean) as string[]

  const type = (text: string) => {
    if (!text) return
    if (live()) send({ t: "text", text })
    else act({ action: "type", text }, (was, now) => ((was.text += now.text as string), true))
  }
  const key = (event: KeyboardEvent) => {
    if (event.key === "F11" || (mac && event.ctrlKey && event.metaKey && event.key.toLowerCase() === "f")) return event.preventDefault(), toggle()
    const named = KEYS[event.key] ?? (event.key.length === 1 ? event.key.toLowerCase() : undefined)
    const modified = event.ctrlKey || event.metaKey || event.altKey
    // Pasting is the field's own, and comes as what's typed.
    if (!named || (!modified && event.key.length === 1) || (modified && event.key.toLowerCase() === "v")) return
    event.preventDefault()
    if (live()) return send({ t: "key", key: named, mods: mods(event) })
    const held = mods(event).map((mod) => ({ Control_L: "ctrl", Alt_L: "alt", Shift_L: "shift", Super_L: "super" })[mod])
    act({ action: "key", keys: [...held, named].join("+") })
  }

  // The mouse, over the stream: where it is as it moves, at most once a frame, and its buttons as they go down and up.
  let moved: { x: number; y: number } | undefined
  const move = (event: PointerEvent) => {
    const point = at(event)
    if (!live() || watch || !point) return
    if (!moved) requestAnimationFrame(() => (send({ t: "move", ...moved! }), (moved = undefined)))
    moved = point
  }
  const button = (event: PointerEvent, down: boolean) => {
    const point = at(event)
    if (!live() || watch || (touch() && !full()) || !point) return
    // What's typed goes to the field, focused once the press is over, as the browser moves focus as it goes down.
    if (down) view.setPointerCapture(event.pointerId)
    else field.focus({ preventScroll: true })
    send({ t: "move", ...point })
    send({ t: "button", b: [1, 2, 3][event.button] ?? 1, down, mods: mods(event) })
  }
  const click = (event: MouseEvent, kind = "left") => {
    // On a phone the view is small: a tap opens it full screen, where taps are clicks.
    if (touch() && !full()) return toggle()
    if (live()) return
    const point = at(event)
    if (watch || !point) return
    act({ action: "click", ...point, button: kind })
    field.focus()
  }
  let turned = { x: 0, y: 0 }
  const wheel = (event: WheelEvent) => {
    const point = at(event)
    if (watch || !point || (!event.deltaY && !event.deltaX)) return
    event.preventDefault()
    if (!live()) {
      if (!event.deltaY) return
      return act({ action: "scroll", ...point, direction: event.deltaY > 0 ? "down" : "up", amount: 1 }, (was, now) => was.direction === now.direction && ((was.amount = Math.min(50, (was.amount as number) + 1)), true))
    }
    // A wheel's steps, from however far it turned: a trackpad turns a little at a time, a mouse's wheel a line.
    const lines = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? STEP / 3 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? STEP * 10 : 1
    turned = { x: turned.x + event.deltaX * lines, y: turned.y + event.deltaY * lines }
    send({ t: "move", ...point })
    for (const [axis, less, more] of [["y", 4, 5], ["x", 6, 7]] as const) {
      while (Math.abs(turned[axis]) >= STEP) {
        const b = turned[axis] > 0 ? more : less
        send({ t: "button", b, down: true, mods: [] })
        send({ t: "button", b, down: false, mods: [] })
        turned[axis] -= Math.sign(turned[axis]) * STEP
      }
    }
  }

  return (
    <div class="ui-live-screen">
      <div
        ref={view}
        class={["view", { full: full() === "window", watch }]}
        style={{ "aspect-ratio": watch && size() ? `${size()![0]} / ${size()![1]}` : undefined }}
        onClick={(event) => click(event)}
        onContextMenu={(event) => !watch && (event.preventDefault(), click(event, "right"))}
        onPointerMove={move}
        onPointerDown={(event) => button(event, true)}
        onPointerUp={(event) => button(event, false)}
        onWheel={wheel}
      >
        <video
          ref={video}
          hidden={!live()}
          aria-label={props.label}
          autoplay
          muted
          playsinline
          disablepictureinpicture
          onResize={() => setSize([video.videoWidth, video.videoHeight])}
        />
        <Show when={!live() && picture()}>
          <img ref={img} src={picture()} alt={props.label} draggable={false} onLoad={() => setSize([img.naturalWidth, img.naturalHeight])} />
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
        <Show when={pictures()}>
          <p class="note" title={pictures()}>
            Live video couldn't connect, so this shows a picture each second.
          </p>
        </Show>
        <Button variant="ghost" size="icon" aria-label={said.fullScreen} title={`${said.fullScreen} (${mac ? "⌃⌘F" : "F11"})`} onClick={toggle}>
          <Icon name="corners-out" />
        </Button>
      </div>
    </div>
  )
}
