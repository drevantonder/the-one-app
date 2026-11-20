// The sign-in page. It asks for the passkey, and what that lets happen depends on how the page was opened (`Flow`):
// with an invitation, someone joins; from an assistant's request to be let in, the assistant is, once they Allow it
// on a consent page as Google's and GitHub's are. Opened with a new
// device's code, as that device's link has it, its person signs in if they aren't yet, and is shown the code to
// compare with the device's before they let it in or stop it (`Stage`). Opened from the app's Sign In & Security, someone
// makes another passkey. Whichever it was, this browser is signed in after, and is one of that person's devices,
// whose Home Screen it goes on to when there's nothing else to answer. In the app's own window, the page first asks
// who's signing in, then shows the number that person types on one of their devices to let this one in, as it waits
// (`Request` in `edge/auth.ts`). Use a Passkey shows the device's code instead, its button goes on to the passkey there (one the app may use, its network's, as `/.well-known`
// says), and Use Another Device gives the link and the code to open where they're signed in, for a passkey the app
// can't reach. Every passkey is asked for its PRF output, which unlocks its person's key (`passkey.ts`): a device
// approved with one that can holds the key as it comes in, and a device that's in but doesn't hold it opens this page
// to Unlock with Passkey. Where the network signs people in with Google too, Continue with Google is the first way,
// and the passkey the second: Google sends them back here, to carry on with what the page was opened for.
import { createSignal, Show } from "solid-js"
import { render } from "@solidjs/web"
import mark from "../../../../icons/Logo.png"
import { Field, Icon, Input, SignIn } from "ui"
import { createAuthClient } from "better-auth/client"
import { deviceAuthorizationClient } from "better-auth/client/plugins"
import { passkeyClient } from "@better-auth/passkey/client"
import Bowser from "bowser"
import { code, forDevice, NO_PRF, opened as openedKey, output, PRF, prfKey } from "./passkey.ts"
import "./page.css"

const auth = createAuthClient({ basePath: "/sign-in/auth", plugins: [passkeyClient(), deviceAuthorizationClient()] })
const opened = new URLSearchParams(location.hash.slice(1))
const params = new URLSearchParams(location.search)
/// Whether the network signs people in with Google (`/social-providers` in `edge/auth.ts`).
const [google, setGoogle] = createSignal(false)
fetch("/sign-in/auth/social-providers")
  .then((response) => response.json())
  .then(({ providers }) => setGoogle(providers.includes("google")))
  .catch(() => {})
/// Why Google's sign-in didn't let someone in, as Better Auth sends them back here.
const refused = params.get("error_description") ?? params.get("error") ?? undefined

/// How the page was opened, which picks what it does: someone joining with an invitation, someone who has joined
/// making another passkey (the app's Sign In & Security), an assistant's request to be let in, which Better Auth carries signed
/// in the query and goes back as it came, a new device's code in a browser, or in the app's own window, a device of
/// theirs asking for their key (the Vault's Unlock with Passkey), or a passkey made ready to unlock it, and otherwise
/// signing in.
type Flow =
  | { flow: "join"; invite: string }
  | { flow: "addPasskey"; another: string }
  | { flow: "assistant"; request: string; client: string }
  | { flow: "device"; code: string }
  | { flow: "deviceInApp"; code: string }
  | { flow: "unlock"; device?: string }
  | { flow: "signIn" }

const flow: Flow = params.has("client_id")
  ? { flow: "assistant", request: location.search.slice(1), client: params.get("client_id")! }
  : opened.has("invite")
    ? { flow: "join", invite: opened.get("invite")! }
    : opened.has("passkey")
      ? { flow: "addPasskey", another: opened.get("passkey")! }
      : opened.has("unlock")
      ? { flow: "unlock", device: opened.get("unlock") || undefined }
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
/// pressed shows under way; and how that ended. An assistant's: learning who's asking, and then asking whether to
/// Allow it. Either may fail, and Try Again starts it again.
type Stage =
  | { stage: "checking" }
  | { stage: "needsSignIn" }
  | { stage: "claiming" }
  | { stage: "comparing"; name: string; device: string }
  | { stage: "approved"; name: string; unlocked: boolean }
  | { stage: "denied"; name: string }
  | { stage: "ready"; app: App }
  | { stage: "created" }
  | { stage: "unlocked"; name?: string }
  | { stage: "failed"; why: string }

