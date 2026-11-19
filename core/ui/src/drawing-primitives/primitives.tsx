// The drawing primitives, as the DOM draws them: a box, a scroll, a text, an image, a path and an editable text, each a
// plain element with the semantic props a native host's renderer has (renderer.ts). A native host draws
// the same props from the mutations Solid's universal renderer tells it. A primitive's `style` is styles (../styles.ts),
// drawn by their classes, or, on a page written for version 1, one style object, drawn as inline CSS.
import { createEffect, createSignal, For, omit, onCleanup, onSettled, Show } from "solid-js"
import { Dynamic, Portal, render as mount, type JSX } from "@solidjs/web"
import { frame, sized, tellDrawn, unframe } from "../bridge.ts"
import { layer } from "./layer.ts"
import type { Ice, Props, TextInputAction, Version1Style } from "./renderer.ts"
import { fullScreen, mods, named, type Input } from "./input.ts"
import type { Intersected, Laid, Located, Pressed, Wheeled } from "../types/ui.d.ts"
import { flat, web, type Styles } from "../styles.ts"

/// `<Layer modal>…</Layer>`: what's drawn over the whole window, over the page and every layer before it, as a
/// sheet or a menu is: a box the window's size that lets presses through where nothing of its own is (what's in it
/// says `pointerEvents: "auto"`). While a `modal` one is shown, everything else is inert: nothing else is pressed,
/// focused or read by a screen reader.
export function Layer(props: { modal?: boolean; children?: JSX.Element }) {
  let box!: HTMLDivElement
  onSettled(() => {
    if (!props.modal) return
    const others = [...document.body.children].filter((element) => element !== box.parentElement && element !== box && !(element as HTMLElement).inert) as HTMLElement[]
    for (const other of others) other.inert = true
    // The keyboard goes into it: to what asks for it (`autofocus`, a phone's keyboard with it), or to its dialog.
    box.querySelector<HTMLElement>("[autofocus], [aria-modal='true']")?.focus()
    return () => others.forEach((other) => (other.inert = false))
  })
  return (
    <Portal mount={document.body}>
      <div ref={box} {...drawn("ui-view", layer)}>
        {props.children}
      </div>
    </Portal>
  )
}

/** A picker's props: what can be chosen, the one that is, and what's told as another is chosen. */
export interface PickerProps {
  options: { value: string; label: string; disabled?: boolean }[]
  value?: string
  disabled?: boolean
  "aria-label"?: string
  onInput?: (event: { currentTarget: { value: string } }) => void
  style?: Style
}

/// The device's own picker of one of a few, as a phone's wheel or sheet is (renderer.ts's `picker`): on the DOM a
/// `<select>`, see-through over its box, which the browser opens as its own.
export function Picker(props: PickerProps) {
  return (
    <select aria-label={props["aria-label"]} disabled={props.disabled} value={props.value ?? ""} onInput={(event) => props.onInput?.({ currentTarget: { value: event.currentTarget.value } })} {...drawn("ui-picker", props.style)}>
      <For each={props.options}>{(option) => <option value={option.value} disabled={option.disabled} selected={option.value === props.value}>{option.label}</option>}</For>
    </select>
  )
}

/** A video's props: its stream's signalling, which the page does, and what it's told of it. */
export interface VideoProps {
  ice?: Ice
  answer?: string
  watch?: boolean
  label: string
  autofocus?: boolean
  style?: Style
  onOffer?: (event: { sdp: string }) => void
  onState?: (event: { state: "connected" | "lost" }) => void
  onSize?: (event: { width: number; height: number }) => void
}

/** Whether ⌘ is the key held for a shortcut, as on a Mac, an iPhone or an iPad. */
const mac = /Mac|iPhone|iPad/.test(navigator.platform)
/** How far a wheel turns for one of its steps, in pixels. */
const STEP = 60

/// A screen's live video over WebRTC (renderer.ts's `video`): on the DOM a `<video>`, its peer connection and its
/// data channel, which what's done on it goes down, and a field, unseen, that what's typed goes to, focused as it's
/// pressed, or as its `autofocus` is set, so a phone shows its keyboard. Each `ice` is a connection, the last one closed; its `offer` is told once
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
    () => !!props.autofocus && !props.watch,
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
        aria-label={props.label}
        autoplay
        muted
        playsinline
        disablepictureinpicture
        onResize={() => props.onSize?.({ width: video.videoWidth, height: video.videoHeight })}
      />
      <Show when={!props.watch}>
        <input
          ref={field}
          aria-label={`Type in ${props.label}`}
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

/// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
/// window on a native host. → what takes it away again. Its client is told once the page has drawn: once what it first
/// asked for has settled, under its `<Loading>`s.
export const render = (code: () => JSX.Element): (() => void) => mount(() => (onSettled(() => void tellDrawn()), code()), document.body)

