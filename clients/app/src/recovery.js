// The client's own page where a person's recovery key is shown, or typed to let this device in, as an Apple device
// shows one in its own system's UI and never in an app. An app's page only asks for it (`showRecoveryKey`,
// `useRecoveryKey`) and hears whether it was done; it's drawn in a window of the client's own over the app's, and only
// this page's host answers what it asks here (`one/recovery`, in native.rs), so the key never reaches an app's page.
// On the web, the window page's own dialog does the same (`recoveryKey` in page/client.js), in the same words.
import { createComponent as h, createSignal, Show } from "/ui/client/solid.js"
import { Button, Input, render, Screen, Section, Text } from "/ui/client/native/ui.js"
import { ask, cancelOnEscape } from "/client/ask.js"

const { typed } = await ask("one/recovery")
const [key, setKey] = createSignal("")
const [entry, setEntry] = createSignal("")
const [wrong, setWrong] = createSignal("")
const [approving, setApproving] = createSignal(false)
const done = (done) => ask("one/recovery/done", { done })
cancelOnEscape(() => done(false))

if (!typed) ask("one/recovery/key").then(setKey, (error) => setWrong(error.message))

async function approve() {
  setApproving(true)
  try {
    await ask("one/recovery/recover", { key: entry() })
    await done(true)
  } catch (error) {
    setWrong(error.message)
  } finally {
    setApproving(false)
  }
}

const shown = () => [
  h(Text, { children: "Write it down, and keep it off your devices. It approves a new device when no trusted device is left." }),
  h(Text, { get children() { return key() } }),
  h(Show, { get when() { return wrong() }, get children() { return h(Text, { role: "alert", get children() { return wrong() } }) } }),
  h(Button, { onPress: () => done(true), children: "Done" }),
]

const asking = () => [
  h(Text, { children: "Type it as you wrote it down. This device will be trusted." }),
  h(Input, {
    "aria-label": "Recovery Key",
    autoFocus: true,
    autoCapitalize: "characters",
    autoCorrect: false,
    spellCheck: false,
    autoComplete: "off",
    get value() { return entry() },
    onChangeText: setEntry,
    onSubmitEditing: approve,
  }),
  h(Show, { get when() { return wrong() }, get children() { return h(Text, { role: "alert", get children() { return wrong() } }) } }),
  h(Button, { get loading() { return approving() }, get disabled() { return !entry().trim() }, onPress: approve, children: "Approve This Device" }),
  h(Button, { variant: "outline", onPress: () => done(false), children: "Cancel" }),
]

// Its window's title bar says what it is, so the page doesn't say it again.
render(() => h(Screen, { get children() { return h(Section, { get children() { return typed ? asking() : shown() } }) } }))
