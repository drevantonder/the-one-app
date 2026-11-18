// A toast: one line about what just happened, at the foot of the window for a few seconds. Written once in strict, on
// the primitives, the same on the DOM and on a native host: a layer over the window (#primitives' `Layer`), drawn
// while there are toasts, so it's over whatever was open when the first came.
import { createRoot, createSignal, For, Show } from "solid-js"
import { Box, Layer, Text } from "#primitives"
import { Button } from "./button.tsx"
import { Sheet } from "./sheet.tsx"
import { Details } from "./states.tsx"
import { messages, readable, whole } from "./words.ts"
import { styles } from "./styles.ts"
import { duration, tokens } from "./tokens.ts"

const look = styles({
  stack: {
    position: "absolute",
    start: 0,
    end: 0,
    bottom: tokens.length.safeBottom,
    alignItems: "center",
    gap: tokens.grid[2],
    padding: tokens.length.gutter,
  },
  toast: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[3],
    width: "100%",
    maxWidth: tokens.length.column,
    minHeight: tokens.grid[12],
    paddingBlock: tokens.grid[1],
    paddingStart: tokens.grid[4],
    paddingEnd: tokens.grid[2],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.card,
    color: tokens.colour.label,
    font: tokens.type.medium,
    boxShadow: tokens.shadow.window,
    pointerEvents: "auto",
    opacity: { default: 1, entering: 0 },
    translateY: { default: 0, entering: tokens.grid[4] },
    transitionProperty: ["opacity", "translateY"],
    transition: tokens.motion.settle,
  },
  leaving: { opacity: 0 },
  title: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
})

/** At most this many show at once; the oldest goes for a new one. */
const MOST = 3
/** How long a toast takes to fade as it goes. */
const fading = duration(tokens.motion.settle)

type Toast = { id: number; title: string; action?: { label: string; run: () => void }; leaving?: boolean }

const [toasts, setToasts] = createSignal<Toast[]>([])
let made = 0
let drawn = false

const dismiss = (id: number) => {
  setToasts((now) => now.map((one) => (one.id === id ? { ...one, leaving: true } : one)))
  setTimeout(() => setToasts((now) => now.filter((one) => one.id !== id)), fading)
}

/// `toast("Deleted", { action: { label: "Undo", run: restore } })`: one line about what just happened, at the foot
/// of the window for a few seconds, with one thing to do about it. What went well needs none: fine is silent.
/// An error, as `call` throws it (`.catch(toast)`), or a tool's error as text, says what a person reads of it
/// (`readable`), and with no `action` of its own has Details, which shows the rest, to copy for an agent or for support.
export async function toast(text: string | Error, { action }: { action?: { label: string; run: () => void } } = {}): Promise<void> {
  const all = whole(text)
  if (all !== readable(all)) action ??= { label: messages.details, run: () => details(all) }
  if (!drawn) {
    drawn = true
    createRoot(() => <Toaster />)
  }
  const id = ++made
  const showing = toasts().filter((one) => !one.leaving)
  if (showing.length >= MOST) dismiss(showing[0].id)
  setToasts((now) => [...now, { id, title: readable(all), action }])
  setTimeout(() => dismiss(id), action ? 8000 : 4000)
}

function Toaster() {
  return (
    <Show when={toasts().length}>
      <Layer>
        <Box style={look.stack}>
          <For each={toasts()} keyed={(one) => one.id}>
            {(one) => (
              <Box role="status" aria-live="polite" style={[look.toast, one().leaving && look.leaving]}>
                <Text style={look.title}>{one().title}</Text>
                <Show when={one().action}>
                  {(action) => (
                    <Button variant="ghost" onClick={() => (dismiss(one().id), action().run())}>
                      {action().label}
                    </Button>
                  )}
                </Show>
              </Box>
            )}
          </For>
        </Box>
      </Layer>
    </Show>
  )
}

let detail: ((text: string) => void) | undefined

/// Shows all of a tool's error in a sheet: what a person reads of it, then the rest and Copy. What a toast's Details
/// opens.
export function details(text: string): void {
  if (!detail) {
    const [shown, setShown] = createSignal<string>()
    const [open, setOpen] = createSignal(false)
    createRoot(() => (
      <Sheet heading={messages.details} open={open()} onClose={() => setOpen(false)}>
        <Show when={shown()}>
          {(text) => (
            <>
              <Text>{readable(text())}</Text>
              <Details text={text()} />
            </>
          )}
        </Show>
      </Sheet>
    ))
    detail = (text) => (setShown(text), setOpen(true))
  }
  detail(text)
}