/** A primitive's `style`: styles, or version 1's one object. */
type Style = Styles | Version1Style
/** Whether `style` is version 1's. */
const v1 = (style: Style | undefined): style is Version1Style => !!style && !Array.isArray(style) && typeof style === "object" && !("$id" in style)
/** What a primitive's element has of its `style`: its classes after the primitive's own, its inline CSS, and a version
 * 1 type style's `data-font`. */
export const drawn = (own: string, style: Style | undefined) => (v1(style) ? { class: own, style: css(style), "data-font": style.font } : (({ class: names, style: inline }) => ({ class: `${own} ${names}`, style: inline }))(web(style as Styles)))

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
  style?: Style
  tabindex?: 0 | -1
  onClick?: (event: Located) => void
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
  autofocus?: boolean
}

/** A box's props. */
export interface BoxProps extends Listens {
  children?: JSX.Element
}

/** The roles of what's pressed, which the DOM draws as a button, so the keyboard reaches it too. */
const pressed = new Set(["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link", "combobox"])
/** The roles of what's chosen from a list another holds the keyboard for, which a press takes no focus from it. */
const chosen = new Set(["option"])

/// `<Box style={{ flexDirection: "row", gap: "gutter", padding: 12, backgroundColor: "muted", borderRadius: "lg" }}>...</Box>`:
/// an app's own layout, around anything else: a flex column unless its `style` says otherwise, as React Native's
/// View, styled from the design tokens only. With `onClick` and a role of what's pressed (`button`, `menuitem`,
/// `option`, …) it's something to press, which the keyboard reaches too; any other `role` and `aria-…` say what it is
/// to a screen reader and to an agent, as ARIA has them. A page moved by hand listens to its pointers too (`follows`).
export function Box(props: BoxProps) {
  const own = omit(props, "style", "children", "autofocus", "onLayout", "onIntersection", ...moved)
  const moves = movable(props)
  const button = () => !!props.role && pressed.has(props.role) && !!props.onClick
  return (
    <Dynamic
      component={button() ? "button" : "div"}
      type={button() ? "button" : undefined}
      {...own}
      {...moves}
      ref={[moves.ref, (element: HTMLElement) => watched(element, props)]}
      data-scrolls={v1(props.style) && props.style.overflow === "scroll" ? "" : undefined}
      onMouseDown={(event: MouseEvent) => props.role && chosen.has(props.role) && event.preventDefault()}
      {...drawn("ui-view", props.style)}
    >
      {props.children}
    </Dynamic>
  )
}

/** A scroll's props. */
export interface ScrollProps extends BoxProps {
  onScroll?: (event: Scrolled) => void
}

/// `<Scroll style={{ flexGrow: 1, gap: "gutter", paddingInline: "gutter" }}>...</Scroll>`: a box whose content scrolls
/// along its `flexDirection`, down in a column and sideways in a row, with no scrollbar beside it, as on a phone.
/// `onScroll` is told how far it's scrolled.
export function Scroll(props: ScrollProps) {
  const own = omit(props, "style", "children", ...moved)
  const moves = movable(props)
  return (
    <div {...own} {...moves} data-scrolls="" {...drawn("ui-view ui-scroll-view", props.style)}>
      {props.children}
    </div>
  )
}

/// A box's keyboard and layout: it takes the keyboard each time its `autofocus` is set, is told where it's drawn as
/// that changes (`onLayout`), and whether it's near the window (`onIntersection`), within half the window of what
/// shows of it, whatever scrolls it (`scrollMargin`, where the browser has it).
function watched(element: HTMLElement, props: BoxProps) {
  createEffect(
    () => !!props.autofocus,
    (focus) => void (focus && queueMicrotask(() => element.focus({ preventScroll: false }))),
  )
  if (props.onIntersection) {
    const near = new IntersectionObserver((entries) => props.onIntersection?.({ isIntersecting: entries.at(-1)!.isIntersecting }), { rootMargin: "50%", scrollMargin: "50%" } as IntersectionObserverInit)
    near.observe(element)
    onCleanup(() => near.disconnect())
  }
  if (!props.onLayout) return
  // A box moves as well as grows: as it's resized, and as the window is, which moves what's pinned to its foot.
  const report = () => props.onLayout?.({ currentTarget: element })
  const sized = new ResizeObserver(report)
  sized.observe(element)
  window.addEventListener("resize", report)
  onCleanup(() => (sized.disconnect(), window.removeEventListener("resize", report)))
}

/** What a page moved by hand listens for (renderer.ts's `Moved`), which `movable` draws. */
const moved = ["onPointerDown", "onPointerMove", "onPointerUp", "onPointerCancel", "onContextMenu", "onWheel"] as const