const [stage, setStage] = createSignal<Stage>({ stage: "checking" })
/// How the app's own window lets its device in: by who's signing in, whose devices are asked, by a passkey with the
/// device's code, or with the link and the code to open elsewhere (Use Another Device).
const [how, setHow] = createSignal<"named" | "passkey" | "away">("named")
/// Who's signing in on a new device, as they type it, and once their devices are asked, the number it shows them.
const [person, setPerson] = createSignal("")
const [number, setNumber] = createSignal<number>()
/// Whether the link was copied, where there's no share sheet.
const [copied, setCopied] = createSignal(false)
/// The name someone joining goes by, as they type it.
const [called, setCalled] = createSignal("")

/// An assistant asking to be let in, as its own metadata names it (its Client ID Metadata Document or its
/// registration), with who'd let it in if they're signed in here already.
interface App {
  name: string
  logo?: string
  /// Where it says it's from, when that isn't where it sends you back.
  from?: string
  to?: string
  /// What this person goes by.
  person?: string
}

/// The site an app says it's from, if it sends you back to a different one on the web: its metadata doesn't add up.
/// One sent back to `localhost` or an app's own link is a program on this device.
function elsewhere(from: string | undefined, redirect: string | null) {
  try {
    const [said, back] = [new URL(from!).hostname, new URL(redirect!)]
    if (back.protocol !== "https:") return {}
    const to = back.hostname
    const same = said === to || to.endsWith(`.${said}`) || said.endsWith(`.${to}`)
    return same ? {} : { from: said, to }
  } catch {
    return {}
  }
}

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
      const [client, { data }] = await Promise.all([
        ask<{ client_name?: string; client_uri?: string; logo_uri?: string }>("/oauth2/public-client-prelogin", { client_id: flow.client, oauth_query: flow.request }),
        auth.getSession(),
      ])
      // An app named by its metadata document's address is from that address's site, which Better Auth checked.
      const site = client.client_uri ?? (flow.client.startsWith("https://") ? flow.client : undefined)
      const user = data?.user as { name: string; called?: string | null } | undefined
      return setStage({
        stage: "ready",
        app: {
          name: client.client_name || (site ? new URL(site).hostname : "An app"),
          logo: client.logo_uri?.startsWith("https://") ? client.logo_uri : undefined,
          ...elsewhere(site, params.get("redirect_uri")),
          person: user && (user.called || user.name),
        },
      })
    }
    if (flow.flow === "device") {
      const { data } = await auth.getSession()
      return data ? claim() : setStage({ stage: "needsSignIn" })
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
  setStage({ stage: "comparing", name: (data as { name?: string }).name ?? "A device", device: data.client_id })
}

/// The code of the key the passkey answered on this page derives, which a device that allows it shows too.
let shownCode: string | undefined

/// The passkey answered on this page, by its credential ID, with its PRF output if it gave one.
let used: { credential: string; prf?: Uint8Array; at: number } | undefined

/// What WebAuthn said as a passkey was made or used, as Better Auth hands it back.
type Answered = { webauthn?: { response: { id: string }; clientExtensionResults?: AuthenticationExtensionsClientOutputs } }

/// Keeps the passkey just answered, and says with it which key its PRF output derives, or that it gave none (`null`),
/// which a device that holds its person's key encrypts the key to (`edge/auth.ts`).
async function answered(result: Answered, prf: Uint8Array | undefined | null) {
  used = { credential: result.webauthn!.response.id, prf: prf ?? undefined, at: Date.now() }
  const key = prf && (await prfKey(prf))
  if (prf !== undefined) await ask("/passkey/prf-key", { credential: used.credential, key })
  shownCode = key ? await code(key) : undefined
}

