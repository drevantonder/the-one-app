// The sign-in page. It asks for the passkey, and what that lets happen depends on how the page was opened: with an
// invitation, someone joins; from an assistant's request to be let in, the assistant is. Opened with a new device's
// code, as that device's link has it, its person signs in if they aren't yet, and is shown the code to compare
// with the device's before they let it in or stop it. Opened from the app's Passkeys, someone makes another passkey. Whichever it was, this browser is signed in after, and is one
// of that person's devices. In the app's own window, which has no passkeys, the page shows the device's code and its
// button opens the page in the device's browser instead.
import { createSignal } from "solid-js"
import { render } from "@solidjs/web"
import { Field, Input, SignIn } from "ui"
import { createAuthClient } from "better-auth/client"
import { deviceAuthorizationClient } from "better-auth/client/plugins"
import { passkeyClient } from "@better-auth/passkey/client"
import Bowser from "bowser"

const auth = createAuthClient({ basePath: "/sign-in/auth", plugins: [passkeyClient(), deviceAuthorizationClient()] })
const opened = new URLSearchParams(location.hash.slice(1))
const asked = new URLSearchParams(location.search)
/// What Better Auth carries an assistant's request in while a person answers it. It's signed, so it goes back as it came.
const request = asked.has("client_id") ? location.search.slice(1) : null
const invite = opened.get("invite")
/// What lets someone who has joined make another passkey, as the app's Passkeys opens this page with.
const another = opened.get("passkey")
/// The code a new device shows, which its link carries.
const code = asked.get("user_code")
const app = asked.has("app")
/// Why the app shows a new code, in the words the device says it in (`NotApproved` in `core/client/src/lib.rs`).
const why = app && { denied: "It was denied where you signed in.", expired: "Its code expired before it was approved. Try again." }[asked.get("why")]
/// A code as it's shown, in two halves: WDJB-MJHT.
const shown = code && code.replace(/(.{4})(?=.)/g, "$1-")

/// What this page asks Better Auth besides the passkey.
async function ask(path, body) {
  const answer = await fetch(`/sign-in/auth${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
  const said = await answer.json()
  if (!answer.ok) throw new Error(said.message ?? "That didn't work. Try again.")
  return said
}

/// The assistant that asked to be let in, or the device whose code this is once this person is shown it.
const [acts, setActs] = createSignal()
/// Whether this person is signed in here, for a device's code: they're asked for the passkey only if not.
const [signedIn, setSignedIn] = createSignal()
const [problem, setProblem] = createSignal()
const [done, setDone] = createSignal()
/// The name someone joining goes by, as they type it.
const [called, setCalled] = createSignal("")
if (request) ask("/oauth2/public-client-prelogin", { client_id: asked.get("client_id"), oauth_query: request }).then((client) => setActs(client.client_name))

/// Claims the device's code for whoever is signed in here, which only they can then answer, and learns what it is.
async function claim() {
  const { data, error } = await auth.device({ query: { user_code: code } })
  if (error?.status === 429) throw new Error("That's too many tries. Try again later.")
  if (error?.error === "expired_token") throw new Error("Expired. Start again on the device.")
  if (error) throw new Error("No device is showing that code.")
  // Someone else opened this link first, and it's theirs to answer.
  if (!data.client_id) throw new Error("Someone else is approving this device.")
  if (data.status !== "pending") throw new Error("Already approved.")
  setActs(data.name)
}
if (code && !app)
  auth.getSession().then(({ data }) => {
    setSignedIn(!!data)
    if (data) claim().catch((error) => setProblem(error.message))
  })

/// The passkey question, and what answering it lets in. Someone joining makes their passkey, and is signed in
/// with it. Whoever was signed in here before no longer is.
async function signIn() {
  if (another) {
    const { error } = await auth.passkey.addPasskey({ context: another, name: ["Passkey", Bowser.parse(navigator.userAgent).os.name].filter(Boolean).join(" on ") })
    // Better Auth's own words for a device that holds a passkey of theirs already, which is all the browser says.
    if (error?.code === "ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED") throw new Error("This device has a passkey of yours already. Open the link on another one.")
    if (error) throw new Error(error.message ?? "That passkey wasn't made. Try again.")
    return setDone("Your passkey is created. Go back to the app.")
  }
  if (invite && !called().trim()) throw new Error("Say your name first.")
  await auth.signOut()
  const { error } = invite ? await auth.passkey.addPasskey({ context: invite, createSession: true }) : await auth.signIn.passkey()
  if (error) throw new Error(error.message ?? "That passkey didn't work. Try again.")
  if (invite) await ask("/update-user", { called: called().trim() })
  if (request) {
    // Signing in was saying yes, so the assistant is sent back with what it came for.
    const { url } = await ask("/oauth2/consent", { accept: true, oauth_query: request })
    return location.replace(url)
  }
  if (!code) return setDone(invite ? "You've joined. Now open the app on your phone or computer, and sign in there." : "You're signed in. Now open the app on your phone or computer.")
  setSignedIn(true)
  await claim()
}

/// This person's answer to the device whose code they compared.
async function answer(yes) {
  await ask(yes ? "/device/approve" : "/device/deny", { userCode: code })
  setDone(yes ? `It's yours now: ${acts()}. Go back to it.` : `Denied: ${acts()} wasn't approved.`)
}

/// The link the device gives, as it's opened in the device's own browser.
const link = new URL(location.href)
link.searchParams.delete("app")
link.searchParams.delete("why")
/// Whether this person is answering a device: they're signed in, and its code is theirs to answer.
const answering = () => code && !app && signedIn() && acts()

render(
  () => (
    <SignIn
      mark={<img src="./mark.png" alt="" />}
      heading={answering() ? `Approve ${acts()}?` : another ? "Add a passkey" : invite ? "Join" : "Sign in"}
      note={app ? "Check this code where you sign in" : "It should be showing"}
      code={(app || answering()) && shown}
      warning={
        answering()
          ? "It will act as you. If the code differs, or you didn't ask for this, stop."
          : request && acts() && `${acts()} will act as you. Sign in only if you asked for this.`
      }
      problem={problem() ?? why}
      // Someone who's signed in has nothing to press until the code is theirs, or it couldn't be.
      button={app ? "Sign in" : answering() ? "It matches" : code && signedIn() !== false ? undefined : invite || another ? "Create a passkey" : "Sign in"}
      href={app ? link.href : undefined}
      onSignIn={answering() ? () => answer(true) : signIn}
      stop={answering() && "Stop"}
      onStop={() => answer(false)}
      done={done()}
    >
      {invite && !done() && (
        <Field label="Your name">
          <Input name="name" autocomplete="name" required value={called()} onInput={(event) => setCalled(event.target.value)} />
        </Field>
      )}
    </SignIn>
  ),
  document.body,
)
