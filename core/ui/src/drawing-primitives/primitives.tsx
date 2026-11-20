// The drawing primitives, as the DOM draws them: a box, a scroll, a text, an image, a path and an editable text, each a
// plain element with the semantic props a native host's renderer has (renderer.ts). A native host draws
// the same props from the mutations Solid's universal renderer tells it. A primitive's `style` is styles (../styles.ts),
// drawn by their classes.
import { createEffect, createMemo, createSignal, For, merge, omit, onCleanup, onSettled, Repeat, Show } from "solid-js"
import { Dynamic, Portal as Inserted, render as mount, type JSX } from "@solidjs/web"
import { frame, sized, tellDrawn, unframe } from "../bridge.ts"
import { layer as whole } from "./layer.ts"
import type { Ice, Props, TextInputAction } from "./renderer.ts"
import { fullScreen, mods, named, type Input } from "./input.ts"
import type { Dragged, Intersected, Laid, Located, Pressed, Wheeled } from "../types/ui.d.ts"
import { flat, web, type Styles } from "../styles.ts"
import { literal, options } from "./props.ts"

/// `<Modal visible={open()}>…</Modal>`: what's drawn over the whole window while it's `visible` (unless it says, it
/// is), over the page and everything over it before, with everything else inert: nothing else is pressed, focused or
/// read by a screen reader. It's a box the window's size, see-through, that lets presses through where nothing of its
/// own is (what's in it says `pointerEvents: "auto"`), so it draws its own scrim and sheet: React Native's
/// `transparent`, always, and no `animationType`, as what's in it moves itself. Escape and Back are what's in it to
/// listen for (`onKeyDown`), not `onRequestClose`.
export function Modal(props: { visible?: boolean; children?: JSX.Element }) {
  return <Show when={props.visible ?? true}>{layer(true, () => props.children)}</Show>
}

/// `<Portal>…</Portal>`: what's drawn over the whole window, over the page and everything over it before, without
/// taking the page from anyone, as a toast or a select's list is: a box the window's size, see-through, that lets
/// presses through where nothing of its own is (what's in it says `pointerEvents: "auto"`). React Native has no
/// such component of its own; this is Solid's `Portal` and @gorhom/portal's.
export function Portal(props: { children?: JSX.Element }) {
  return layer(false, () => props.children)
}

/** A box the window's size, at the end of the page, over it, and while it's `modal`, everything else inert. */
function layer(modal: boolean, content: () => JSX.Element) {
  let box!: HTMLDivElement
  onSettled(() => {
    if (!modal) return
    const opener = document.activeElement as HTMLElement | null
    const others = [...document.body.children].filter((element) => element !== box.parentElement && element !== box && !(element as HTMLElement).inert) as HTMLElement[]
    for (const other of others) other.inert = true
    // The keyboard goes into it: to what asks for it (`autofocus`, a phone's keyboard with it), an alert's first action,
    // what does least by convention, as WAI-ARIA's alertdialog pattern has it, or its dialog.
    const first = box.querySelector("[role='alertdialog']")?.querySelector<HTMLElement>("button:not([disabled]), [tabindex]:not([tabindex='-1'])")
    ;(box.querySelector<HTMLElement>("[autofocus]") ?? first ?? box.querySelector<HTMLElement>("[aria-modal='true']"))?.focus()
    return () => {
      others.forEach((other) => (other.inert = false))
      // Gone, it gives the keyboard back to what had it before, as WAI-ARIA's dialog pattern asks, unless it's gone
      // somewhere else since.
      const now = document.activeElement
      if (opener?.isConnected && (!now || now === document.body || box.contains(now))) opener.focus({ preventScroll: true })
    }
  })
  return (
    <Inserted mount={document.body}>
      <div ref={box} {...drawn("ui-view", whole)}>
        {content()}
      </div>
    </Inserted>
  )
}

/** A picker's props, as @react-native-picker/picker's: the value chosen, what's told as another is, and whether it
 * can be. */
export interface PickerProps {
  selectedValue?: string
  onValueChange?: (value: string, index: number) => void
  enabled?: boolean
  "aria-label"?: string
  style?: Styles
  children?: JSX.Element
}

/** An option of a picker's, as @react-native-picker/picker's `Picker.Item`. */
export interface PickerItemProps {
  label: string
  value: string
  enabled?: boolean
}


// `<Picker selectedValue={value()} onValueChange={setValue}><Picker.Item label="Meals" value="meals" /></Picker>`:
// the device's own picker of one of a few, as a phone's wheel or sheet is (renderer.ts's `picker`), as
// @react-native-picker/picker's: on the DOM a `<select>`, see-through over its box, which the browser opens as its own.
export const Picker = Object.assign(
  (props: PickerProps) => {
    const all = options(() => props.children)
    return (
      <select aria-label={props["aria-label"]} disabled={props.enabled === false} value={props.selectedValue ?? ""} onInput={(event) => props.onValueChange?.(event.currentTarget.value, event.currentTarget.selectedIndex)} {...drawn("ui-picker", props.style)}>
        <For each={all()}>{(option) => <option value={option.value} disabled={option.disabled} selected={option.value === props.selectedValue}>{option.label}</option>}</For>
      </select>
    )
  },
  { Item: (props: PickerItemProps) => props as unknown as JSX.Element },
)

