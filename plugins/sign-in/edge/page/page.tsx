// The sign-in page. It asks for the passkey, and what that lets happen depends on how the page was opened (`Flow`):
// with an invitation, someone joins; from an assistant's request to be let in, the assistant is. Opened with a new
// device's code, as that device's link has it, its person signs in if they aren't yet, and is shown the code to
// compare with the device's before they let it in or stop it (`Stage`). Opened from the app's Passkeys, someone
// makes another passkey. Whichever it was, this browser is signed in after, and is one of that person's devices,
// whose Home Screen it goes on to when there's nothing else to answer. In the app's own window, which has no
// passkeys, the page shows the device's code and its button opens the page in the device's browser instead.
import { createSignal } from "solid-js"
import { render } from "@solidjs/web"
import { Field, Input, SignIn } from "ui"
import { createAuthClient } from "better-auth/client"
import { deviceAuthorizationClient } from "better-auth/client/plugins"
import { passkeyClient } from "@better-auth/passkey/client"
import Bowser from "bowser"

const auth = createAuthClient({ basePath: "/sign-in/auth", plugins: [passkeyClient(), deviceAuthorizationClient()] })
const opened = new URLSearchParams(location.hash.slice(1))
const params = new URLSearchParams(location.search)

/// How the page was opened, which picks what it does: someone joining with an invitation, someone who has joined
/// making another passkey (the app's Passkeys), an assistant's request to be let in, which Better Auth carries signed
/// in the query and goes back as it came, a new device's code in a browser, or in the app's own window, and otherwise
/// signing in.
type Flow =
  | { flow: "join"; invite: string }
  | { flow: "addPasskey"; another: string }
  | { flow: "assistant"; request: string; client: string }
  | { flow: "device"; code: string }
  | { flow: "deviceInApp"; code: string }
  | { flow: "signIn" }

const flow: Flow = params.has("client_id")
  ? { flow: "assistant", request: location.search.slice(1), client: params.get("client_id")! }
  : opened.has("invite")
    ? { flow: "join", invite: opened.get("invite")! }
    : opened.has("passkey")
      ? { flow: "addPasskey", another: opened.get("passkey")! }
      : params.has("user_code")
        ? { flow: params.has("app") ? "deviceInApp" : "device", code: params.get("user_code")! }
        : { flow: "signIn" }

/// Why the app shows a new code, in the words the device says it in (`NotApproved` in `core/client/src/lib.rs`).
const why = flow.flow === "deviceInApp" ? { denied: "It was denied where you signed in.", expired: "Its code expired before it was approved. Try again.", removed: "This device was removed from your network." }[params.get("why") ?? ""] : undefined
/// A code as it's shown, in two halves: WDJB-MJHT.
const shown = "code" in flow ? flow.code.replace(/(.{4})(?=.)/g, "$1-") : undefined

/// Where answering an assistant or a device is, one stage at a time, which only `setStage` moves. A device's: whether
/// this person is signed in here, which they do first if not; its code claimed for them, which only they can then
/// answer, as what it's for is learned; comparing codes, and saying it matches or stopping it, which the button
/// pressed shows under way; and how that ended. An assistant's: learning who's asking, and then saying so beside
/// the passkey question. Either may fail, and Try Again starts it again.
type Stage =
  | { stage: "checking" }
  | { stage: "needsPasskey" }
  | { stage: "claiming" }
  | { stage: "comparing"; name: string }
  | { stage: "approved"; name: string }
  | { stage: "denied"; name: string }
  | { stage: "ready"; name: string }
  | { stage: "created" }
  | { stage: "failed"; why: string }

const [stage, setStage] = createSignal<Stage>({ stage: "checking" })
/// The name someone joining goes by, as they type it.
const [called, setCalled] = createSignal("")

