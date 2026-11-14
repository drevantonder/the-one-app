// The sign-in page. It asks one thing, the passkey, and what that lets happen depends on how the page was
// opened: with an invitation, someone joins; with a device's key, that device becomes theirs too; from an
// assistant's request to be let in, the assistant does. Whichever it was, this browser is signed in after, and is
// one of that person's devices. In the app's own window, which has no passkeys, its button opens the page in the
// device's browser instead.
import { createSignal } from "solid-js"
import { render } from "@solidjs/web"
import { SignIn } from "ui"
import { createAuthClient } from "better-auth/client"
import { passkeyClient } from "@better-auth/passkey/client"

const auth = createAuthClient({ basePath: "/sign-in/auth", plugins: [passkeyClient()] })
const opened = new URLSearchParams(location.hash.slice(1))
const asked = new URLSearchParams(location.search)
/// What Better Auth carries an assistant's request in while a person answers it. It's signed, so it goes back as it came.
const request = asked.has("client_id") ? location.search.slice(1) : null
const invite = opened.get("invite")
const device = opened.get("device")
const app = asked.has("app")

/// What this page asks Better Auth besides the passkey.
async function ask(path, body) {
  const answer = await fetch(`/sign-in/auth${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
  const said = await answer.json()
  if (!answer.ok) throw new Error(said.message ?? "That didn't work. Try again.")
  return said
}

/// Who will act as whoever signs in: the assistant that asked, or the device this was opened for.
const [acts, setActs] = createSignal(device && !app ? opened.get("name") || "A new device" : undefined)
const [done, setDone] = createSignal()
if (request) ask("/oauth2/public-client-prelogin", { client_id: asked.get("client_id"), oauth_query: request }).then((client) => setActs(client.client_name))

/// The passkey question, and what answering it lets in. Someone joining makes their passkey, and is signed in
/// with it. Whoever was signed in here before no longer is.
async function signIn() {
  await auth.signOut()
  const { error } = invite ? await auth.passkey.addPasskey({ context: invite, createSession: true }) : await auth.signIn.passkey()
  if (error) throw new Error(error.message ?? "That passkey didn't work. Try again.")
  if (request) {
    // Signing in was saying yes, so the assistant is sent back with what it came for.
    const { url } = await ask("/oauth2/consent", { accept: true, oauth_query: request })
    return location.replace(url)
  }
  if (device) await ask("/device/add", { device, name: acts() })
  setDone(device ? `${acts()} is yours now. Go back to it.` : "You're signed in.")
}

render(
  () => (
    <SignIn
      mark={<img src="./mark.png" alt="" />}
      heading={invite ? "Join" : "Sign in"}
      warning={acts() && `${acts()} will act as you. Sign in only if you asked for this.`}
      button={invite ? "Create a passkey" : "Sign in"}
      href={app ? `./${location.hash}` : undefined}
      onSignIn={signIn}
      done={done()}
    />
  ),
  document.body,
)
