// A screen as it is now, as a remote desktop app shows one: written once on the drawing primitives (#primitives),
// the same on the DOM and on a native host. Its live video is the `Video` primitive, which a host draws, and its
// pictures, where video can't connect, are an `Image` the page draws itself.
import { createSignal, onCleanup, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { assign, createActor, fromCallback, fromPromise, setup } from "xstate"
import { Image, Portal, Pressable, Text, TextInput, Video, View } from "#primitives"
import { client, safeArea, touch, viewport, visible } from "#bridge"
import { BarEnd, Button, Icon } from "./button.tsx"
import { Problem, Status } from "./states.tsx"
import { fullScreen, mods, named } from "../drawing-primitives/input.ts"
import type { Ice } from "../drawing-primitives/renderer.ts"
import type { Keyed, Laid, Located, Pressed, Wheeled } from "../types/ui.d.ts"
import { messages } from "../words.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

// The sizes a screen is resized to: no narrower than Chrome draws its window (500), and no bigger than Computer Use's
// screens take.
const SIZES = { width: [500, 7680], height: [240, 4320] }
// How long it shows pictures before it tries video again, each time it's tried: as a video call does, more slowly
// the longer it's been.
const RETRIES = [5_000, 10_000, 30_000, 60_000]
// Pictures that didn't come, one after another, before the screen is said to be out of reach: a hold-up of a few
// seconds isn't a problem.
const LASTING = 3

const look = styles({
  root: { gap: tokens.grid[1], minWidth: 0 },
  // As wide as its place, and as tall as fits under the bar with a line or two left under it (its `height`).
  view: {
    position: "relative",
    overflow: "hidden",
    borderWidth: tokens.length.hairline,
    borderColor: tokens.colour.quinaryLabel,
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.background,
    touchAction: "pan-y",
  },
  // Full screen: the whole window, over the page.
  full: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, borderWidth: 0, borderRadius: 0, pointerEvents: "auto", touchAction: "auto" },
  fill: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, width: "100%", height: "100%" },
  surface: { backgroundColor: tokens.colour.quinaryLabel, cursor: "default" },
  hidden: { display: "none" },
  // What it shows in place of the screen: in its middle, on the view's grey, over what was there.
  state: { backgroundColor: tokens.colour.background },
  middle: { flexGrow: 1, alignItems: "center", justifyContent: "center", padding: tokens.length.gutter, backgroundColor: tokens.colour.quinaryLabel },
  // Where what's typed on its picture goes: unseen, but focused, so a phone shows its keyboard.
  keys: { position: "absolute", bottom: 0, start: 0, width: tokens.length.hairline, height: tokens.length.hairline, opacity: 0, pointerEvents: "none", font: tokens.type.field },
  leave: (top: number, end: number) => ({ position: "absolute", opacity: 0.85, top, end }),
  tall: (height: number) => ({ height }),
  bar: { flexDirection: "row", alignItems: "center", justifyContent: "flex-end", gap: tokens.grid[2] },
  note: { flexGrow: 1, flexShrink: 1, minWidth: 0, color: tokens.colour.secondaryLabel, font: tokens.type.small },
})

/** What's sent to a screen: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>

const json = (answer: unknown) => (typeof answer === "string" ? JSON.parse(answer) : answer)

/** A PNG's size, from its header (Computer Use's pictures are PNGs), as a `data:` address has it. */
function pngSize(url: string): [number, number] | undefined {
  const start = url.indexOf(",") + 1
  const bytes = atob(url.slice(start, start + 32))
  const word = (at: number) => ((bytes.charCodeAt(at) << 24) | (bytes.charCodeAt(at + 1) << 16) | (bytes.charCodeAt(at + 2) << 8) | bytes.charCodeAt(at + 3)) >>> 0
  return bytes.startsWith("\x89PNG") ? [word(16), word(20)] : undefined
}

/// `<LiveScreen label="The browser" screen={(inputs) => call("chrome_screen", { session, ...inputs })} />`: a screen
/// as it is now, as a remote desktop app shows one, which you click, scroll and type on. It's live video over WebRTC:
/// `screen` is called with `stream`, for the ICE servers and then with its offer, for the screen's answer, and what's
/// done on it goes to the screen over the stream's data channel. Video lost is connected to again. Where it can't
/// connect it shows a picture each second meanwhile, and says so, and tries video again, more slowly the longer it's
/// been: `screen` is then called with Computer Use's actions (`screenshot`, which answers with the picture, `click`,
/// `scroll`, `type`, `key`), in order. A screen that gives no picture either for a few seconds is a problem, with Try
/// Again. It's as wide as its place and as tall as fits, and the screen is resized to match it (`resize`), so its text
/// is as big as the page's. Full Screen (F11, or ⌃⌘F on a Mac) fills the window with it, and on a phone a tap is what
/// opens it. `watch` only looks: nothing is sent to it, and it keeps its size. Until there's a `screen` (one that's
/// still starting) its place shows what it's given instead, its `Steps` most often.
export function LiveScreen(props: { label: string; screen?: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean; children?: JSX.Element }) {
  return (
    <Show
      when={props.screen}
      keyed
      fallback={
        <View style={look.root}>
          <View style={[look.view, look.tall(Math.max(256, viewport().height - 240))]}>
            <View style={look.middle}>{props.children}</View>
          </View>
        </View>
      }
    >
      {(screen) => <Connected label={props.label} screen={screen} watch={props.watch} />}
    </Show>
  )
}