/// Signs in with Google, which brings them back to this page, as it was opened, to carry on: someone joining to Home,
/// with their invitation and the name they typed, if they did. Back here, they're signed in, as of just now.
async function withGoogle() {
  const back = new URL(location.href)
  back.searchParams.delete("error")
  back.searchParams.delete("error_description")
  const { error } = await auth.signIn.social({
    provider: "google",
    callbackURL: flow.flow === "join" ? "/" : back.href,
    errorCallbackURL: back.href,
    ...(flow.flow === "join" && { additionalData: { invite: flow.invite, called: called().trim() } }),
  })
  if (error) throw new Error(error.message ?? "Google didn't answer. Try again.")
  // Google's page is opening.
  await new Promise(() => {})
}

/// Whether this browser's session was made in the last few minutes, as the edge asks of whoever approves something.
async function signedInJustNow() {
  const { data } = await auth.getSession()
  return !!data && Date.now() - new Date(data.session.createdAt).getTime() < 4 * 60_000
}

/// Signs in with a passkey, asked for its PRF output, which whoever was signed in here before no longer is.
async function withPasskey() {
  await auth.signOut()
  const result = await auth.signIn.passkey({ returnWebAuthnResponse: true, extensions: PRF })
  const { error } = result
  if ((error as { code?: string } | null)?.code === "PASSKEY_NOT_FOUND") {
    // The browser offered a passkey this network doesn't have: one since removed, or one an earlier network at this
    // address made. WebAuthn's Signal API has the password manager remove it, so it isn't offered again.
    const { webauthn } = result as Answered
    if (webauthn && PublicKeyCredential.signalUnknownCredential) PublicKeyCredential.signalUnknownCredential({ rpId: location.hostname, credentialId: webauthn.response.id })
    throw new Error("This passkey is no longer valid for this network. Try again and choose another one, or ask to be invited again if you have no other.")
  }
  if (error) throw new Error(error.message ?? "That passkey didn't work. Try again.")
  await answered(result as Answered, output((result as Answered).webauthn?.clientExtensionResults) ?? null)
}

/// A passkey just made: its PRF output if making it gave one. One that has PRF but gave none as it was made (most
/// providers but iCloud Keychain and Google Password Manager) gives it once it's used, so someone adding a passkey
/// to unlock with uses it once more here. Someone joining has no key yet, and gives it the next time they sign in.
async function made(result: Answered, again: boolean) {
  const results = result.webauthn?.clientExtensionResults
  const prf = output(results)
  if (prf || !results?.prf?.enabled) return answered(result, prf ?? null)
  if (again) return withPasskey()
  used = { credential: result.webauthn!.response.id, at: Date.now() }
}

/// Calls a tool of the network's as whoever is signed in here, as the web client does (`clients/web/host.js`).
async function tool<T>(name: string, inputs: object): Promise<T> {
  const response = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: inputs } }) })
  const { result, error } = await response.json()
  const said = result?.content?.[0]?.text
  if (error || result?.isError) throw new Error(error?.message ?? said ?? "That didn't work. Try again.")
  return JSON.parse(said)
}

/**
 * Lets the device whose key is `device` hold its person's key, with the passkey just answered here, if its PRF
 * output opens the key a device that holds it encrypted to it: it's encrypted to the device, as a device that holds
 * it approves one. Whether it did.
 */
async function unlock(device: string, name: string) {
  if (!used?.prf) return false
  const { owner, key } = await ask<{ owner: string; key: string | null }>("/passkey/encrypted-key", { credential: used.credential })
  if (!key) return false
  const opened = await openedKey(used.prf, owner, used.credential, key)
  try {
    await tool("encryption_approve", { device, name, key: await forDevice(opened, owner, device) })
  } finally {
    opened.fill(0)
  }
  return true
}

/// The device a link to Unlock with Passkey names, once a passkey's opened it: it's one of this person's that's
/// asking for their key, as the Vault asks before it opens the link.
async function unlockAsked(device: string) {
  if (!used?.prf) throw new Error(NO_PRF)
  const { asking } = await tool<{ asking: { device: string; name: string }[] }>("encryption_key", {})
  const name = asking.find((asked) => asked.device === device)?.name
  if (!name) throw new Error("That device isn't asking for your key. Choose Unlock with Passkey on it again.")
  if (!(await unlock(device, name))) throw new Error("This passkey can't unlock your key yet. Allow it in Sign In & Security on a device that holds your key, then try again.")
  setStage({ stage: "unlocked", name })
}

