// The Vault's page at a network's address, where an outside service sends whoever signed in to it back. It ends the
// sign-in as people expect from where it began (`@sign_in_moves` in `apps/vault/server/vault.ex`): begun on an app's
// screen, it goes back to that screen; begun by an agent or `one`, whose link was opened in a tab of its own, it says
// they're done there and to go back to it, as a CLI's or an MCP client's sign-in does. It never sends them to the
// agent's site: the network doesn't know the conversation's address, and a page that sends people where it's told
// to is an open redirect. Cancelled or failed, it says so, with Try Again.
import { createSignal, For, Show } from "solid-js"
import { render } from "@solidjs/web"
import mark from "../../../../icons/Logo.png"
import { Icon, SignIn } from "ui"
import "./page.css"

/// Where a sign-in began, as the server kept it: an app's screen, an agent by its own name, or `one` in a terminal.
type Began = { screen: string } | { agent: string } | { cli: true }
/// What the server said of it: how it ended, and where it began, or only a `problem` when it can't say whose it is.
type Ended = {
  service?: string
  status?: "started" | "ok" | "cancelled" | "failed"
  began?: Began
  apps?: string[]
  item?: string
  account?: string
  problem?: string
  url?: string
}

/// The page: finishing it, then how it ended.
const [ended, setEnded] = createSignal<Ended>()
const ask = (body: object) =>
  fetch(location.href, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
    .then((response) => response.json() as Promise<Ended>)
    .catch((): Ended => ({ problem: "Couldn't reach the network. Start the sign-in again." }))

ask({}).then((answer) => {
  setEnded(answer)
  // A tab a script opened, as the Vault's screen opens a service's page, closes, which takes them back to where they
  // were; any other stays, saying what to do.
  if (answer.status === "ok") window.close()
})

/// Try Again: the server starts it again as it began, and this tab goes on to the service's page.
async function again() {
  const answer = await ask({ again: true })
  if (!answer.url) throw new Error(answer.problem ?? "It couldn't start again.")
  location.assign(answer.url)
}

/// Where they go back to, as a sentence's end, for what began elsewhere than an app's screen.
const back = (began?: Began) => (began && "agent" in began ? began.agent : began && "cli" in began ? "your terminal" : undefined)
/// An app's screen, by its app, and how its button names it.
const screen = (began?: Began) => (began && "screen" in began ? began.screen : undefined)
/// An app's name, as its button says it, from a link to it (`settings#vault/accounts` is Settings).
const titled = (app: string) => app[0].toUpperCase() + app.split("#")[0].slice(1)
/// "Gmail, Google Calendar and Google Drive".
const listed = (things: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(things)

render(() => {
  const status = () => ended()?.status
  const began = () => ended()?.began
  const after = () => back(began())
  const app = () => screen(began())
  return (
    <SignIn
      mark={mark}
      heading={!ended() ? "Signing in" : status() === "ok" ? "You're signed in" : status() === "cancelled" ? "Sign-in cancelled" : "Not signed in"}
      status={ended() ? undefined : "Signing in…"}
      problem={status() !== "cancelled" ? ended()?.problem : undefined}
      button={status() === "ok" ? app() && `Open ${titled(app()!)}` : began() && "Try Again"}
      onSignIn={status() === "ok" && app() ? () => location.assign(`/?open=${encodeURIComponent(app()!)}`) : again}
      stop={status() !== "ok" && app() ? `Open ${titled(app()!)}` : undefined}
      onStop={() => location.assign(`/?open=${encodeURIComponent(app()!)}`)}
    >
      <Show when={status() === "ok"}>
        <section class="connected">
          <h2>{ended()!.account ?? ended()!.item} is connected to</h2>
          <ul>
            <For each={ended()!.apps?.length ? ended()!.apps : [ended()!.service!]}>
              {(name) => (
                <li>
                  <Icon name="check" />
                  {name}
                </li>
              )}
            </For>
          </ul>
        </section>
      </Show>
      <Show when={status() === "cancelled"}>
        <p class="said">
          You didn't let {listed(ended()!.apps?.length ? ended()!.apps! : ["the One App"])} use your {ended()!.service} account.
        </p>
      </Show>
      <Show when={ended() && after()}>
        <p class="note">You can close this tab and go back to {after()}.</p>
      </Show>
    </SignIn>
  )
}, document.body)
