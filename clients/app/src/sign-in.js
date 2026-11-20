// The launcher's own page while this device isn't in its network, where it signs in, as the network's sign-in page has
// a device in the app do (`deviceInApp` in apps/sign-in/edge/page/page.tsx), in the same words. Its person uses a
// passkey, or Google, in the device's browser, where theirs is (`one/sign-in/browser`); or opens its link on another
// device where they're signed in. The device learns it's in by itself, and the Home Screen takes this page's place then.
import { createComponent as h, createSignal } from "/ui/client/solid.js"
import { Input, leave, render, SignIn } from "/ui/client/native/ui.js"
import { ask, cancelOnEscape } from "/client/ask.js"

cancelOnEscape(leave)
const { typed, code, why, mark } = await ask("one/sign-in")
/// Why it shows a new code, as the device says it (`NotApproved` in core/client/src/auth.rs).
const said = { denied: "It was denied where you signed in.", expired: "Its code expired before it was approved. Try again.", removed: "This device was removed from your network." }[why]
/// How it signs in: in the browser with its code, or with its link opened elsewhere.
const [how, setHow] = createSignal("browser")
const [copied, setCopied] = createSignal(false)

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
      onStop: () => setHow("browser"),
      children: h(Input, { "aria-label": "Link", readOnly: true, value: typed }),
    }
  return { heading: "Sign in", note: "Check this code where you sign in", code, problem: said, button: "Sign in", onSignIn: () => ask("one/sign-in/browser"), stop: "Use Another Device", onStop: () => setHow("away") }
}

const prop = (name) => ({ get: () => shows()[name], enumerable: true })
render(() => h(SignIn, Object.defineProperties({ mark }, Object.fromEntries(["heading", "note", "code", "problem", "status", "quiet", "button", "onSignIn", "stop", "onStop", "children"].map((name) => [name, prop(name)])))))