/// What a passkey made here is called, by where it was made, as its person's Sign In & Security lists it, and asks
/// whether it may unlock their key.
const passkeyName = ["Passkey", Bowser.parse(navigator.userAgent).os.name].filter(Boolean).join(" on ")

/// The passkey question, and what answering it lets in. Someone joining makes their passkey, and is signed in
/// with it. Whoever was signed in here before no longer is.
async function signIn() {
  if (flow.flow === "addPasskey") {
    await auth.signOut()
    const result = await auth.passkey.addPasskey({ context: flow.another, name: passkeyName, createSession: true, extensions: PRF, returnWebAuthnResponse: true })
    const { error } = result
    // Better Auth's own words for a device that holds a passkey of theirs already, which is all the browser says.
    if ((error as { code?: string } | null)?.code === "ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED") throw new Error("This device has a passkey of yours already. Open the link on another one.")
    if (error) throw new Error(error.message ?? "That passkey wasn't made. Try again.")
    await made(result as Answered, true)
    return setStage({ stage: "created" })
  }
  if (flow.flow === "join") {
    if (!called().trim()) throw new Error("Say your name first.")
    await auth.signOut()
    const result = await auth.passkey.addPasskey({ context: flow.invite, name: passkeyName, createSession: true, extensions: PRF, returnWebAuthnResponse: true })
    if (result.error) throw new Error(result.error.message ?? "That passkey didn't work. Try again.")
    await made(result as Answered, false)
    await ask("/update-user", { called: called().trim() })
  } else if (flow.flow === "assistant" && (await signedInJustNow())) {
    // Signed in just now, with Google or a passkey: Allow needs nothing more.
  } else if (flow.flow === "assistant" && google() && !((await auth.getSession()).data?.session as { passkey?: string | null } | undefined)?.passkey) {
    // Someone who signs in with Google does so again, and comes back here to Allow.
    return withGoogle()
  } else await withPasskey()
  if (flow.flow === "unlock") return flow.device ? unlockAsked(flow.device) : setStage(used?.prf ? { stage: "unlocked" } : { stage: "failed", why: NO_PRF })
  if (flow.flow === "assistant") {
    // They allowed it and signed in, so the assistant is sent back with what it came for.
    const { url } = await ask<{ url: string }>("/oauth2/consent", { accept: true, oauth_query: flow.request })
    return location.replace(url)
  }
  // Signed in, someone is shown their Home Screen: this browser has the app too, the web client.
  if (flow.flow !== "device") return location.replace("/")
  await claim()
}

/// Cancel on the consent page: the assistant is sent back told no (`access_denied`), whether or not anyone's signed in.
async function cancel() {
  const { url } = await ask<{ url: string }>("/oauth2/consent", { accept: false, oauth_query: (flow as { request: string }).request })
  location.replace(url)
}

/// This person's answer to the device whose code they compared. Saying it matches asks for their passkey, if it
/// wasn't just answered here, so the device holds their key as it comes in when the passkey unlocks it.
async function respond(yes: boolean) {
  const { name, device } = stage() as { name: string; device: string }
  if (!yes) {
    await ask("/device/deny", { userCode: (flow as { code: string }).code })
    return setStage({ stage: "denied", name })
  }
  // The edge approves only for someone who signed in in the last five minutes (`edge/auth.ts`).
  if (!(used && Date.now() - used.at < 4 * 60_000) && !(await signedInJustNow())) await (google() && !used ? withGoogle() : withPasskey())
  const unlocked = await unlock(device, name)
  await ask("/device/approve", { userCode: (flow as { code: string }).code })
  setStage({ stage: "approved", name, unlocked })
}

/// The link the device gives, as it's opened anywhere else, and as it's shown, which is short enough to type.
const link = new URL(location.href)
link.searchParams.delete("app")
link.searchParams.delete("why")
const typed = `${link.host}${link.pathname}${link.search}`
/// Whether this device has a share sheet to hand the link to.
const shares = "share" in navigator

