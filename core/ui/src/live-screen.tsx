import { createSignal, onSettled, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { assign, createActor, fromCallback, fromPromise, setup } from "xstate"
import { Button, Icon } from "./button.tsx"
import { messages, touch, whole } from "./shared.ts"
import { Problem, Status } from "./states.tsx"

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
// How long it shows pictures before it tries video again, each time it's tried: as a video call does, more slowly
// the longer it's been.
const RETRIES = [5_000, 10_000, 30_000, 60_000]
// Pictures that didn't come, one after another, before the screen is said to be out of reach: a hold-up of a few
// seconds isn't a problem.
const LASTING = 3

/** What's sent to a screen: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>

/** What the person does on a stream, as its data channel carries it to the screen. */
type Input = { t: "move"; x: number; y: number } | { t: "button"; b: number; down: boolean; mods: string[] } | { t: "key"; key: string; mods: string[] } | { t: "text"; text: string }

const json = (answer: unknown) => (typeof answer === "string" ? JSON.parse(answer) : answer)

/// `<LiveScreen label="The browser" screen={(inputs) => call("chrome_screen", { session, ...inputs })} />`: a screen
/// as it is now, as a remote desktop app shows one, which you click, scroll and type on. It's live video over WebRTC:
/// `screen` is called with `stream`, for the ICE servers and then with its offer, for the screen's answer, and what's
/// done on it goes to the screen over the stream's data channel. Video lost is connected to again. Where it can't
/// connect it shows a picture each second meanwhile, and says so, and tries video again, more slowly the longer it's
/// been: `screen` is then called with Computer Use's actions (`screenshot`, which answers with the picture, `click`,
/// `scroll`, `type`, `key`), in order. A screen that gives no picture either for a few seconds is a problem, with Try
/// Again. It's as wide as its place and as tall as fits, and the screen is resized to match it (`resize`), so its text
/// is as big as the page's. Full Screen (F11, or ⌃⌘F on a Mac) fills the display with it, and on a phone a tap is what
/// opens it. `watch` only looks: nothing is sent to it, and it keeps its size. Until there's a `screen` (one that's
/// still starting) its place shows what it's given instead, its `Steps` most often.
export function LiveScreen(props: { label: string; screen?: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean; children?: JSX.Element }) {
  return (
    <Show when={props.screen} keyed fallback={<div class="ui-live-screen"><div class={["view", { watch: props.watch }]}>{props.children}</div></div>}>
      {(screen) => <Connected label={props.label} screen={screen} watch={props.watch} />}
    </Show>
  )
}

/** What happens to a live screen's connection, as its machine is told it. */
type Happened = { type: "lost" } | { type: "looked" } | { type: "unseen"; why: string | Error } | { type: "again" }

function Connected(props: { label: string; screen: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean }) {
  // What it's of, for good: what's still on its way once it's gone goes to it.
  const screen = props.screen
  const watch = props.watch
  const [picture, setPicture] = createSignal<string>()
  // Full screen: the browser's ("display"), or filling the window where a page can't have that.
  const [full, setFull] = createSignal<"display" | "window">()
  const [size, setSize] = createSignal<[number, number]>()
  let view!: HTMLDivElement
  let video!: HTMLVideoElement
  let img!: HTMLImageElement
  let field!: HTMLInputElement
  let queue: Promise<unknown> = Promise.resolve()
  let last: ScreenInputs | undefined
  let peer: RTCPeerConnection | undefined
  let channel: RTCDataChannel | undefined

  /// Connects to the screen's stream, and once it's lost tells the machine. `signal` is the machine leaving the state
  /// that connects, which closes what this made.
  const connect = async (signal: AbortSignal) => {
    peer?.close()
    const pc = (peer = new RTCPeerConnection({ ...json(await screen({ action: "stream" })), iceTransportPolicy: "relay" }))
    signal.addEventListener("abort", () => pc === peer && pc.connectionState !== "connected" && pc.close())
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
    const lost = () => pc === peer && connection.send({ type: "lost" })
    data.onclose = lost
    pc.onconnectionstatechange = () => ["failed", "closed"].includes(pc.connectionState) && lost()
  }

  /// The connection: video, connected to again when it's lost (as it is when the screen is resized), or else a
  /// picture each second while video is tried again, or else out of reach.
  const machine = setup({
    types: {} as { context: { tries: number; unseen: number; why?: string | Error }; events: Happened },
    actors: {
      connect: fromPromise(({ signal }) => connect(signal)),
      // A picture every `every` ms, while the page is seen: each one that comes, or doesn't, is told.
      look: fromCallback<Happened, { every: number }>(({ sendBack, input }) => {
        const take = () =>
          document.visibilityState === "visible" &&
          screen({ action: "screenshot" }).then(
            (data) => (setPicture(data as string), sendBack({ type: "looked" })),
            (error) => sendBack({ type: "unseen", why: error }),
          )
        take()
        const timer = setInterval(take, input.every)
        return () => clearInterval(timer)
      }),
    },
    // Why it isn't video: what went wrong connecting, or getting a picture.
    actions: { why: assign({ why: ({ event }) => ("why" in event ? event.why : ((event as { error?: Error }).error ?? "")) }) },
    delays: { retry: ({ context }) => RETRIES[Math.min(context.tries, RETRIES.length - 1)] },
    guards: { video: () => "RTCPeerConnection" in window, lasting: ({ context }) => context.unseen + 1 >= LASTING },
  }).createMachine({
    context: { tries: 0, unseen: 0 },
    initial: "connecting",
    states: {
      connecting: {
        always: { guard: () => !("RTCPeerConnection" in window), target: "pictures", actions: assign({ why: "This device's browser has no WebRTC" }) },
        invoke: { src: "connect", onDone: "live", onError: { target: "pictures", actions: "why" } },
      },
      live: {
        id: "live",
        entry: assign({ tries: 0, unseen: 0, why: undefined }),
        on: { lost: "reconnecting" },
      },
      reconnecting: {
        invoke: { src: "connect", onDone: "live", onError: { target: "pictures", actions: "why" } },
      },
      pictures: {
        invoke: { src: "look", input: { every: 1000 } },
        on: {
          looked: { actions: assign({ unseen: 0 }) },
          unseen: [{ guard: "lasting", target: "unreachable", actions: "why" }, { actions: assign({ unseen: ({ context }) => context.unseen + 1 }) }],
        },
        initial: "waiting",
        states: {
          waiting: { after: { retry: { guard: "video", target: "trying" } } },
          trying: { invoke: { src: "connect", onDone: "#live", onError: { target: "waiting", actions: assign({ tries: ({ context }) => context.tries + 1 }) } } },
        },
      },
      // Still looked at now and then, so it comes back by itself once the screen does.
      unreachable: {
        invoke: { src: "look", input: { every: 5000 } },
        on: { looked: "connecting", again: { target: "connecting", actions: assign({ tries: 0, unseen: 0 }) } },
      },
    },
  })
  const connection = createActor(machine)
  const [state, setState] = createSignal(connection.getSnapshot())
  connection.subscribe(setState)
  const live = () => state().matches("live")
  // The video stays while it's connected to again, under what says so.
  const showsVideo = () => state().matches("live") || state().matches("reconnecting")

  const send = (input: Input) => channel?.readyState === "open" && channel.send(JSON.stringify(input))

  const look = () =>
    screen({ action: "screenshot" }).then(
      (data) => setPicture(data as string),
      () => {},
    )

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
      .catch(() => {})
      .then(look)
  }

  // Where the screen is drawn in the view: as big as fits, in the middle, in the video while it's shown (reconnecting
  // too), or else in the picture. Before the first picture nothing is drawn, so there's nowhere to point.
  const at = (event: MouseEvent) => {
    const drawn = showsVideo() ? video : picture() && img
    if (!size() || !drawn) return
    const [width, height] = size()!
    const box = drawn.getBoundingClientRect()
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
    resizing = setTimeout(() => (queue = queue.then(async () => (await resize()) && (live() ? connection.send({ type: "lost" }) : look()))), 250)
  }

  onSettled(() => {
    // Its first size is the view's, so it isn't connected to twice.
    if (watch) connection.start()
    else queue = queue.then(resize).then(() => connection.start())
    const sized = new ResizeObserver(fit)
    if (!watch) sized.observe(view)
    const left = () => setFull(document.fullscreenElement === view ? "display" : undefined)
    document.addEventListener("fullscreenchange", left)
    return () => {
      connection.stop()
      peer?.close()
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
          hidden={!showsVideo()}
          aria-label={props.label}
          autoplay
          muted
          playsinline
          disablepictureinpicture
          onResize={() => setSize([video.videoWidth, video.videoHeight])}
        />
        <Show when={!showsVideo() && picture()}>
          <img ref={img} src={picture()} alt={props.label} draggable={false} onLoad={() => setSize([img.naturalWidth, img.naturalHeight])} />
        </Show>
        <Show when={state().matches("connecting") || (state().matches("pictures") && !size())}>
          <Status heading="Connecting…" />
        </Show>
        <Show when={state().matches("reconnecting")}>
          <Status over heading="Reconnecting…" />
        </Show>
        <Show when={state().matches("unreachable")}>
          <Problem
            heading="Can't reach this screen"
            text={state().context.why}
            action={
              <button type="button" class="ui-button press ring" data-variant="outline" onClick={(event) => (event.stopPropagation(), connection.send({ type: "again" }))}>
                Try Again
              </button>
            }
          />
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
          <button type="button" class="ui-button press ring leave" data-variant="secondary" data-size="icon" aria-label={messages.exitFullScreen} onClick={(event) => (event.stopPropagation(), toggle())}>
            <Icon name="corners-in" />
          </button>
        </Show>
      </div>
      <div class="bar">
        <Show when={state().matches("pictures")}>
          <p class="note" title={state().context.why && whole(state().context.why!)}>
            Live video isn't connecting, so this updates each second.
          </p>
        </Show>
        <button type="button" class="ui-button press ring" data-variant="ghost" data-size="icon" aria-label={messages.fullScreen} title={`${messages.fullScreen} (${mac ? "⌃⌘F" : "F11"})`} onClick={toggle}>
          <Icon name="corners-out" />
        </button>
      </div>
    </div>
  )
}
