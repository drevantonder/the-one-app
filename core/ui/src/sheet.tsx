// What's over the page, as a system's own: a sheet, up from the foot of a narrow window and in the middle of a wide
// one, and an alert, a question that stops everything until it's answered. Written once in strict, on the
// primitives, the same on the DOM and on a native host: each a modal layer (#primitives' `Layer`) over the window.
import { children, createEffect, createRoot, createSignal, onCleanup, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Layer, Scroll, Text } from "#primitives"
import { covers, touch, viewport } from "#bridge"
import { Button, Icon } from "./button.tsx"
import { messages } from "./words.ts"
import { Collect } from "./collect.ts"
import { negative, styles } from "./styles.ts"
import { duration, tokens } from "./tokens.ts"
import type { Pressed } from "./strict/types/ui.d.ts"

/** How long what's over the page takes to go, in milliseconds: the travel pace's (tokens.ts). */
const going = duration(tokens.motion.travel)

/** Where a sheet is in the middle of the window, not at its foot: a window 40rem wide or more. */
const wide = () => viewport().width >= 640

const look = styles({
  scrim: {
    position: "absolute", top: 0, bottom: 0, start: 0, end: 0, backgroundColor: tokens.colour.overlay, pointerEvents: "auto",
    opacity: { default: 1, entering: 0 }, transitionProperty: ["opacity"], transition: tokens.motion.travel,
  },
  gone: { opacity: 0 },
  place: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, alignItems: "stretch", justifyContent: "flex-end" },
  middle: { alignItems: "center", justifyContent: "center", padding: tokens.grid[4] },
  panel: {
    maxHeight: "92%", backgroundColor: tokens.colour.background, color: tokens.colour.label, boxShadow: tokens.shadow.window,
    borderTopStartRadius: tokens.radius.large, borderTopEndRadius: tokens.radius.large, pointerEvents: "auto", overflow: "hidden",
    translateY: { default: 0, entering: "100%" }, transitionProperty: ["translateY", "opacity", "scale"], transition: tokens.motion.travel,
  },
  down: { translateY: "100%" },
  floating: { width: "100%", maxWidth: tokens.length.sheet, borderRadius: tokens.radius.window, translateY: 0, opacity: { default: 1, entering: 0 }, scale: { default: 1, entering: 0.97 } },
  // On a computer, a panel of glass over the window, as Raycast's.
  glass: { backgroundColor: tokens.colour.panel, backdropFilter: tokens.material.glass },
  shrunk: { opacity: 0, scale: 0.97 },
  pulled: { transitionProperty: [] },
  grabber: { alignSelf: "center", width: tokens.grid[10], height: tokens.length.grabber, marginTop: tokens.length.grabberGap, borderRadius: tokens.radius.full, backgroundColor: tokens.colour.quaternaryLabel },
  header: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], paddingBlock: tokens.grid[2], paddingStart: tokens.length.gutter, paddingEnd: tokens.grid[2] },
  heading: { flexGrow: 1, minWidth: 0, font: tokens.type.large, fontWeight: tokens.weight.semibold, lineClamp: 2 },
  computerHeading: { font: tokens.type.barTitle, fontWeight: tokens.weight.semibold },
  centred: { textAlign: "center" },
  body: { gap: tokens.length.gutter, paddingTop: tokens.grid[1], paddingInline: tokens.length.gutter, paddingBottom: tokens.length.gutter },
  alert: { width: "100%", maxWidth: tokens.length.alert, gap: tokens.grid[2.5], paddingTop: tokens.grid[5], paddingInline: tokens.grid[4], paddingBottom: tokens.grid[4], borderRadius: tokens.radius.window },
  text: { color: tokens.colour.secondaryLabel },
  footer: { flexDirection: "row", flexWrap: "wrap-reverse", gap: tokens.grid[2], marginTop: tokens.grid[1.5] },
  half: { flexGrow: 1, flexBasis: "45%" },
})