/** What happens to a live screen's connection, as its machine is told it: its video `lost`, or ended to connect again
 * at its new size (`resized`). */
type Happened = { type: "lost" } | { type: "resized" } | { type: "looked" } | { type: "unseen"; why: string | Error } | { type: "again" }

/** Why, as words: an error's message, or what was said. */
const said = (why: string | Error | undefined) => (why instanceof Error ? why.message : (why ?? ""))

function Connected(props: { label: string; screen: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean }) {
  // What it's of, for good: what's still on its way once it's gone goes to it.
  const screen = props.screen
  const watch = props.watch
  const [picture, setPicture] = createSignal<string>()
  const [full, setFull] = createSignal(false)
  // The screen's size, as its video or its picture has it, and its place's, where it's drawn.
  const [size, setSize] = createSignal<[number, number]>()
  const [place, setPlace] = createSignal<{ width: number; height: number }>()
  // The stream: the ICE servers it connects with, and the screen's answer to its offer, which the video is given.
  const [ice, setIce] = createSignal<Ice>()
  const [answer, setAnswer] = createSignal<string>()
  // Whether what's typed goes to its picture: set as it's pressed, and gone as the keyboard goes elsewhere.
  const [typing, setTyping] = createSignal(false)
  // What's in the field typed in, emptied as it's sent, which tells a host each time.
  const [typed, setTyped] = createSignal("", { equals: false })
  const [mac, setMac] = createSignal(false)
  client().then(({ mac }) => setMac(!!mac), () => {})
  let queue: Promise<unknown> = Promise.resolve()
  let last: ScreenInputs | undefined
  let gone = false
  // What the stream connecting is told by its video: its offer, and whether it connected.
  let connecting: { offer: (sdp: string) => void; state: (state: "connected" | "lost") => void } | undefined

  /// Connects to the screen's stream, through the video: the ICE servers, its offer, the screen's answer, and its data
  /// channel open. `signal` is the machine leaving the state that connects, which closes what this made.
  const connect = async (signal: AbortSignal) => {
    const servers = json(await screen({ action: "stream" }))
    signal.throwIfAborted()
    let timer: ReturnType<typeof setTimeout> | undefined
    try {
      await new Promise<void>((connected, failed) => {
        signal.addEventListener("abort", () => failed(signal.reason), { once: true })
        connecting = {
          offer: (sdp) =>
            void screen({ action: "stream", offer: sdp }).then((answered) => {
              setAnswer(json(answered).answer)
              timer = setTimeout(() => failed(new Error("It didn't connect through TURN within 10 seconds")), 10_000)
            }, failed),
          state: (state) => (state === "connected" ? connected() : failed(new Error("Its video was lost as it connected"))),
        }
        setAnswer(undefined)
        setIce({ ...servers, iceTransportPolicy: "relay" })
      })
    } catch (error) {
      // What didn't connect is closed.
      setIce(undefined)
      throw error
    } finally {
      clearTimeout(timer)
      connecting = undefined
    }
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
          visible() &&
          screen({ action: "screenshot" }).then(
            (data) => (shown(data as string), sendBack({ type: "looked" })),
            (error) => sendBack({ type: "unseen", why: error }),
          )
        take()
        const timer = setInterval(take, input.every)
        return () => clearInterval(timer)
      }),
    },
    // Why it isn't video: what went wrong connecting, or getting a picture. Each time it goes wrong, Telemetry's told
    // too (`reportError`, which the page's client records), by what happened, then why.
    actions: {
      why: assign({ why: ({ event }) => ("why" in event ? event.why : ((event as { error?: Error }).error ?? "")) }),
      tell: (_, what: string) => reportError(new Error(what)),
    },
    delays: { retry: ({ context }) => RETRIES[Math.min(context.tries, RETRIES.length - 1)] },
    guards: { lasting: ({ context }) => context.unseen + 1 >= LASTING },
  }).createMachine({
    context: { tries: 0, unseen: 0 },
    initial: "connecting",
    states: {
      connecting: {
        invoke: { src: "connect", onDone: "live", onError: { target: "pictures", actions: ["why", { type: "tell", params: ({ event }) => `Live video didn't connect: ${said(event.error as Error)}` }] } },
      },
      live: {
        id: "live",
        entry: assign({ tries: 0, unseen: 0, why: undefined }),
        on: {
          lost: { target: "reconnecting", actions: { type: "tell", params: "Live video was lost as it played" } },
          resized: "reconnecting",
        },
      },
      reconnecting: {
        invoke: {
          src: "connect",
          onDone: "live",
          onError: { target: "pictures", actions: ["why", { type: "tell", params: ({ event }) => `Live video didn't connect again: ${said(event.error as Error)}` }] },
        },
      },
      pictures: {
        invoke: { src: "look", input: { every: 1000 } },
        on: {
          looked: { actions: assign({ unseen: 0 }) },
          unseen: [
            { guard: "lasting", target: "unreachable", actions: ["why", { type: "tell", params: ({ event }) => `The screen gave no picture either: ${said(event.why)}` }] },
            { actions: assign({ unseen: ({ context }) => context.unseen + 1 }) },
          ],
        },
        initial: "waiting",
        states: {
          waiting: { after: { retry: "trying" } },
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
  onCleanup(() => ((gone = true), connection.stop()))
  const live = () => state().matches("live")
  // The video stays while it's connected to again, under what says so.
  const showsVideo = () => state().matches("live") || state().matches("reconnecting")

  /** A picture of the screen, and its size, which where it's pressed is worked out from. */
  const shown = (data: string) => {
    setPicture(data)
    const drawn = pngSize(data)
    if (drawn) setSize(drawn)
  }
  const take = () =>
    screen({ action: "screenshot" }).then(
      (data) => shown(data as string),
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
      .then(take)
  }

  // Where on the screen a press on its picture is: drawn as big as fits, in the middle of its place. Before the first
  // picture nothing is drawn, so there's nowhere to point.
  const at = (event: { clientX: number; clientY: number; currentTarget: Located["currentTarget"] }) => {
    if (!size() || !picture()) return
    const [width, height] = size()!
    const box = event.currentTarget.getBoundingClientRect()
    const scale = Math.min(box.width / width, box.height / height)
    const x = (event.clientX - box.x - (box.width - width * scale) / 2) / scale
    const y = (event.clientY - box.y - (box.height - height * scale) / 2) / scale
    return { x: Math.round(Math.min(Math.max(x, 0), width - 1)), y: Math.round(Math.min(Math.max(y, 0), height - 1)) }
  }

  // The screen is resized to the view, in CSS pixels: Chrome on it draws at one pixel each and doesn't take a new
  // scale while it runs, so a screen of the device's pixels would draw its text smaller than the page's. Its stream
  // ends as it's resized, and is connected to again at its new size.
  let resizing: ReturnType<typeof setTimeout> | undefined
  let fixed = false
  let requested = ""
  const resize = () => {
    const { width: wide, height: tall } = place()!
    // A view smaller than the least a screen takes keeps its shape, and is drawn a little smaller.
    const up = Math.max(1, SIZES.width[0] / wide, SIZES.height[0] / tall)
    const width = Math.min(Math.round(wide * up), SIZES.width[1])
    const height = Math.min(Math.round(tall * up), SIZES.height[1])
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
    resizing = setTimeout(() => (queue = queue.then(async () => (await resize()) && (live() ? connection.send({ type: "resized" }) : take()))), 250)
  }
  /// Where it's drawn, as it's laid out and each time that changes: its first place is the one the screen is resized
  /// to before it's connected to, so it isn't connected to twice.
  const laid = (event: Laid) => {
    const { width, height } = event.currentTarget.getBoundingClientRect()
    const first = !place()
    setPlace({ width, height })
    if (!first) return void (!watch && fit())
    if (watch) connection.start()
    else queue = queue.then(resize).then(() => !gone && connection.start())
  }

  // Full Screen fills the window with it, over the page: a page has no display of its own to fill. What's typed goes
  // to the screen from then on.
  const toggle = () => {
    setFull(!full())
    if (!watch) setTyping(true)
  }

  // What's done on its picture, while there's no video: Computer Use's actions, each a picture again after.
  let pointer: { clientX: number; clientY: number; currentTarget: Located["currentTarget"] } | undefined
  const click = (event: Located, kind = "left") => {
    if (live() || watch) return
    const point = at(event)
    if (!point) return
    act({ action: "click", ...point, button: kind })
    setTyping(true)
  }
  const wheel = (event: Wheeled) => {
    const point = pointer && at(pointer)
    if (live() || watch || !point || !event.deltaY) return
    act({ action: "scroll", ...point, direction: event.deltaY > 0 ? "down" : "up", amount: 1 }, (was, now) => was.direction === now.direction && ((was.amount = Math.min(50, (was.amount as number) + 1)), true))
  }
  const type = (text: string) => {
    if (text) act({ action: "type", text }, (was, now) => ((was.text += now.text as string), true))
    setTyped("")
  }
  const key = (event: Keyed) => {
    const name = !fullScreen(event) && named(event)
    if (!name) return
    event.preventDefault()
    const held = mods(event, mac()).map((mod) => ({ Control_L: "ctrl", Alt_L: "alt", Shift_L: "shift", Super_L: "super" })[mod])
    act({ action: "key", keys: [...held, name].join("+") })
  }

  // Where the person types, presses and scrolls: its video, which sends it all itself, or its picture.
  const view = (
    <View
      onKeyDown={(event) => fullScreen(event) && (event.preventDefault(), toggle())}
      style={[
        look.view,
        full() ? look.full : look.tall(watch && size() && place() ? Math.min((place()!.width * size()![1]) / size()![0], viewport().height - 128) : Math.max(256, viewport().height - 240)),
      ]}
    >
      <Pressable
        onLayout={laid}
        onPress={watch ? undefined : (event) => click(event)}
        onContextMenu={watch ? undefined : (event) => click(event, "right")}
        onPointerMove={watch ? undefined : (event: Pressed) => (pointer = event)}
        onWheel={watch ? undefined : wheel}
        style={[look.fill, look.surface]}
      >
        <Show when={ice()}>
          {(servers) => (
            <Video
              ice={servers()}
              answer={answer()}
              watch={watch}
              aria-label={props.label}
              autoFocus={full()}
              onOffer={({ sdp }) => connecting?.offer(sdp)}
              onState={({ state }) => (connecting ? connecting.state(state) : state === "lost" && connection.send({ type: "lost" }))}
              onSize={({ width, height }) => setSize([width, height])}
              style={[look.fill, !showsVideo() && look.hidden]}
            />
          )}
        </Show>
        <Show when={!showsVideo() && picture()}>{(src) => <Image src={src()} alt={props.label} style={look.fill} />}</Show>
      </Pressable>
      <Show when={!watch && !live()}>
        <TextInput
          aria-label={`Type in ${props.label}`}
          value={typed()}
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect={false}
          spellCheck={false}
          autoFocus={typing()}
          onBlur={() => setTyping(false)}
          onKeyDown={key}
          onChangeText={(text) => type(text)}
          style={look.keys}
        />
      </Show>
      <Show when={state().matches("connecting") || (state().matches("pictures") && !size())}>
        <View style={[look.fill, look.state]}>
          <View style={look.middle}>
            <Status heading="Connecting…" />
          </View>
        </View>
      </Show>
      <Show when={state().matches("reconnecting")}>
        <Status over heading="Reconnecting…" />
      </Show>
      <Show when={state().matches("unreachable")}>
        <View style={[look.fill, look.state]}>
          <View style={look.middle}>
            <Problem
              heading="Can't reach this screen"
              text={state().context.why}
              action={
                <Button variant="outline" onPress={() => connection.send({ type: "again" })}>
                  Try Again
                </Button>
              }
            />
          </View>
        </View>
      </Show>
      {/* On a phone the view is small: a tap opens it full screen, where taps are clicks. */}
      <Show when={touch() && !full()}>
        <Pressable onPress={toggle} style={look.fill} />
      </Show>
      {/* Full screen has no Escape, which is the screen's: it has its own way out. */}
      <Show when={full()}>
        <View style={look.leave(Math.max(8, safeArea().top), Math.max(8, safeArea().right))}>
          <Button variant="secondary" size="icon" aria-label={messages.exitFullScreen} onPress={toggle}>
            <Icon name="corners-in" />
          </Button>
        </View>
      </Show>
    </View>
  )

  return (
    <View style={look.root}>
      {!full() && view}
      <Show when={full()}>
        <Portal>{view}</Portal>
      </Show>
      <View style={look.bar}>
        <Show when={state().matches("pictures")}>
          {/* Why it fell back and what it shows are two things, said as two lines. */}
          <View style={look.note}>
            <Text>Live video didn't connect, so this updates each second.</Text>
            <Show when={said(state().context.why)}>
              <Text>{said(state().context.why)}</Text>
            </Show>
          </View>
        </Show>
        <BarEnd value={true}>
          <Button variant="ghost" size="icon" aria-label={messages.fullScreen} onPress={toggle}>
            <Icon name="corners-out" />
          </Button>
        </BarEnd>
      </View>
    </View>
  )
}