/// A box's or a scroll's `ref` and `onContextMenu` for what it moves by hand: its pointers (`follows`), and a
/// right-click or a touch held, in place of the device's own menu.
const movable = (props: BoxProps) => ({
  ref: (box: HTMLElement) => (props.onPointerDown || props.onPointerMove || props.onPointerUp || props.onPointerCancel || props.onWheel) && follows(box, props),
  onContextMenu: (event: MouseEvent) => props.onContextMenu && (event.preventDefault(), event.stopPropagation(), props.onContextMenu(event as unknown as Located)),
})

/// A box's pointers, as the renderer has them: a pointer's moves over it, a mouse's too, and once one is down on it,
/// its moves and its end wherever it goes, as a touch's are, each told where the box is (`currentTarget`). While a
/// finger that's down on it moves, the browser scrolls nothing for it, unless it's in something in the box that
/// scrolls (an iPhone's web view would bounce the page). A wheel over it is its own, unless it's up and down over
/// something in it that scrolls, whose that is.
function follows(box: HTMLElement, props: BoxProps) {
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
  style?: Style
  /** Pressed where its words are, as an inline link (`role="link"`), reached by Tab with a `tabindex`. */
  tabindex?: 0 | -1
  onClick?: (event: Located) => void
  children?: JSX.Element
}

/// `<Text style={{ font: "title" }}>Recently Deleted</Text>`: words, in one of the type styles (`font`: `body`,
/// `label`, `title`, `figure`, `subheadline`, `footnote`, `caption-1`, `caption-2`) and a token's `color`, or as
/// what it's in has them. It runs on in a text it's in, and is a block in a box. `role="heading"` with `aria-level`
/// makes it a heading; one inside another with `role="link"` and `onClick` is a link in its words. An app's words
/// are always in a `Text` or in a UI library component that takes them.
export function Text(props: TextProps) {
  const own = omit(props, "style", "children")
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
  style?: Style
}

/// `<Image src="icon.svg" alt="" style={{ width: 24, height: 24 }} />`: a picture, from a `data:` address or the app's
/// own files, named by `alt`, scaled to fit its box.
export function Image(props: ImageProps) {
  return <img src={props.src} alt={props.alt} aria-hidden={props["aria-hidden"]} {...drawn("ui-image", props.style)} />
}