/// What this page asks Better Auth besides the passkey.
async function ask<T = unknown>(path: string, body: object): Promise<T> {
  const response = await fetch(`/sign-in/auth${path}`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) })
  const data = await response.json()
  if (!response.ok) throw new Error(data.message ?? "That didn't work. Try again.")
  return data
}

/// Starts answering what the page was opened for, or does so again after it failed: an assistant is named, and a
/// device's code is claimed once this person is signed in here.
async function begin() {
  setStage({ stage: "checking" })
  try {
    if (flow.flow === "assistant") {
      const client = await ask<{ client_name: string }>("/oauth2/public-client-prelogin", { client_id: flow.client, oauth_query: flow.request })
      return setStage({ stage: "ready", name: client.client_name })
    }
    if (flow.flow === "device") {
      const { data } = await auth.getSession()
      return data ? claim() : setStage({ stage: "needsPasskey" })
    }
  } catch (error) {
    setStage({ stage: "failed", why: (error as Error).message })
  }
}

/// Claims the device's code for whoever is signed in here, which only they can then answer, and learns what it is.
async function claim() {
  setStage({ stage: "claiming" })
  const { data, error } = await auth.device({ query: { user_code: (flow as { code: string }).code } })
  if (error?.status === 429) return setStage({ stage: "failed", why: "That's too many tries. Try again later." })
  // Better Auth's server answers an expired code `expired_token`, which its client's types leave out.
  if ((error?.error as string | undefined) === "expired_token") return setStage({ stage: "failed", why: "Expired. Start again on the device." })
  if (error) return setStage({ stage: "failed", why: "No device is showing that code." })
  // Someone else opened this link first, and it's theirs to answer.
  if (!data.client_id) return setStage({ stage: "failed", why: "Someone else is approving this device." })
  if (data.status !== "pending") return setStage({ stage: "failed", why: "Already approved." })
  // What the device calls itself, which this network's hook adds (`edge/auth.ts`).
  setStage({ stage: "comparing", name: (data as { name?: string }).name ?? "A device" })
}

/// The passkey question, and what answering it lets in. Someone joining makes their passkey, and is signed in
/// with it. Whoever was signed in here before no longer is.
async function signIn() {
  if (flow.flow === "addPasskey") {
    const { error } = await auth.passkey.addPasskey({ context: flow.another, name: ["Passkey", Bowser.parse(navigator.userAgent).os.name].filter(Boolean).join(" on ") })
    // Better Auth's own words for a device that holds a passkey of theirs already, which is all the browser says.
    if ((error as { code?: string } | null)?.code === "ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED") throw new Error("This device has a passkey of yours already. Open the link on another one.")
    if (error) throw new Error(error.message ?? "That passkey wasn't made. Try again.")
    return setStage({ stage: "created" })
  }
  if (flow.flow === "join" && !called().trim()) throw new Error("Say your name first.")
  await auth.signOut()
  const result = flow.flow === "join" ? await auth.passkey.addPasskey({ context: flow.invite, createSession: true }) : await auth.signIn.passkey({ returnWebAuthnResponse: true })
  const { error } = result
  if ((error as { code?: string } | null)?.code === "PASSKEY_NOT_FOUND") {
    // The browser offered a passkey this network doesn't have: one since removed, or one an earlier network at this
    // address made. WebAuthn's Signal API has the password manager remove it, so it isn't offered again.
    const { webauthn } = result as { webauthn?: { response: { id: string } } }
    if (webauthn && PublicKeyCredential.signalUnknownCredential) PublicKeyCredential.signalUnknownCredential({ rpId: location.hostname, credentialId: webauthn.response.id })
    throw new Error("This passkey is no longer valid for this network. Try again and choose another one, or ask to be invited again if you have no other.")
  }
  if (error) throw new Error(error.message ?? "That passkey didn't work. Try again.")
  if (flow.flow === "join") await ask("/update-user", { called: called().trim() })
  if (flow.flow === "assistant") {
    // Signing in was saying yes, so the assistant is sent back with what it came for.
    const { url } = await ask<{ url: string }>("/oauth2/consent", { accept: true, oauth_query: flow.request })
    return location.replace(url)
  }
  // Signed in, someone is shown their Home Screen: this browser has the app too, the web client.
  if (flow.flow !== "device") return location.replace("/")
  await claim()
}