/// Hands the link to the device's share sheet, or copies it where there's none. Closing the sheet isn't a problem.
async function share() {
  if (!shares) return navigator.clipboard.writeText(link.href).then(() => setCopied(true))
  await navigator.share({ url: link.href }).catch((error: Error) => {
    if (error.name !== "AbortError") throw error
  })
}

/// Names who's signing in on this device, which asks their devices, and shows the number they tap on one of them. The
/// device learns it's in by itself, and the app moves on from this page then.
async function name() {
  if (!person().trim()) throw new Error("Say who's signing in first.")
  const { number } = await ask<{ number: number }>("/device/claim", { userCode: (flow as { code: string }).code, person: person() })
  setNumber(number)
}

/// What a stage shows, as `SignIn` takes it, whether it asks for a name, and the assistant it asks about.
type Shows = Parameters<typeof SignIn>[0] & { asksName?: boolean; asksWho?: boolean; app?: App; elsewhere?: boolean }

/// What each stage shows: its heading, the spinner's line while something's under way, the code, the warning, the
/// problem, the button and what it does, and the one line left once it's answered.
function shows(): Shows {
  const now = stage()
  const failed = now.stage === "failed" ? now.why : undefined
  switch (flow.flow) {
    case "deviceInApp":
      if (how() === "away") return { heading: "Sign in on another device", button: shares ? "Share Link" : copied() ? "Copied" : "Copy Link", onSignIn: share, stop: "Back", onStop: () => setHow("passkey"), elsewhere: true }
      if (how() === "passkey") return { heading: "Sign in", note: "Check this code where you sign in", code: shown, problem: why, button: "Sign in", onSignIn: () => location.assign(link.href), stop: "Use Another Device", onStop: () => setHow("away") }
      if (number() !== undefined)
        return { heading: "Check your devices", note: "On another of your devices, open Devices and enter", code: String(number()), status: "Waiting for approval…", quiet: true, button: "Use a Passkey Instead", onSignIn: () => setHow("passkey"), stop: "Back", onStop: () => setNumber() }
      return { heading: "Sign in", problem: why, button: "Continue", onSignIn: name, stop: "Use a Passkey", onStop: () => setHow("passkey"), asksWho: true }
    case "device":
      switch (now.stage) {
        case "checking":
          return { heading: "Approve a device", status: "Checking you're signed in…" }
        case "needsSignIn":
          return google() ? { heading: "Sign in", problem: refused, ...withGoogleFirst("Use a Passkey") } : { heading: "Sign in", button: "Sign in", onSignIn: signIn }
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
          return { heading: `Approve ${now.name}?`, done: now.unlocked ? `It's yours now, and holds your key: ${now.name}. Go back to it.` : `It's yours now: ${now.name}. Go back to it.` }
        case "denied":
          return { heading: `Approve ${now.name}?`, done: `Denied: ${now.name} wasn't approved.` }
        default:
          return { heading: "Approve a device", problem: failed, button: "Try Again", onSignIn: begin }
      }
    case "assistant":
      switch (now.stage) {
        case "ready": {
          const { app } = now
          return {
            mark: <Logos app={app} />,
            heading: `${app.name} wants to use your network`,
            app,
            button: "Allow",
            onSignIn: signIn,
            stop: "Cancel",
            onStop: cancel,
            beside: true,
          }
        }
        case "failed":
          return { heading: "Sign in", problem: `Couldn't tell who's asking: ${failed}`, button: "Try Again", onSignIn: begin }
        default:
          return { heading: "Sign in", status: "Checking who's asking…" }
      }
    case "unlock":
      switch (now.stage) {
        case "unlocked":
          return { heading: "Unlock with Passkey", done: now.name ? `Unlocked: ${now.name} holds your key now. Go back to it.` : `Your passkey is set up. Allow it in Sign In & Security on a device that holds your key, if it shows this code: ${shownCode}` }
        case "failed":
          return { heading: "Unlock with Passkey", problem: failed, button: "Try Again", onSignIn: signIn }
        default:
          return { heading: "Unlock with Passkey", note: flow.device ? "Use a passkey of yours to let the device hold your key." : "Use the passkey that should unlock your key on a new device.", button: "Continue", onSignIn: signIn }
      }
    case "addPasskey":
      return now.stage === "created" ? { heading: "Add a passkey", done: shownCode ? `Your passkey is created. Go back to the app, and allow it there if it shows this code: ${shownCode}` : "Your passkey is created. Go back to the app." } : { heading: "Add a passkey", button: "Create a passkey", onSignIn: signIn }
    case "join":
      return google() ? { heading: "Join", problem: refused, ...withGoogleFirst("Create a Passkey"), asksName: true } : { heading: "Join", button: "Create a passkey", onSignIn: signIn, asksName: true }
    case "signIn":
      return google() ? { heading: "Sign in", problem: refused, ...withGoogleFirst("Use a Passkey") } : { heading: "Sign in", button: "Sign in", onSignIn: signIn }
  }
}

