// The launcher's own page while this device isn't in its network, where it signs in, as the network's sign-in page has
// a device in the app do (`deviceInApp` in apps/sign-in/edge/page/page.tsx), in the same words. Its person names whose
// device it is, which asks their devices, and is shown the number to enter on one of them (`one/sign-in/claim`); or uses
// a passkey in the device's browser, where theirs is (`one/sign-in/browser`); or opens its link on another device where
// they're signed in. The device learns it's in by itself, and the Home Screen takes this page's place then.
import { createComponent as h, createSignal } from "/ui/client/solid.js"
import { Field, Input, render, SignIn } from "/ui/client/native/ui.js"
import { ask } from "/client/ask.js"

const { typed, code, why, mark } = await ask("one/sign-in")
/// Why it shows a new code, as the device says it (`NotApproved` in core/client/src/auth.rs).
const said = { denied: "It was denied where you signed in.", expired: "Its code expired before it was approved. Try again.", removed: "This device was removed from your network." }[why]
/// How it signs in: by whose device it is, by a passkey with its code, or with its link opened elsewhere.
const [how, setHow] = createSignal("named")
const [person, setPerson] = createSignal("")
/// The number its person enters on one of their devices, once they're asked.
const [number, setNumber] = createSignal()
const [copied, setCopied] = createSignal(false)

async function name() {
  if (!person().trim()) throw new Error("Say who's signing in first.")
  const { number } = await ask("one/sign-in/claim", { person: person() })
  setNumber(number)
}

async function copy() {
  await ask("one/copy", { text: typed })
  setCopied(true)
}

/// What each way shows, as `SignIn` takes it.
function shows() {
  if (how() === "away")
    return {
      heading: "Sign in on another device",
      note: "Open this link where you're signed in. It will show",
      code,
      button: copied() ? "Copied" : "Copy Link",
      onSignIn: copy,
      stop: "Back",
      onStop: () => setHow("passkey"),
      children: h(Input, { "aria-label": "Link", readOnly: true, value: typed }),
    }
  if (how() === "passkey") return { heading: "Sign in", note: "Check this code where you sign in", code, problem: said, button: "Sign in", onSignIn: () => ask("one/sign-in/browser"), stop: "Use Another Device", onStop: () => setHow("away") }
  if (number() !== undefined)
    return { heading: "Check your devices", note: "On another of your devices, open Devices and enter", code: String(number()), status: "Waiting for approval…", quiet: true, button: "Use a Passkey Instead", onSignIn: () => setHow("passkey"), stop: "Back", onStop: () => setNumber() }
  return {
    heading: "Sign in",
    problem: said,
    button: "Continue",
    onSignIn: name,
    stop: "Use a Passkey",
    onStop: () => setHow("passkey"),
    children: h(Field, {
      get children() {
        return h(Input, { "aria-label": "Who's signing in?", autoFocus: true, autoComplete: "username", autoCapitalize: "none", placeholder: "Name or email", get value() { return person() }, onChangeText: setPerson })
      },
    }),
  }
}

const prop = (name) => ({ get: () => shows()[name], enumerable: true })
render(() => h(SignIn, Object.defineProperties({ mark }, Object.fromEntries(["heading", "note", "code", "problem", "status", "quiet", "button", "onSignIn", "stop", "onStop", "children"].map((name) => [name, prop(name)])))))