/** A video's props: its stream's signalling, which the page does, and what it's told of it. */
export interface VideoProps {
  ice?: Ice
  answer?: string
  watch?: boolean
  "aria-label": string
  autoFocus?: boolean
  style?: Styles
  onOffer?: (event: { sdp: string }) => void
  onState?: (event: { state: "connected" | "lost" }) => void
  onSize?: (event: { width: number; height: number }) => void
}

/** Whether ⌘ is the key held for a shortcut, as on a Mac, an iPhone or an iPad. */
const mac = /Mac|iPhone|iPad/.test(navigator.platform)
/** How far a wheel turns for one of its steps, in pixels. */
const STEP = 60

/// A screen's live video over WebRTC (renderer.ts's `video`), which `LiveScreen` draws: on the DOM a `<video>`, its
/// peer connection and its data channel, which what's done on it goes down, and a field, unseen, that what's typed
/// goes to, focused as it's pressed, or as its `autoFocus` is set, so a phone shows its keyboard. Neither
/// react-native-video's `Video`, which plays a file, nor react-native-webrtc's `RTCView`, which shows a stream made
/// elsewhere, is this, which makes its own connection; it's named for what it shows. Each `ice` is a connection, the last one closed; its `offer` is told once
/// its candidates are gathered, and its `answer` is set as it comes.
export function Video(props: VideoProps) {
  let video!: HTMLVideoElement
  let field: HTMLInputElement | undefined
  let peer: RTCPeerConnection | undefined
  let channel: RTCDataChannel | undefined
  // What the last connection is told once it's lost.
  let lose = () => {}
  const send = (input: Input) => !props.watch && channel?.readyState === "open" && channel.send(JSON.stringify(input))

  createEffect(
    () => props.ice,
    (ice) => {
      if (!ice) return
      if (!globalThis.RTCPeerConnection) return void props.onState?.({ state: "lost" })
      const pc = (peer = new RTCPeerConnection(ice))
      let over = false
      const lost = (lose = () => !over && ((over = true), props.onState?.({ state: "lost" })))
      pc.addTransceiver("video", { direction: "recvonly" })
      // Both ends make the data channel (id 0), so it opens with the connection, with nothing more to say.
      const data = (channel = pc.createDataChannel("input", { negotiated: true, id: 0 }))
      pc.ontrack = ({ track, receiver }) => {
        // As little as can be held back to play it smoothly: it's a screen you're using.
        receiver.jitterBufferTarget = 0
        video.srcObject = new MediaStream([track])
      }
      data.onopen = () => !over && props.onState?.({ state: "connected" })
      data.onclose = lost
      pc.onconnectionstatechange = () => ["failed", "closed"].includes(pc.connectionState) && lost()
      // The offer goes whole, with its candidates: once they're all gathered, or a moment after the first through
      // TURN, since each TURN server more is one more to wait on.
      pc.setLocalDescription()
        .then(
          () =>
            new Promise<void>((done) => {
              const timer = setTimeout(done, 5000)
              pc.onicegatheringstatechange = () => pc.iceGatheringState === "complete" && done()
              pc.onicecandidate = ({ candidate }) => candidate?.type === "relay" && (clearTimeout(timer), setTimeout(done, 300))
            }),
        )
        .then(() => !over && props.onOffer?.({ sdp: pc.localDescription!.sdp }), lost)
      return () => {
        over = true
        pc.close()
      }
    },
  )
  // Its field has the keyboard each time `autofocus` is set, so what's typed goes to the screen.
  createEffect(
    () => !!props.autoFocus && !props.watch,
    (on) => void (on && requestAnimationFrame(() => field?.isConnected && field.focus({ preventScroll: true }))),
  )
  createEffect(
    () => props.answer,
    (sdp) => void (sdp && peer?.signalingState === "have-local-offer" && peer.setRemoteDescription({ type: "answer", sdp }).catch(lose)),
  )

  // Where on the screen the pointer is: in its pixels, from where the video is drawn, as big as fits, in the middle.
  const at = (event: MouseEvent) => {
    const [width, height] = [video.videoWidth, video.videoHeight]
    if (!width || !height) return
    const box = video.getBoundingClientRect()
    const scale = Math.min(box.width / width, box.height / height)
    const x = (event.clientX - box.left - (box.width - width * scale) / 2) / scale
    const y = (event.clientY - box.top - (box.height - height * scale) / 2) / scale
    return { x: Math.round(Math.min(Math.max(x, 0), width - 1)), y: Math.round(Math.min(Math.max(y, 0), height - 1)) }
  }
  // The pointer, where it is as it moves, at most once a frame, and its buttons as they go down and up.
  let moved: { x: number; y: number } | undefined
  const move = (event: PointerEvent) => {
    const point = at(event)
    if (!point) return
    if (!moved) requestAnimationFrame(() => (send({ t: "move", ...moved! }), (moved = undefined)))
    moved = point
  }
  const button = (event: PointerEvent, down: boolean) => {
    const point = at(event)
    if (props.watch || !point) return
    // What's typed goes to the field, focused once the press is over, as the browser moves focus as it goes down.
    if (down) (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
    else field?.focus({ preventScroll: true })
    send({ t: "move", ...point })
    send({ t: "button", b: [1, 2, 3][event.button] ?? 1, down, mods: mods(event, mac) })
  }
  let turned = { x: 0, y: 0 }
  const wheel = (event: WheelEvent) => {
    const point = at(event)
    if (props.watch || !point || (!event.deltaY && !event.deltaX)) return
    event.preventDefault()
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
  const key = (event: KeyboardEvent) => {
    // The device's full screen is the page's, and goes on to what it's in.
    const name = !fullScreen(event) && named(event)
    if (!name) return
    event.preventDefault()
    send({ t: "key", key: name, mods: mods(event, mac) })
  }
  return (
    <div
      ref={(box: HTMLDivElement) => box.addEventListener("wheel", wheel, { passive: false })}
      onPointerMove={move}
      onPointerDown={(event) => button(event, true)}
      onPointerUp={(event) => button(event, false)}
      onContextMenu={(event) => !props.watch && event.preventDefault()}
      {...drawn("ui-video", props.style)}
    >
      <video
        ref={video}
        aria-label={props["aria-label"]}
        autoplay
        muted
        playsinline
        disablepictureinpicture
        onResize={() => props.onSize?.({ width: video.videoWidth, height: video.videoHeight })}
      />
      <Show when={!props.watch}>
        <input
          ref={field}
          aria-label={`Type in ${props["aria-label"]}`}
          autocomplete="off"
          autocapitalize="off"
          spellcheck={false}
          onKeyDown={key}
          onInput={(event) => (event.target.value && send({ t: "text", text: event.target.value }), (event.target.value = ""))}
        />
      </Show>
    </div>
  )
}

// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
// window on a native host. → what takes it away again. Its client is told once the page has drawn: once what it first
// asked for has settled, under its `<Loading>`s.
export const render = (code: () => JSX.Element): (() => void) => mount(() => (onSettled(() => void tellDrawn()), code()), document.body)

/** What a primitive's element has of its `style`: its classes after the primitive's own, and its inline CSS. */
export const drawn = (own: string, style: Styles | undefined) => (({ class: names, style: inline }) => ({ class: `${own} ${names}`, style: inline }))(web(style))

/** What a primitive takes of the renderer's props: its role and states, with what it listens for. */
type Semantics = Pick<Props, Extract<keyof Props, "role" | "id" | `aria-${string}`>>

/** What a key pressed is told, as the DOM's `KeyboardEvent`: `preventDefault()` keeps what the key does by itself
 * in a field (a caret moved, the focus moved on by Tab) from happening, on the DOM. */
export interface Keyed {
  key: string
  shiftKey: boolean
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
  preventDefault(): void
}

/** What a `Scroll` scrolling is told: how far it's scrolled. */
export interface Scrolled {
  currentTarget: { scrollTop: number; scrollLeft: number }
}

/** What every primitive but an image takes: how it looks, and what it listens for. */
interface Listens extends Semantics {
  style?: Styles
  /** What Tab reaches, as the web's `tabIndex`: `0` for what it otherwise wouldn't, `-1` for what's focused only by a
   * press. What's pressed and a text input are reached already. */
  tabIndex?: 0 | -1
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
  onPointerDown?: (event: Pressed) => void
  onPointerMove?: (event: Pressed) => void
  onPointerUp?: (event: Pressed) => void
  onPointerCancel?: (event: Pressed) => void
  onContextMenu?: (event: Located) => void
  onWheel?: (event: Wheeled) => void
  /** Told where it's drawn as it's laid out, and each time that changes, as React Native's `onLayout`. */
  onLayout?: (event: Laid) => void
  /** Told whether it's near the window, as it's first laid out and each time that changes, as an
   * `IntersectionObserver` with a `rootMargin` of 50% tells. */
  onIntersection?: (event: Intersected) => void
  /** Has the keyboard as it's drawn, and again each time it's set. */
  autoFocus?: boolean
}

/** A view's props. */
export interface ViewProps extends Listens {
  /** Files dropped on it from the system. */
  onDrop?: (files: File[]) => void
  /** Follows a pointer or a finger dragging it, as Motion's `drag`: along (`"x"`), down (`"y"`) or both (`true`),
   * moved by the host where the input arrives, never by a turn of the page's JS. */
  drag?: "x" | "y" | boolean
  /** How far it may be dragged up, left, right and down from where its style puts it, in pixels; past that it gives
   * as a rubber band, as loose as `dragElastic` (0 none, 1 loose, 0.5 if unsaid). */
  dragConstraints?: { top?: number; left?: number; right?: number; bottom?: number }
  dragElastic?: number
  /** Moves from where it was laid out to where it is as that changes, as Motion's `layout`: a list reordered, a card
   * grown. The host draws it on its way, by a spring, with no layout again each frame. */
  layout?: boolean
  /** What it shares its place with, as Motion's `layoutId` and SwiftUI's `matchedGeometryEffect`: one drawn where
   * another with the same id was a moment ago goes there from it (a tab's pill sliding to the tab chosen, an icon
   * growing into its window). */
  layoutId?: string
  onDragStart?: () => void
  /** Dragged on, as Motion's `onDrag`: how far it's held from where its style puts it, as it moves. A host before
   * this one doesn't say it. */
  onDrag?: (event: Pick<Dragged, "offset">) => void
  /** Let go: how far it went and how fast it was going, in pixels a second. It springs from where it is, at that
   * speed, to where its style puts it, which this may move. */
  onDragEnd?: (event: Dragged) => void
  children?: JSX.Element
}

/** A pressable's props: a view's, and what pressing it does. */
export interface PressableProps extends ViewProps {
  onPress?: (event: Located) => void
  disabled?: boolean
}

/** The roles of what's pressed, which the DOM draws as a button, so the keyboard reaches it too. */
const pressed = new Set(["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"])
/** The roles of what's chosen from a list another holds the keyboard for, which a press takes no focus from it. */
const chosen = new Set(["option"])

/// `<View style={look.row}>...</View>`: an app's own layout, around anything else: a flex column unless its `style`
/// says otherwise, as React Native's View, styled from the design tokens only. `role` and `aria-…` say what it is to
/// a screen reader and to an agent, as ARIA has them, which React Native takes too in place of its older
/// `accessibility…` props; `id` and `tabIndex` are the web's, and there's no `testID` or `nativeID`. `pointerEvents`
/// is in its style, not a prop. What's pressed is a `Pressable`. A page moved by hand listens to its pointers too
/// (`onPointerDown`…, as the web and React Native both name them).
export function View(props: ViewProps) {
  return Pressable(props)
}

/// `<Pressable role="button" aria-label="Close" onPress={close}>...</Pressable>`: a view that's pressed, by a pointer,
/// a finger, or Enter or Space where the keyboard is, as React Native's Pressable, its one way to press (none of the
/// `Touchable…` family). With a role of what's pressed (`button`, `menuitem`, `option`, …) the keyboard reaches it.
/// `onPress` is told where it was (`clientX`, `currentTarget.getBoundingClientRect()`), and a `disabled` one isn't
/// pressed. How it looks hovered or pressed is its style's (`backgroundColor: { default: …, pressed: … }`), not a
/// function of its state, and a touch held is `onContextMenu`, not `onLongPress`.
export function Pressable(props: PressableProps) {
  const own = merge(omit(props, "style", "children", "autoFocus", "tabIndex", "onPress", "disabled", "onLayout", "onIntersection", "onDrop", ...moved), droppable(props), {
    get tabindex() {
      return props.tabIndex
    },
    get onClick() {
      return props.disabled ? undefined : props.onPress
    },
    get "aria-disabled"() {
      return props.disabled ? "true" : props["aria-disabled"]
    },
  })
  const moves = movable(props)
  const button = () => !!props.role && pressed.has(props.role) && !!own.onClick
  return (
    <Dynamic
      component={button() ? "button" : "div"}
      type={button() ? "button" : undefined}
      {...own}
      {...moves}
      ref={[moves.ref, (element: HTMLElement) => watched(element, props)]}
      onMouseDown={(event: MouseEvent) => props.role && chosen.has(props.role) && event.preventDefault()}
      {...drawn("ui-view", props.style)}
    >
      {props.children}
    </Dynamic>
  )
}

/** A scroll view's props. */
export interface ScrollViewProps extends ViewProps {
  onScroll?: (event: Scrolled) => void
}

/// `<ScrollView style={look.list}>...</ScrollView>`: a view whose content scrolls along its `flexDirection`, down in
/// a column and sideways in a row, with no scrollbar beside it, as on a phone. It has one style, where React Native's
/// takes `horizontal` and a `contentContainerStyle` too. `onScroll` is told how far it's scrolled, as the web's
/// (`currentTarget.scrollTop`).
export function ScrollView(props: ScrollViewProps) {
  const own = merge(omit(props, "style", "children", "tabIndex", "onDrop", ...moved), droppable(props), {
    get tabindex() {
      return props.tabIndex
    },
  })
  const moves = movable(props)
  return (
    <div {...own} {...moves} data-scrolls="" {...drawn("ui-view ui-scroll-view", props.style)}>
      {props.children}
    </div>
  )
}

export interface VirtualizedListProps<T> extends Semantics {
  style?: Styles
  /** The rows' items, each drawn by `renderItem` as it comes near the window. */
  data: readonly T[]
  /** A row: its item, and which it is, from 0. */
  renderItem: (info: { item: T; index: number }) => JSX.Element
  /** How tall a row is taken to be until one's drawn, in CSS pixels, as FlashList's. */
  estimatedItemSize?: number
}

/** How many rows a list draws before it knows where it is: a window's. */
const FIRST_ROWS = 24

/// `<VirtualizedList data={notes()} estimatedItemSize={38} renderItem={({ item }) => <Row heading={item.text} />} />`:
/// a long list, of which only the rows near the window are drawn, as React Native's VirtualizedList (FlatList's) and
/// FlashList draw them: those a window either side of what shows, each as it comes into that and gone as it leaves it,
/// the room of the rest held as tall as the drawn ones are on average (`estimatedItemSize` until one is), so a list of
/// 10,000 is as quick as one of 20. A native host owns where it is (renderer.ts's `list`); here the page does, from
/// where it's drawn as anything scrolls. A row is its item's, by its index: what changes in `data` changes the row.
export function VirtualizedList<T>(props: VirtualizedListProps<T>) {
  let box: HTMLDivElement | undefined
  /** How tall each row drawn was, by its index. */
  const heights = new Map<number, number>()
  const [measured, setMeasured] = createSignal(0)
  const [range, setRange] = createSignal({ first: 0, last: FIRST_ROWS })
  /** Where each row starts, and where the last ends. */
  const offsets = createMemo(() => {
    measured()
    const drawn = [...heights.values()]
    const estimate = drawn.length ? drawn.reduce((sum, height) => sum + height, 0) / drawn.length : (props.estimatedItemSize ?? 38)
    const at = [0]
    for (let row = 0; row < props.data.length; row++) at.push(at[row] + (heights.get(row) ?? estimate))
    return at
  })
  const shown = () => {
    const last = Math.min(range().last, props.data.length)
    const first = Math.min(range().first, last)
    return { first, last, top: offsets()[first], bottom: offsets().at(-1)! - offsets()[last] }
  }
  /// The rows a window either side of what shows of it.
  const ask = () => {
    if (!box) return
    const top = box.getBoundingClientRect().top
    const starts = offsets()
    const count = starts.length - 1
    const from = -top - innerHeight
    const to = -top + 2 * innerHeight
    let first = 0
    while (first < count && starts[first + 1] <= from) first++
    let last = first
    while (last < count && starts[last] < to) last++
    if (first !== range().first || last !== range().last) setRange({ first, last })
  }
  const sized = new ResizeObserver((entries) => {
    for (const entry of entries) heights.set(Number((entry.target as HTMLElement).dataset.index), entry.borderBoxSize[0]?.blockSize ?? entry.contentRect.height)
    setMeasured((times) => times + 1)
    ask()
  })
  addEventListener("scroll", ask, { capture: true, passive: true })
  addEventListener("resize", ask)
  onSettled(ask)
  onCleanup(() => (sized.disconnect(), removeEventListener("scroll", ask, { capture: true }), removeEventListener("resize", ask)))
  const own = omit(props, "style", "data", "renderItem", "estimatedItemSize")
  return (
    <div {...own} ref={(element: HTMLDivElement) => (box = element)} {...drawn("ui-view", props.style)}>
      <div style={{ height: `${shown().top}px`, "flex-shrink": 0 }} />
      <Repeat count={shown().last - shown().first} from={shown().first}>
        {(index) => (
          <div
            class="ui-view"
            data-index={index}
            ref={(element: HTMLDivElement) => {
              sized.observe(element)
              onCleanup(() => sized.unobserve(element))
            }}
          >
            {props.renderItem({
              get item() {
                return props.data[index]
              },
              index,
            })}
          </div>
        )}
      </Repeat>
      <div style={{ height: `${shown().bottom}px`, "flex-shrink": 0 }} />
    </div>
  )
}

/// A view's keyboard and layout: it takes the keyboard each time its `autoFocus` is set, is told where it's drawn as
/// that changes (`onLayout`), and whether it's near the window (`onIntersection`), within half the window of what
/// shows of it, whatever scrolls it (`scrollMargin`, where the browser has it).
function watched(element: HTMLElement, props: ViewProps) {
  createEffect(
    () => !!props.autoFocus,
    (focus) => void (focus && queueMicrotask(() => element.focus({ preventScroll: false }))),
  )
  if (props.onIntersection) {
    const near = new IntersectionObserver((entries) => props.onIntersection?.({ isIntersecting: entries.at(-1)!.isIntersecting }), { rootMargin: "50%", scrollMargin: "50%" } as IntersectionObserverInit)
    near.observe(element)
    onCleanup(() => near.disconnect())
  }
  if (!props.onLayout) return
  // A view moves as well as grows: as it's resized, and as the window is, which moves what's pinned to its foot.
  const report = () => props.onLayout?.({ currentTarget: element })
  const sized = new ResizeObserver(report)
  sized.observe(element)
  window.addEventListener("resize", report)
  onCleanup(() => (sized.disconnect(), window.removeEventListener("resize", report)))
}

/** A view's DOM drag and drop, where it takes files dropped on it (`onDrop`). */
const droppable = (props: ViewProps) => ({
  get onDragOver() {
    return props.onDrop && ((event: DragEvent) => event.preventDefault())
  },
  get onDrop() {
    return props.onDrop && ((event: DragEvent) => (event.preventDefault(), props.onDrop!([...(event.dataTransfer?.files ?? [])])))
  },
})

/** What a page moved by hand listens for (renderer.ts's `Moved`), which `movable` draws. */
// A drag and a move as a node is laid out again are a native host's (`drag`, `layout`), which the DOM leaves where its style puts it.
const moved = ["onPointerDown", "onPointerMove", "onPointerUp", "onPointerCancel", "onContextMenu", "onWheel", "drag", "dragConstraints", "dragElastic", "onDragStart", "onDrag", "onDragEnd", "layout", "layoutId"] as const

/// A box's or a scroll's `ref` and `onContextMenu` for what it moves by hand: its pointers (`follows`), and a
/// right-click or a touch held, in place of the device's own menu.
const movable = (props: ViewProps) => ({
  ref: (box: HTMLElement) => (props.onPointerDown || props.onPointerMove || props.onPointerUp || props.onPointerCancel || props.onWheel) && follows(box, props),
  onContextMenu: (event: MouseEvent) => props.onContextMenu && (event.preventDefault(), event.stopPropagation(), props.onContextMenu(event as unknown as Located)),
})

/// A box's pointers, as the renderer has them: a pointer's moves over it, a mouse's too, and once one is down on it,
/// its moves and its end wherever it goes, as a touch's are, each told where the box is (`currentTarget`). While a
/// finger that's down on it moves, the browser scrolls nothing for it, unless it's in something in the box that
/// scrolls (an iPhone's web view would bounce the page). A wheel over it is its own, unless it's up and down over
/// something in it that scrolls, whose that is.
function follows(box: HTMLElement, props: ViewProps) {
  const down = new Set<number>()
  const told = (event: PointerEvent): Pressed => ({ clientX: event.clientX, clientY: event.clientY, timeStamp: event.timeStamp, pointerId: event.pointerId, pointerType: event.pointerType as Pressed["pointerType"], button: event.button, currentTarget: box })
  const move = (event: PointerEvent) => down.has(event.pointerId) && props.onPointerMove?.(told(event))
  const end = (event: PointerEvent) => {
    if (!down.delete(event.pointerId)) return
    if (event.type === "pointerup") props.onPointerUp?.(told(event))
    else props.onPointerCancel?.(told(event))
    if (down.size) return
    removeEventListener("pointermove", move, true)
    removeEventListener("pointerup", end, true)
    removeEventListener("pointercancel", end, true)
  }
  box.addEventListener("pointerdown", (event) => {
    if (!down.size) {
      addEventListener("pointermove", move, true)
      addEventListener("pointerup", end, true)
      addEventListener("pointercancel", end, true)
    }
    down.add(event.pointerId)
    props.onPointerDown?.(told(event))
  })
  box.addEventListener("pointermove", (event) => !down.has(event.pointerId) && props.onPointerMove?.(told(event)))
  box.addEventListener("dragstart", (event) => event.preventDefault())
  box.dataset.follows = ""
  // A box whose style lets the device scroll one way (`touchAction`) has its pointer cancelled as it does.
  box.addEventListener("touchmove", (event) => down.size && !scrolls(event.target, box) && !getComputedStyle(box).touchAction.startsWith("pan") && event.cancelable && event.preventDefault(), { passive: false })
  box.addEventListener(
    "wheel",
    (event) => {
      if (Math.abs(event.deltaY) > Math.abs(event.deltaX) && scrolls(event.target, box)) return
      event.preventDefault()
      const lines = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? box.clientHeight : 1
      props.onWheel?.({ deltaX: event.deltaX * lines, deltaY: event.deltaY * lines, timeStamp: event.timeStamp })
    },
    { passive: false },
  )
}

/** Whether `target` is in something in `box` that scrolls. */
const scrolls = (target: EventTarget | null, box: HTMLElement) => {
  const scroller = (target as Element | null)?.closest?.("[data-scrolls]")
  return !!scroller && scroller !== box && box.contains(scroller)
}

/** A text's props. */
export interface TextProps extends Semantics {
  style?: Styles
  /** Pressed where its words are, as an inline link (`role="link"`), reached by Tab with a `tabIndex`. */
  tabIndex?: 0 | -1
  onPress?: (event: Located) => void
  children?: JSX.Element
}

/// `<Text style={look.title}>Recently Deleted</Text>`: words, in one of the type styles (`font: tokens.type.title`)
/// and a token's `color`, or as what it's in has them. It runs on in a text it's in, and is a block in a view, as React
/// Native's Text. `role="heading"` with `aria-level` makes it a heading; one inside another with `role="link"` and
/// `onPress` is a link in its words. An app's words are always in a `Text` or in a UI library component that takes them.
export function Text(props: TextProps) {
  const own = merge(omit(props, "style", "children", "onPress", "tabIndex"), {
    get onClick() {
      return props.onPress
    },
    get tabindex() {
      return props.tabIndex
    },
  })
  const tag = () => (props.role === "heading" ? `h${props["aria-level"] ?? 2}` : "span")
  return (
    <Dynamic component={tag()} {...own} role={props.role === "heading" ? undefined : props.role} {...drawn("ui-text", props.style)}>
      {props.children}
    </Dynamic>
  )
}

/** An image's props. */
export interface ImageProps extends Pick<Semantics, "aria-hidden"> {
  /** A `data:` address, or a file of the app's own by its path. */
  src: string
  /** What it shows, which names it; `""` for one that's only how it looks. */
  alt: string
  style?: Styles
}

/// `<Image src="icon.svg" alt="" style={look.icon} />`: a picture, from a `data:` address or the app's
/// own files, named by `alt`, scaled to fit its box.
export function Image(props: ImageProps) {
  return <img src={props.src} alt={props.alt} aria-hidden={props["aria-hidden"]} {...drawn("ui-image", props.style)} />
}

/** A frame's props: the page it shows, as a `one:` link names it after its scheme, for whoever it says. */
export interface FrameProps {
  src: string
  subject?: string
  title?: string
  style?: Styles
}

/// `<Frame src="chrome-profiles/client/profile.html#work" title="Work" style={look.fill} />`: another app's page, in
/// an iframe at its own app's address, which the client answers as that app's, as tall as its page says its content
/// is where its style says no height. Only a trusted app's page shows one.
export function Frame(props: FrameProps) {
  const [shown, setShown] = createSignal<{ url: string; sandbox: string; frame: string }>()
  const [height, setHeight] = createSignal<number>()
  createEffect(
    () => [props.src, props.subject] as const,
    ([src, subject]) => {
      let gone = false
      frame(src, subject).then((at) => {
        if (gone) return
        sized.set(at.frame, setHeight)
        setShown(at)
      })
      return () => {
        gone = true
        const was = shown()
        if (!was) return
        sized.delete(was.frame)
        unframe(was.frame)
      }
    },
  )
  const look = () => drawn("ui-frame", props.style)
  // As tall as its page says, as the native host draws it, unless its style says how tall: an iframe's 150 until then.
  const tall = () => flat(props.style as Styles).some((one) => "height" in one)
  const style = () => ({ ...(look().style as Record<string, string | number> | undefined), ...(height() !== undefined && !tall() && { height: `${height()}px` }) })
  return (
    <Show when={shown()}>
      {(at) => <iframe src={at().url} sandbox={at().sandbox} title={props.title} allow="clipboard-write" class={look().class} style={style()} />}
    </Show>
  )
}

/** A vector drawing's props. */
export interface SvgProps extends Pick<Semantics, "role" | "aria-label" | "aria-hidden"> {
  /** The box its paths are drawn in, as SVG's: `"0 0 256 256"`. */
  viewBox: string
  style?: Styles
  children?: JSX.Element
}

/// `<Svg viewBox="0 0 100 20" style={look.chart}><Path d="M0 20 L50 4 L100 12" /></Svg>`: a vector drawing, as
/// react-native-svg's `Svg`, its `viewBox` stretched to its box (`preserveAspectRatio="none"`, the one way it's
/// drawn): an icon, a chart's line or area. Each `Path` in it is filled with the text's colour (`color` in the Svg's
/// style), as `fill="currentColor"` is. `role="img"` with `aria-label` names one that says something; one with no role
/// is only how it looks.
export function Svg(props: SvgProps) {
  return (
    <svg viewBox={props.viewBox} preserveAspectRatio="none" role={props.role} aria-label={props["aria-label"]} aria-hidden={props["aria-hidden"] ?? (props.role ? undefined : "true")} {...drawn("ui-svg", props.style)}>
      {props.children}
    </svg>
  )
}

/** A path's props: its shape, and how it's filled and stroked. */
export interface PathProps {
  /** Its shape, as SVG's path data. */
  d: string
  /** Its `fill`, `stroke` and `strokeWidth`, in a style as every look is, where react-native-svg takes them as props
   * too. */
  style?: Styles
}

/// `<Path d="M0 20 L50 4 L100 12" />`, in an `Svg`: a shape, as react-native-svg's `Path`, its stroke as wide however
/// it's stretched.
export function Path(props: PathProps) {
  const looks = () => (props.style ? web(props.style) : undefined)
  return <path d={props.d} vector-effect="non-scaling-stroke" class={looks()?.class} style={looks()?.style} />
}

/** What a text input's `onKeyDown` is told, and its keyboard's action. */
type EnterKeyHint = "enter" | "done" | "go" | "next" | "previous" | "search" | "send"

/** A text input's props. */
export interface TextInputProps extends Pick<Semantics, "role" | "aria-label" | "aria-description" | "aria-invalid" | "aria-controls" | "aria-activedescendant" | "aria-expanded"> {
  name?: string
  value?: string
  /** Which keyboard a touch screen shows for it, React Native's and the web's `inputMode` (not React Native's older
   * `keyboardType`): `numeric` is digits alone; `email`, `url`, `tel` and `search` are those kinds of field, to
   * autofill and to a screen reader too, as react-native-web makes them. */
  inputMode?: "text" | "numeric" | "decimal" | "tel" | "email" | "url" | "search"
  /** What's typed is hidden, as a password's. */
  secureTextEntry?: boolean
  multiline?: boolean
  placeholder?: string
  /** What its keyboard's action key says, React Native's and the web's `enterKeyHint` (not React Native's older
   * `returnKeyType`): `done` in one line unless it says, a new line (`enter`) in several. */
  enterKeyHint?: EnterKeyHint
  autoCapitalize?: Props["autocapitalize"]
  autoCorrect?: boolean
  spellCheck?: boolean
  autoComplete?: string
  autoFocus?: boolean
  disabled?: boolean
  /** Shown, chosen and copied, but not typed in: React Native's `readOnly` (the web's), not its older `editable`. */
  readOnly?: boolean
  required?: boolean
  style?: Styles
  /** What's in it now, each time that changes: its one change handler, React Native's `onChangeText`, with no
   * `onChange` too. */
  onChangeText?: (text: string) => void
  /** Its action key was pressed: Return in one line, or a phone's Done, Go, Search or Send. */
  onSubmitEditing?: () => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
}

/** The kinds of field an `inputMode` makes on the DOM, as react-native-web makes them. */
const kinds = new Set(["email", "url", "tel", "search"])
/** What its action key does, by what it says. */
const actions: Record<EnterKeyHint, TextInputAction> = { enter: "newline", done: "done", go: "go", next: "next", previous: "previous", search: "search", send: "send" }

/// `<TextInput value={text()} onChangeText={setText} onSubmitEditing={save} />`: text to type, bare, as React
/// Native's TextInput, which `Input` draws in a field's box: the DOM's own `<input>`, or `<textarea>` with
/// `multiline`, so selection, an input method's composing, paste and autofill are the device's own. `enterKeyHint`
/// names its action key (a phone's Done, Go, Search, Send), and `onSubmitEditing` is told it was pressed: Return in one
/// line, as an input method that's composing doesn't. In several lines Return is a new line unless `enterKeyHint`
/// says otherwise. What a page drawn on the DOM gives it besides (`pattern`, a form's `name`) reaches the element. A
/// `readOnly` one is all selected as it's focused, to copy. Its text runs the way its first strong character does
/// (`dir="auto"`), so Hebrew or Arabic starts at the right.
export function TextInput(props: TextInputProps) {
  const own = omit(props as TextInputProps & { class?: string }, "style", "class", "multiline", "enterKeyHint", "autoCorrect", "autoFocus", "onSubmitEditing", "onKeyDown", "onFocus", "onChangeText", "inputMode", "secureTextEntry", "readOnly", "autoCapitalize", "spellCheck", "autoComplete")
  const action = (): TextInputAction => (props.enterKeyHint ? actions[props.enterKeyHint] : props.multiline ? "newline" : "done")
  const keyed = (event: KeyboardEvent) => {
    props.onKeyDown?.(event)
    if (event.key !== "Enter" || event.isComposing || event.keyCode === 229 || event.defaultPrevented || !props.onSubmitEditing || action() === "newline" || event.shiftKey) return
    event.preventDefault()
    props.onSubmitEditing()
  }
  // What's in it is what the app last set: where the app keeps something else of what was typed (only its digits), the
  // field shows that, as a native host does.
  const typed = (event: InputEvent) => {
    const field = event.currentTarget as HTMLInputElement
    props.onChangeText?.(field.value)
    queueMicrotask(() => props.value !== undefined && field.value !== props.value && (field.value = props.value))
  }
  const focused = (event: FocusEvent) => {
    if (props.readOnly) (event.currentTarget as HTMLInputElement).select()
    props.onFocus?.()
  }
  // A page in a frame of another origin, as an app's is, isn't given `autofocus`: it's focused as it's drawn, and
  // again each time it's set.
  const focus = (element: HTMLElement) =>
    createEffect(
      () => !!props.autoFocus,
      (on) => void (on && requestAnimationFrame(() => element.isConnected && element.focus({ preventScroll: true }))),
    )
  const looks = () => drawn("ui-text-input", props.style)
  const corrected = () => props.autoCorrect ?? (literal(props) ? false : undefined)
  return (
    <Dynamic
      component={props.multiline ? "textarea" : "input"}
      type={props.multiline ? undefined : props.secureTextEntry ? "password" : kinds.has(props.inputMode!) ? props.inputMode : "text"}
      inputmode={props.inputMode}
      dir="auto"
      enterkeyhint={props.enterKeyHint || !props.multiline ? (props.enterKeyHint ?? "done") : undefined}
      autocorrect={corrected() === undefined ? undefined : corrected() ? "on" : "off"}
      autocapitalize={props.autoCapitalize ?? (literal(props) ? "none" : undefined)}
      spellcheck={props.spellCheck}
      autocomplete={props.autoComplete}
      readonly={props.readOnly}
      autofocus={props.autoFocus}
      {...own}
      ref={focus}
      onKeyDown={keyed}
      onInput={typed}
      onFocus={focused}
      {...looks()}
      class={[looks().class, (props as { class?: string }).class].filter(Boolean).join(" ")}
    />
  )
}