/// Continue with Google as the button, and the passkey as the second way, `passkey` saying what that does.
const withGoogleFirst = (passkey: string) => ({ button: "Continue with Google", onSignIn: withGoogle, stop: passkey, onStop: signIn })

/// The app's logo, or its initial on a plain tile when it has none or it doesn't load, joined to the network's mark.
function Logos(props: { app: App }) {
  const [failed, setFailed] = createSignal(false)
  return (
    <div class="logos">
      <Show when={props.app.logo && !failed()} fallback={<span class="tile">{[...props.app.name][0].toUpperCase()}</span>}>
        <img class="tile" src={props.app.logo} alt="" referrerpolicy="no-referrer" onError={() => setFailed(true)} />
      </Show>
      <span class="joins" aria-hidden="true">···</span>
      <img src={mark} alt="" />
    </div>
  )
}

/// Who the app would act as, what it will be able to do, as its token allows, a warning only when its metadata
/// doesn't add up, and where it's taken out again.
/// It reaches `/mcp`, every app's tools, as this person, and with `offline_access` it's given a refresh token.
function Consent(props: { app: App }) {
  const stays = params.get("scope")?.split(" ").includes("offline_access")
  return (
    <>
      <p class="account">{[props.app.person && `as ${props.app.person}`, location.host].filter(Boolean).join(" · ")}</p>
      <section class="can">
        <h2>{props.app.name} will be able to</h2>
        <ul>
          <li><Icon name="check" />Use your apps' tools as you, like email, calendar and files</li>
          <li><Icon name="check" />Reach only what you can, nothing more</li>
          {stays && <li><Icon name="check" />Stay connected until you remove it</li>}
        </ul>
      </section>
      <Show when={props.app.to}>
        <p class="warning">
          <Icon name="warning-circle" />
          <span>{props.app.name} says it's from {props.app.from} but sends you back to {props.app.to}. Allow only if you trust {props.app.to}.</span>
        </p>
      </Show>
      <p class="note">Allow only apps you trust. You can remove {props.app.name} anytime in Devices.</p>
    </>
  )
}

begin()

render(
  () => (
    <SignIn
      mark={shows().mark ?? mark}
      heading={shows().heading}
      note={shows().note}
      code={shows().code}
      warning={shows().warning}
      problem={shows().problem}
      status={shows().status}
      button={shows().button}
      quiet={shows().quiet}
      onSignIn={shows().onSignIn}
      stop={shows().stop}
      onStop={shows().onStop}
      beside={shows().beside}
      done={shows().done}
    >
      {shows().app && <Consent app={shows().app!} />}
      {shows().elsewhere && (
        <div class="elsewhere">
          <p>Open this where you're signed in</p>
          <p class="link">{typed}</p>
          <div class="code">
            <p>It will show</p>
            <p class="shown">{shown}</p>
          </div>
        </div>
      )}
      {shows().asksWho && (
        <Field>
          <Input name="person" aria-label="Who's signing in?" autoFocus autoComplete="username" autoCapitalize="none" placeholder="Name or email" required value={person()} onChangeText={setPerson} />
        </Field>
      )}
      {shows().asksName && !shows().done && (
        <Field>
          <Input name="name" aria-label="Your name" autoFocus autoComplete="name" placeholder="Your name" required value={called()} onChangeText={setCalled} />
        </Field>
      )}
    </SignIn>
  ),
  document.body,
)
