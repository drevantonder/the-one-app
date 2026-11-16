// The Vault's page at a network's address. Someone an outside service sent back, once they signed in to it, is
// told whether that's kept in their Vault now. Someone who opened a request's link is told where it's answered:
// on a device of theirs, which is the only place a secret is typed.
import { createSignal } from "solid-js"
import { render } from "@solidjs/web"
import { SignIn } from "ui"

const callback = location.pathname === "/vault/callback"
/// What the server said of the sign-in: `{ done }` or `{ problem }`.
const [said, setSaid] = createSignal<{ done?: string; problem?: string }>()
if (callback)
  fetch(location.href, { method: "POST" })
    .then((answer) => answer.json())
    .then(setSaid, () => setSaid({ problem: "Couldn't reach the network. Start the sign-in again." }))

render(
  () => (
    <SignIn
      mark={<img src="./mark.png" alt="" />}
      heading={!callback ? "Open Vault" : said()?.done ? "Signed in" : said()?.problem ? "Not signed in" : "Signing in"}
      done={callback ? said()?.done : "Requests for your keys and sign-ins are answered in Vault, on a device of yours."}
      problem={said()?.problem}
    />
  ),
  document.body,
)