/// This person's answer to the device whose code they compared.
async function respond(yes: boolean) {
  const { name } = stage() as { name: string }
  await ask(yes ? "/device/approve" : "/device/deny", { userCode: (flow as { code: string }).code })
  setStage({ stage: yes ? "approved" : "denied", name })
}

/// The link the device gives, as it's opened in the device's own browser.
const link = new URL(location.href)
link.searchParams.delete("app")
link.searchParams.delete("why")

/// What a stage shows, as `SignIn` takes it, and whether it asks for a name.
type Shows = Parameters<typeof SignIn>[0] & { asksName?: boolean }

/// What each stage shows: its heading, the spinner's line while something's under way, the code, the warning, the
/// problem, the button and what it does, and the one line left once it's answered.
function shows(): Shows {
  const now = stage()
  const failed = now.stage === "failed" ? now.why : undefined
  switch (flow.flow) {
    case "deviceInApp":
      return { heading: "Sign in", note: "Check this code where you sign in", code: shown, problem: why, button: "Sign in", href: link.href }
    case "device":
      switch (now.stage) {
        case "checking":
          return { heading: "Approve a device", status: "Checking you're signed in…" }
        case "needsPasskey":
          return { heading: "Sign in", button: "Sign in", onSignIn: signIn }
        case "claiming":
          return { heading: "Approve a device", status: "Finding the device…" }
        case "comparing":
          return {
            heading: `Approve ${now.name}?`,
            note: "It should be showing",
            code: shown,
            warning: "It will act as you. If the code differs, or you didn't ask for this, stop.",
            button: "It matches",
            onSignIn: () => respond(true),
            stop: "Stop",
            onStop: () => respond(false),
          }
        case "approved":
          return { heading: `Approve ${now.name}?`, done: `It's yours now: ${now.name}. Go back to it.` }
        case "denied":
          return { heading: `Approve ${now.name}?`, done: `Denied: ${now.name} wasn't approved.` }
        default:
          return { heading: "Approve a device", problem: failed, button: "Try Again", onSignIn: begin }
      }
    case "assistant":
      switch (now.stage) {
        case "ready":
          return { heading: "Sign in", warning: `${now.name} will act as you. Sign in only if you asked for this.`, button: "Sign in", onSignIn: signIn }
        case "failed":
          return { heading: "Sign in", problem: `Couldn't tell who's asking: ${failed}`, button: "Try Again", onSignIn: begin }
        default:
          return { heading: "Sign in", status: "Checking who's asking…" }
      }
    case "addPasskey":
      return now.stage === "created" ? { heading: "Add a passkey", done: "Your passkey is created. Go back to the app." } : { heading: "Add a passkey", button: "Create a passkey", onSignIn: signIn }
    case "join":
      return { heading: "Join", button: "Create a passkey", onSignIn: signIn, asksName: true }
    case "signIn":
      return { heading: "Sign in", button: "Sign in", onSignIn: signIn }
  }
}

begin()

render(
  () => (
    <SignIn
      mark={<img src="./mark.png" alt="" />}
      heading={shows().heading}
      note={shows().note}
      code={shows().code}
      warning={shows().warning}
      problem={shows().problem}
      status={shows().status}
      button={shows().button}
      href={shows().href}
      onSignIn={shows().onSignIn}
      stop={shows().stop}
      onStop={shows().onStop}
      done={shows().done}
    >
      {shows().asksName && !shows().done && (
        <Field label="Your name">
          <Input name="name" autocomplete="name" required value={called()} onInput={(event) => setCalled(event.target.value)} />
        </Field>
      )}
    </SignIn>
  ),
  document.body,
)