/** A frame's props: the page it shows, as a `one:` link names it after its scheme, for whoever it says. */
export interface FrameProps {
  src: string
  subject?: string
  title?: string
  style?: Style
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

/** A path's props. */
export interface PathProps extends Pick<Semantics, "role" | "aria-label" | "aria-hidden"> {
  /** Its shape, as SVG's path data. */
  d: string
  /** The box `d` is drawn in, as SVG's: `"0 0 256 256"`. */
  viewBox: string
  style?: Style
}

/// `<Path viewBox="0 0 100 20" d="M0 20 L50 4 L100 12" style={{ width: "100%", height: 32 }} />`: a vector shape, as
/// SVG's `<path>`, filled with the text's colour, its `viewBox` stretched to its box: an icon, a chart's line or area.
/// `role="img"` with `aria-label` names one that says something.
export function Path(props: PathProps) {
  return (
    <svg viewBox={props.viewBox} preserveAspectRatio="none" role={props.role} aria-label={props["aria-label"]} aria-hidden={props["aria-hidden"] ?? (props.role ? undefined : "true")} {...drawn("ui-svg", props.style)}>
      <path d={props.d} vector-effect="non-scaling-stroke" />
    </svg>
  )
}

/** What an editable text's `onInput` is told: the DOM's `input` event, whose `currentTarget.value` is what's in it now. */
export interface Typed {
  currentTarget: { value: string }
}

/** What an editable text's `onAction` is told: what its action key did. */
export interface Acted {
  action: TextInputAction
}

/** An editable text's props. */
export interface EditableTextProps extends Pick<Semantics, "role" | "aria-label" | "aria-description" | "aria-invalid" | "aria-controls" | "aria-activedescendant" | "aria-expanded"> {
  name?: string
  value?: string
  type?: Props["type"]
  inputmode?: Props["inputmode"]
  multiline?: boolean
  placeholder?: string
  action?: TextInputAction
  autocapitalize?: Props["autocapitalize"]
  autocorrect?: boolean
  spellcheck?: boolean
  autocomplete?: string
  autofocus?: boolean
  disabled?: boolean
  readonly?: boolean
  required?: boolean
  style?: Style
  onInput?: (event: Typed) => void
  onAction?: (event: Acted) => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
}

/** A phone keyboard's word on its action key, by the action. */
const hints: Record<TextInputAction, "enter" | "done" | "go" | "next" | "previous" | "search" | "send"> = { newline: "enter", done: "done", go: "go", next: "next", previous: "previous", search: "search", send: "send" }

/// `<EditableText value={text()} onInput={(event) => setText(event.currentTarget.value)} onAction={save} />`: text
/// to type, bare: the DOM's own `<input>`, or `<textarea>` with `multiline`, so selection, an input method's
/// composing, paste and autofill are the device's own. `action` names its action key (a phone's Done, Go, Search,
/// Send), and `onAction` is told it was pressed: Return in one line, as an input method that's composing doesn't. In
/// several lines Return is a new line unless `action` says otherwise. What a page drawn on the DOM gives it besides
/// (`pattern`, a form's `name`) reaches the element. A `readonly` one is all selected as it's focused, to copy. Its
/// text runs the way its first strong character does (`dir="auto"`), so Hebrew or Arabic starts at the right.
export function EditableText(props: EditableTextProps) {
  const own = omit(props as EditableTextProps & { class?: string }, "style", "class", "multiline", "action", "autocorrect", "autofocus", "onAction", "onKeyDown", "onFocus", "onInput")
  const action = (): TextInputAction => props.action ?? (props.multiline ? "newline" : "done")
  const keyed = (event: KeyboardEvent) => {
    props.onKeyDown?.(event)
    if (event.key !== "Enter" || event.isComposing || event.keyCode === 229 || event.defaultPrevented || !props.onAction || action() === "newline" || event.shiftKey) return
    event.preventDefault()
    props.onAction({ action: action() })
  }
  // What's in it is what the app last set: where the app keeps something else of what was typed (only its digits), the
  // field shows that, as a native host does.
  const typed = (event: InputEvent) => {
    props.onInput?.(event as unknown as Typed)
    const field = event.currentTarget as HTMLInputElement
    queueMicrotask(() => props.value !== undefined && field.value !== props.value && (field.value = props.value))
  }
  const focused = (event: FocusEvent) => {
    if (props.readonly) (event.currentTarget as HTMLInputElement).select()
    props.onFocus?.()
  }
  // A page in a frame of another origin, as an app's is, isn't given `autofocus`: it's focused as it's drawn, and
  // again each time it's set.
  const focus = (element: HTMLElement) =>
    createEffect(
      () => !!props.autofocus,
      (on) => void (on && requestAnimationFrame(() => element.isConnected && element.focus({ preventScroll: true }))),
    )
  const looks = () => drawn("ui-text-input", props.style)
  return (
    <Dynamic
      component={props.multiline ? "textarea" : "input"}
      type={props.multiline ? undefined : (props.type ?? "text")}
      dir="auto"
      enterkeyhint={props.action || !props.multiline ? hints[action()] : undefined}
      autocorrect={props.autocorrect === undefined ? undefined : props.autocorrect ? "on" : "off"}
      autofocus={props.autofocus}
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

const colours = new Set(["backgroundColor", "color", "borderColor"])
const offsets = new Set(["top", "right", "bottom", "left"])
const spaces = new Set(["gap", "rowGap", "columnGap", "padding", "paddingInline", "paddingBlock", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "margin", "marginInline", "marginBlock"])
const sizes = new Set(["flexBasis", "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight"])
const tokens: Record<string, string> = { gutter: "var(--gutter)", control: "var(--control)", icon: "var(--icon)", readable: "var(--readable)" }

/** A version 1 style as CSS: its tokens as the custom properties they name, its pixels as `px`. Its `font` is the
 * element's `data-font` (ui.css). */
export function css(style: Version1Style | undefined): JSX.CSSProperties | undefined {
  if (!style) return
  const out: Record<string, string | number> = {}
  for (const [name, value] of Object.entries(style) as [keyof Version1Style, string | number][]) {
    const property = name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
    if (name === "font" || value === undefined) continue
    else if (name === "translateX" || name === "translateY") out.translate = `${style.translateX ?? 0}px ${style.translateY ?? 0}px`
    else if (name === "rotate") out.rotate = `${value}deg`
    else if (offsets.has(name)) out[property] = `${value}px`
    else if (colours.has(name)) out[property] = `var(--${value})`
    else if (name === "borderRadius") out[property] = value === "full" ? "9999px" : value ? `var(--radius-${value})` : 0
    else if (name.startsWith("border")) out[property] = `${value}px`
    // What scrolls has no scrollbar drawn beside it, as on a phone and on a native host, which draw one over it as it moves.
    else if (name === "overflow") Object.assign(out, value === "scroll" ? { overflow: "auto", "scrollbar-width": "none" } : { overflow: value })
    else if (name === "lineClamp") Object.assign(out, { display: "-webkit-box", "-webkit-box-orient": "vertical", "-webkit-line-clamp": value, overflow: "hidden" })
    else if (spaces.has(name) || sizes.has(name)) out[property] = typeof value === "number" ? `${value}px` : (tokens[value] ?? value)
    else out[property] = value
  }
  return out
}