/// What a sheet and an alert share: shown over the window in a modal layer while `open`, with what's around it
/// dimmed, kept until it's gone as it closes; Escape, the device's back and a press on what's around it close it,
/// each calling `onClose`.
function Over(props: { open?: boolean; onClose?: () => void; children: (leaving: () => boolean) => JSX.Element }) {
  const [shown, setShown] = createSignal(false)
  const [leaving, setLeaving] = createSignal(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  createEffect(
    () => !!props.open,
    (open) => {
      clearTimeout(timer)
      if (open) return void (setLeaving(false), setShown(true))
      if (!shown()) return
      setLeaving(true)
      timer = setTimeout(() => (setShown(false), setLeaving(false)), going)
    },
  )
  // The device's back closes what's on top, as it would go back.
  createEffect(
    () => !!props.open,
    (open) => {
      if (!open) return
      const shut = covers(() => props.onClose?.())
      return shut
    },
  )
  onCleanup(() => clearTimeout(timer))
  return (
    <Show when={shown()}>
      <Layer modal>
        <Box style={[look.scrim, leaving() && look.gone]} onClick={() => props.onClose?.()} />
        {/* What's over the page isn't the bar's, wherever it's written. */}
        <Collect value={null}>{props.children(leaving)}</Collect>
      </Layer>
    </Show>
  )
}

/// `<Sheet heading="Add note" open={adding()} onClose={() => setAdding(false)}>...</Sheet>`: something to fill in
/// or look at, over the page: up from the foot of a narrow window, in the middle of a wide one, while `open`.
/// Escape, the device's back, a press outside and a pull down close it too: each calls `onClose`, which sets
/// `open` false. `autofocus` on a field in it brings up a phone's keyboard as it opens.
export function Sheet(props: { heading?: JSX.Element; open?: boolean; onClose?: () => void; children?: JSX.Element }) {
  // A finger pulling it down: where it started, how far it's gone, and its last moves, for how fast it was let go.
  const [pull, setPull] = createSignal<{ y: number; down: number; height: number; started: number; moves: { time: number; down: number }[] }>()
  const press = (event: Pressed) => {
    if (wide() || event.pointerType === "mouse") return
    setPull({ y: event.clientY, down: 0, height: event.currentTarget.getBoundingClientRect().height, started: event.timeStamp, moves: [] })
  }
  const move = (event: Pressed) => {
    const was = pull()
    if (!was) return
    const down = Math.max(0, event.clientY - was.y)
    setPull({ ...was, down, moves: [...was.moves.slice(-31), { time: event.timeStamp, down }] })
  }
  /// Let go a third of the way down, or flicked down, it closes; otherwise it goes back.
  const lift = (event: Pressed) => {
    const was = pull()
    setPull(undefined)
    if (!was) return
    const last = was.moves.at(-1)
    const from = was.moves.findLast((move) => last!.time - move.time >= 80) ?? was.moves[0]
    const speed = !last || event.timeStamp - last.time > 80 ? 0 : (last.down - from.down) / Math.max(1, last.time - from.time)
    const flicked = speed > 0 && was.down / Math.max(1, event.timeStamp - was.started) > 0.11
    if (was.down > was.height / 3 || flicked) props.onClose?.()
  }
  const keys = (event: { key: string }) => event.key === "Escape" && props.onClose?.()
  return (
    <Over open={props.open} onClose={props.onClose}>
      {(leaving) => (
        <Box style={[look.place, wide() && look.middle]}>
          <Box
            role="dialog"
            aria-modal="true"
            tabindex={-1}
            aria-label={typeof props.heading === "string" ? props.heading : undefined}
            onKeyDown={keys}
            onPointerDown={press}
            onPointerMove={move}
            onPointerUp={lift}
            onPointerCancel={() => setPull(undefined)}
            style={[look.panel, wide() && look.floating, !touch() && look.glass, leaving() && (wide() ? look.shrunk : look.down), pull() && look.pulled, pull() && { translateY: pull()!.down }]}
          >
            <Show when={!wide()}>
              <Box aria-hidden="true" style={look.grabber} />
            </Show>
            <Box role="banner" style={look.header}>
              <Text role="heading" aria-level={2} style={[look.heading, !touch() && look.computerHeading]}>
                {props.heading}
              </Text>
              <Button variant="ghost" size="icon" aria-label={messages.close} onClick={() => props.onClose?.()}>
                <Icon name="x" />
              </Button>
            </Box>
            <Scroll style={look.body}>{props.children}</Scroll>
          </Box>
        </Box>
      )}
    </Over>
  )
}

/// `<Alert heading="Vault wants to use a key" text="Sent only to api.github.com." open={!!asking()} onClose={() => setAsking()} action={<><Button variant="outline" onClick={decline}>Don't Allow</Button><Button onClick={allow}>Allow</Button></>}>...</Alert>`:
/// a question that stops everything until it's answered, as a system's permission prompt does, in the middle of the
/// window while `open`: its `heading`, its grey `text`, what's chosen or typed for it, then its buttons (`action`),
/// sharing the row while two fit, and otherwise one over the other, the first below, as iOS lays them out.
/// `confirm` asks a question with only Cancel and OK; this is for one that needs more. Escape and the device's back
/// close it too, which calls `onClose`, and the app sets `open` false.
export function Alert(props: { heading: JSX.Element; text?: JSX.Element; open?: boolean; onClose?: () => void; action?: JSX.Element; children?: JSX.Element }) {
  const actions = children(() => props.action)
  return (
    <Over open={props.open} onClose={props.onClose}>
      {(leaving) => (
        <Box style={[look.place, look.middle]}>
          <Box
            role="alertdialog"
            aria-modal="true"
            tabindex={-1}
            aria-label={typeof props.heading === "string" ? props.heading : undefined}
            onKeyDown={(event) => event.key === "Escape" && props.onClose?.()}
            style={[look.panel, look.floating, !touch() && look.glass, look.alert, leaving() && look.shrunk]}
          >
            <Text role="heading" aria-level={2} style={[look.heading, !touch() && look.computerHeading, look.centred]}>
              {props.heading}
            </Text>
            <Show when={props.text}>
              <Text style={[look.text, look.centred]}>{props.text}</Text>
            </Show>
            {props.children}
            <Show when={props.action}>
              <Box style={look.footer}>{actions.toArray().map((action) => <Box style={look.half}>{action}</Box>)}</Box>
            </Show>
          </Box>
        </Box>
      )}
    </Over>
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
    const [asked, setAsked] = createSignal<Question>()
    const [open, setOpen] = createSignal(false)
    let answer: (yes: boolean) => void = () => {}
    const answered = (yes: boolean) => (setOpen(false), answer(yes))
    createRoot(() => (
      <Alert
        heading={asked()?.heading}
        text={asked()?.text}
        open={open()}
        onClose={() => answered(false)}
        action={
          <>
            <Button variant="secondary" onClick={() => answered(false)}>
              {messages.cancel}
            </Button>
            <Button variant={asked()?.destructive ? "destructive" : "default"} onClick={() => answered(true)}>
              {asked()?.action || messages.ok}
            </Button>
          </>
        }
      />
    ))
    ask = (question) => new Promise<boolean>((done) => ((answer = done), setAsked(question), setOpen(true)))
  }
  return ask(question)
}

