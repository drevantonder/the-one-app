// The Vault's page at a network's address, where an outside service sends whoever signed in to it back: it says
// whether that's kept in their Vault now, and opens the Vault from there.
import { createSignal } from "solid-js"
import { render } from "@solidjs/web"
import mark from "../../../../icons/Logo.png"
import { SignIn } from "ui"

/// What the server said of the sign-in: `{ done }` or `{ problem }`.
const [result, setResult] = createSignal<{ done?: string; problem?: string }>()
fetch(location.href, { method: "POST" })
  .then((response) => response.json())
  .then(setResult, () => setResult({ problem: "Couldn't reach the network. Start the sign-in again." }))

render(
  () => (
    <SignIn
      mark={<img src={mark} alt="" />}
      heading={result()?.done ? "Signed in" : result()?.problem ? "Not signed in" : "Signing in"}
      status={result() ? undefined : "Signing in…"}
      problem={result()?.problem}
      button="Open Vault"
      href="/?open=vault"
    >
      {result()?.done && <p class="done">{result()!.done}</p>}
    </SignIn>
  ),
  document.body,
)
