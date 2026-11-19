// A toast: one line about what just happened, at the foot of the window for a few seconds. Written once on the drawing
// primitives, the same on the DOM and on a native host: a portal over the window (#primitives' `Portal`), drawn
// while there are toasts, so it's over whatever was open when the first came.
import { createRoot, createSignal, For, Show } from "solid-js"
import { createToaster } from "../headless-components/toast.ts"
import { Portal, Text, View } from "#primitives"
import { Button } from "./button.tsx"
import { Sheet } from "./sheet.tsx"
import { Details } from "./states.tsx"
import { messages, readable, whole } from "../words.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

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
  // A pill, as Raycast's toasts are: one line, its action after a hairline.
  toast: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[2],
    maxWidth: "100%",
    minHeight: tokens.length.pill,
    paddingStart: tokens.grid[3],
    paddingEnd: tokens.grid[1],
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.panel,
    backdropFilter: tokens.material.glass,
    color: tokens.colour.label,
    font: tokens.type.medium,
    boxShadow: tokens.shadow.window,
    pointerEvents: "auto",
    opacity: { default: 1, entering: 0, exiting: 0 },
    translateY: { default: 0, entering: tokens.grid[4] },
    transitionProperty: ["opacity", "translateY"],
    transition: tokens.motion.settle,
  },
  alone: { paddingEnd: tokens.grid[3] },
  rule: { width: tokens.length.hairline, alignSelf: "stretch", marginBlock: tokens.grid[2], backgroundColor: tokens.colour.quaternaryLabel },
  title: { flexShrink: 1, minWidth: 0, lineClamp: 1 },
})

let toaster: ReturnType<typeof createToaster> | undefined

/// `toast("Deleted", { action: { label: "Undo", run: restore } })`: one line about what just happened, at the foot
/// of the window for a few seconds, with one thing to do about it. What went well needs none: fine is silent.
/// An error, as `call` throws it (`.catch(toast)`), or a tool's error as text, says what a person reads of it
/// (`readable`), and with no `action` of its own has Details, which shows the rest, to copy for an agent or for support.
export async function toast(text: string | Error, { action }: { action?: { label: string; run: () => void } } = {}): Promise<void> {
  const all = whole(text)
  if (all !== readable(all)) action ??= { label: messages.details, run: () => details(all) }
  if (!toaster) {
    const made = (toaster = createRoot(() => createToaster()))
    createRoot(() => <Toaster toaster={made} />)
  }
  toaster.show(readable(all), action)
}

function Toaster(props: { toaster: ReturnType<typeof createToaster> }) {
  return (
    <Show when={props.toaster.toasts().length}>
      <Portal>
        <View style={look.stack}>
          <For each={props.toaster.toasts()} keyed={(one) => one.id}>
            {(one) => (
              <View {...props.toaster.region()} style={[look.toast, !one().action && look.alone]}>
                <Text style={look.title}>{one().title}</Text>
                <Show when={one().action}>
                  {(action) => (
                    <>
                      <View style={look.rule} />
                      <Button variant="ghost" onPress={() => (props.toaster.dismiss(one().id), action().run())}>
                        {action().label}
                      </Button>
                    </>
                  )}
                </Show>
              </View>
            )}
          </For>
        </View>
      </Portal>
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
