// The sign-in page. It asks for the passkey, and what that lets happen depends on how the page was opened (`Flow`):
// with an invitation, someone joins; from an assistant's request to be let in, the assistant is, once they Allow it
// on a consent page as Google's and GitHub's are. Opened with a new
// device's code, as that device's link has it, its person signs in if they aren't yet, and is shown the code to
// compare with the device's before they let it in or stop it (`Stage`). Opened from the app's Sign In & Security, someone
// makes another passkey. Whichever it was, this browser is signed in after, and is one of that person's devices,
// whose Home Screen it goes on to when there's nothing else to answer. In the app's own window, the page shows the
// device's code, its button goes on to the passkey there (one the app may use, its network's, as `/.well-known`
// says), and Use Another Device gives the link and the code to open where they're signed in, for a passkey the app
// can't reach. Every passkey is asked for its PRF output, which unlocks its person's key (`passkey.ts`): a device
// approved with one that can holds the key as it comes in, and a device that's in but doesn't hold it opens this page
// to Unlock with Passkey. Where the network signs people in with Google too, Continue with Google is the first way,
// and the passkey the second: Google sends them back here, to carry on with what the page was opened for.
import { createSignal, Show } from "solid-js"
import mark from "../../../../icons/Logo.png"
import { Field, Icon, Image, Input, render, SignIn, styles, Text, tokens, View } from "ui"
import { createAuthClient } from "better-auth/client"
import { deviceAuthorizationClient } from "better-auth/client/plugins"
import { passkeyClient } from "@better-auth/passkey/client"
import { check, code, forDevice, forPasskey, forServers, NO_PRF, opened as openedKey, output, PRF, prfKey, unwrap } from "./passkey.ts"

const look = styles({
  /// The consent page an assistant is let in on, as Google's and GitHub's are: its logo joined to ours.
  logos: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3] },
  logo: { width: tokens.grid[12], height: tokens.grid[12] },
  /// An app's initial, where it has no logo of its own.
  tile: { width: tokens.grid[12], height: tokens.grid[12], alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.large, backgroundColor: tokens.colour.quinaryLabel },
  initial: { font: tokens.type.title },
  joins: { color: tokens.colour.secondaryLabel, font: tokens.type.headline },
  grey: { color: tokens.colour.secondaryLabel },
  /// Who it would act as, and where.
  account: { alignSelf: "stretch", color: tokens.colour.secondaryLabel, textAlign: "center" },
  /// What it will be able to do, in a plain box.
  can: { alignSelf: "stretch", gap: tokens.grid[2.5], padding: tokens.grid[4], borderWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, borderRadius: tokens.radius.medium },
  heading: { font: tokens.type.headline },
  row: { flexDirection: "row", gap: tokens.grid[2.5], alignItems: "flex-start" },
  tick: { color: tokens.colour.secondaryLabel, paddingTop: tokens.grid[0.5] },
  grows: { flexShrink: 1 },
  warning: { flexDirection: "row", gap: tokens.grid[2.5], alignItems: "flex-start", color: tokens.colour.orange },
  note: { color: tokens.colour.secondaryLabel, font: tokens.type.small, textAlign: "center" },
  /// Use Another Device, in the app's own window: the link to open where they're signed in, short enough to type, and
  /// the code it will show there, as big as the one a device shows.
  elsewhere: { alignSelf: "stretch", gap: tokens.grid[2], alignItems: "center" },
  link: { alignSelf: "stretch", paddingBlock: tokens.grid[3], paddingInline: tokens.grid[4], borderWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, borderRadius: tokens.radius.medium, fontFamily: tokens.font.mono, textAlign: "center" },
  shown: { font: tokens.type.largeTitle, fontFamily: tokens.font.mono },
  /// Done, as Google's and GitHub's device pages show it: a check mark where the network's mark was.
  done: { width: tokens.grid[20], height: tokens.grid[20], alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.full, backgroundColor: tokens.colour.green, color: tokens.colour.onAccent },
})

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
  | { flow: "unlock"; device?: string; scope?: string }
  | { flow: "signIn" }

// What it's opened for is read once, as it loads: a browser that opens another link to it in the same tab, which
// differs only after `#`, loads it again, as Safari does with a link from the app.
addEventListener("hashchange", () => location.reload())

const flow: Flow = params.has("client_id")
  ? { flow: "assistant", request: location.search.slice(1), client: params.get("client_id")! }
  : opened.has("invite")
    ? { flow: "join", invite: opened.get("invite")! }
    : opened.has("passkey")
      ? { flow: "addPasskey", another: opened.get("passkey")! }
      : opened.has("unlock")
      ? { flow: "unlock", device: opened.get("unlock") || undefined, scope: opened.get("scope") || undefined }
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
  | { stage: "confirm" }
  | { stage: "created" }
  | { stage: "unlocked"; name?: string; servers: string[]; whose: string }
  | { stage: "failed"; why: string; ended?: boolean }

const [stage, setStage] = createSignal<Stage>({ stage: "checking" })
/// How the app's own window lets its device in: by a passkey with the device's code, or with the link and the code to
/// open elsewhere (Use Another Device).
const [how, setHow] = createSignal<"passkey" | "away">("passkey")
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

/// What a device's code that isn't waiting any more says, as the edge has it (`/device/state` in `edge/auth.ts`).
const ended = {
  expired: "This code expired. Start again on the device.",
  approved: "Already approved.",
  denied: "This device was stopped. Start again on it.",
  unknown: "No device is showing that code. Start again on the device.",
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
          // From here, where the network passes it on (`logo` in `edge/worker.ts`): a page draws only pictures of its own.
          logo: client.logo_uri?.startsWith("https://") ? `/sign-in/logo?${new URLSearchParams({ client_id: flow.client, oauth_query: flow.request })}` : undefined,
          ...elsewhere(site, params.get("redirect_uri")),
          person: user && (user.called || user.name),
        },
      })
    }
    if (flow.flow === "device") {
      const { data } = await auth.getSession()
      if (data) return claim()
      // A code that ended says so, rather than asking them to sign in for nothing.
      const { state } = await (await fetch(`/sign-in/auth/device/state?user_code=${encodeURIComponent(flow.code)}`)).json()
      return setStage(state === "waiting" ? { stage: "needsSignIn" } : { stage: "failed", why: ended[state as keyof typeof ended] ?? ended.unknown, ended: true })
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
  if ((error?.error as string | undefined) === "expired_token") return setStage({ stage: "failed", why: ended.expired, ended: true })
  if (error) return setStage({ stage: "failed", why: ended.unknown, ended: true })
  // Someone else opened this link first, and it's theirs to answer.
  if (!data.client_id) return setStage({ stage: "failed", why: "Someone else is approving this device.", ended: true })
  if (data.status !== "pending") return setStage({ stage: "failed", why: ended.approved, ended: true })
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
  if (error) throw await plainly("use", error)
  await answered(result as Answered, output((result as Answered).webauthn?.clientExtensionResults) ?? null)
}

/// A passkey just made: its PRF output if making it gave one. One that has PRF but gave none as it was made (most
/// providers but iCloud Keychain and Google Password Manager) gives it once it's used, so someone adding a passkey to
/// unlock with uses it once more here, with a tap of its own: a browser asks for a passkey only right after one, and
/// Safari refuses it otherwise. Someone joining has no key yet, and gives it the next time they sign in.
async function made(result: Answered, again: boolean) {
  const results = result.webauthn?.clientExtensionResults
  const prf = output(results)
  if (prf || !results?.prf?.enabled) return answered(result, prf ?? null)
  used = { credential: result.webauthn!.response.id, at: Date.now() }
  if (again) setStage({ stage: "confirm" })
}

/// What a browser said when a passkey wasn't made or used, in the page's words, with what to do. Its own words go to
/// Telemetry, as they help whoever looks into it and nobody here: Safari's NotAllowedError says only that "the user
/// agent or the platform" didn't allow it. What this network's sign-in says (`edge/auth.ts`) is said as it is.
async function plainly(as: "make" | "use", error: { code?: string; message?: string }) {
  const browser = !error.code || /^(ERROR_|AUTH_CANCELLED|UNKNOWN_ERROR)/.test(error.code)
  if (!browser) return new Error(error.message ?? "That didn't work. Try again.")
  const record = { name: "sign_in.page.passkey", attributes: { as, code: error.code ?? "" }, error: error.message ?? "" }
  console.error("passkey", record)
  await tool("telemetry_record", { records: [record] }).catch(() => {})
  return new Error(as === "make" ? "Not saved. Create a passkey to try again." : "No passkey was used. Try again.")
}

/// Calls a tool of the network's as whoever is signed in here, as the web client does (`clients/web/host.js`), for
/// them or for a group or the network they own (`subject`).
async function tool<T>(name: string, inputs: object, subject?: string): Promise<T> {
  const params = { name, arguments: inputs, ...(subject && { _meta: { subject } }) }
  const response = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params }) })
  const { result, error } = await response.json()
  const said = result?.content?.[0]?.text
  if (error || result?.isError) throw new Error(error?.message ?? said ?? "That didn't work. Try again.")
  return JSON.parse(said)
}

/// What the encryption app says of someone's key (`encryption_key`): what tells it from another, once there's one, the
/// devices asking for it, a group's or the network's sealed to the person's own, and which servers should hold the
/// keys of what's encrypted for them and don't.
interface Held {
  check: string | null
  asking: { device: string; name: string }[]
  wrapped: string | null
  access: { artifact: string; epoch: number; servers: Server[]; missing: Server[]; extra: Server[] }[]
}
type Server = { key: string; name: string | null }

/// Someone's whose a key is, as a person reads it.
const whose = (scope: string) => (scope === "network" ? "the network's" : `${scope.split("/").pop()}'s`)

/**
 * The person's own key, which the passkey just answered here opens, with whose it is. Where they have none yet, it's
 * made here, sealed to this passkey, which is how anyone's key is first made. Where they have one this passkey can't
 * open, it says how to allow it.
 */
async function ownKey(): Promise<{ owner: string; key: Uint8Array<ArrayBuffer>; made: boolean }> {
  if (!used?.prf) throw new Error(NO_PRF)
  const { owner, key } = await ask<{ owner: string; key: string | null }>("/passkey/encrypted-key", { credential: used.credential })
  if (key) return { owner, key: await openedKey(used.prf, owner, used.credential, key), made: false }
  if ((await tool<Held>("encryption_key", {})).check) throw new Error("This passkey can't unlock your key yet. Allow it in Sign In & Security on a device that holds your key, then try again.")
  const made = crypto.getRandomValues(new Uint8Array(32))
  const { passkeys } = await tool<{ passkeys: { id: string; credential: string; prf_key: string | null }[] }>("sign-in_devices", {})
  const passkey = passkeys.find((passkey) => passkey.credential === used!.credential)
  if (!passkey?.prf_key) throw new Error(NO_PRF)
  await tool("encryption_store", { check: await check(made) }, owner)
  const keys = JSON.stringify({ [passkey.credential]: { prf_key: passkey.prf_key, key: await forPasskey(made, owner, passkey.credential, passkey.prf_key) } })
  await tool("sign-in_encrypt_for_passkeys", { passkey: passkey.id, keys })
  return { owner, key: made, made: true }
}

/// Lets the device a link to Unlock with Passkey names hold its person's key: it's one of theirs that's asking for
/// it, as the Vault asks before it opens the link. What it's called.
async function toDevice(key: Uint8Array<ArrayBuffer>, owner: string, device: string) {
  const name = (await tool<Held>("encryption_key", {})).asking.find((asked) => asked.device === device)?.name
  if (!name) throw new Error("That device isn't asking for your key. Choose Unlock with Passkey on it again.")
  await tool("encryption_store", { device: { key: device, name, ...(await forDevice(key, owner, device)) } }, owner)
  return name
}

/**
 * Hands the keys of what's encrypted for `scope` to the servers that act for it and are waiting for them, and takes
 * them from any that hold them and no longer do: its person's own, or a group's or the network's they own, which is
 * sealed to their key. The names of the servers that were waiting.
 */
async function toServers(key: Uint8Array<ArrayBuffer>, owner: string, scope: string) {
  const held = await tool<Held>("encryption_key", {}, scope)
  let opened = key
  if (scope !== owner) {
    if (!held.wrapped) throw new Error(`Your key doesn't open ${whose(scope)} key yet.`)
    opened = await unwrap(key, owner, scope, held.wrapped)
    if ((await check(opened)) !== held.check) throw new Error(`What's kept as ${whose(scope)} key isn't it, so nothing was handed over.`)
  }
  try {
    const waiting = held.access.filter((artifact) => artifact.missing.length || artifact.extra.length)
    const artifacts = []
    for (const { artifact, epoch, servers, missing, extra } of waiting) {
      const should = [...servers, ...missing].map((server) => server.key).filter((server) => !extra.some((other) => other.key === server))
      artifacts.push(await forServers(opened, artifact, epoch, should))
    }
    if (artifacts.length) await tool("encryption_store", { artifacts }, scope)
    return [...new Set(waiting.flatMap((artifact) => artifact.missing.map((server) => server.name ?? server.key.slice(0, 8))))]
  } finally {
    if (opened !== key) opened.fill(0)
  }
}

/**
 * Unlock with Passkey: the passkey just answered opens its person's key, or makes it, and hands it to the device the
 * link names, if it does, and to the servers waiting for it, of the person or of the group or network the link
 * names (`scope`), which their key opens.
 */
async function unlock(device: string | undefined, scope: string | undefined) {
  const { owner, key } = await ownKey()
  try {
    const name = device && (await toDevice(key, owner, device))
    const servers = await toServers(key, owner, scope ?? owner)
    setStage({ stage: "unlocked", name: name || undefined, servers, whose: scope && scope !== owner ? whose(scope) : "your" })
  } finally {
    key.fill(0)
  }
}

/// A passkey that can unlock: made from here, where the one just used can't, as Sign In & Security's Add Passkey does.
async function another() {
  const { link } = await ask<{ link: string }>("/passkey/another", { credential: used?.credential })
  location.replace(link)
  location.reload()
}

/// The passkey question, and what answering it lets in. Someone joining makes their passkey, and is signed in
/// with it. Whoever was signed in here before no longer is.
async function signIn() {
  if (flow.flow === "addPasskey") {
    await auth.signOut()
    const result = await auth.passkey.addPasskey({ context: flow.another, createSession: true, extensions: PRF, returnWebAuthnResponse: true })
    const { error } = result
    // Better Auth's own words for a device that holds a passkey of theirs already, which is all the browser says.
    if ((error as { code?: string } | null)?.code === "ERROR_AUTHENTICATOR_PREVIOUSLY_REGISTERED") throw new Error("This device has a passkey of yours already. Open the link on another one.")
    if (error) throw await plainly("make", error)
    await made(result as Answered, true)
    if (stage().stage !== "confirm") setStage({ stage: "created" })
    return
  }
  if (flow.flow === "join") {
    if (!called().trim()) throw new Error("Say your name first.")
    await auth.signOut()
    const result = await auth.passkey.addPasskey({ context: flow.invite, createSession: true, extensions: PRF, returnWebAuthnResponse: true })
    if (result.error) throw await plainly("make", result.error)
    await made(result as Answered, false)
    await ask("/update-user", { called: called().trim() })
  } else if (flow.flow === "assistant" && (await signedInJustNow())) {
    // Signed in just now, with Google or a passkey: Allow needs nothing more.
  } else if (flow.flow === "assistant" && google() && !((await auth.getSession()).data?.session as { passkey?: string | null } | undefined)?.passkey) {
    // Someone who signs in with Google does so again, and comes back here to Allow.
    return withGoogle()
  } else await withPasskey()
  // What stops it is a stage of its own, which for a passkey that can't unlock offers to make one that can.
  if (flow.flow === "unlock") return unlock(flow.device, flow.scope).catch((error: Error) => setStage({ stage: "failed", why: error.message }))
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
  await ask("/device/approve", { userCode: (flow as { code: string }).code })
  setStage({ stage: "approved", name, unlocked: await handedTo(device, name) })
}

/// Lets a device just approved hold its person's key too, where the passkey just answered unlocks it. Whether it does.
async function handedTo(device: string, name: string) {
  if (!used?.prf) return false
  const { owner, key } = await ask<{ owner: string; key: string | null }>("/passkey/encrypted-key", { credential: used.credential })
  if (!key) return false
  const opened = await openedKey(used.prf, owner, used.credential, key)
  try {
    await tool("encryption_store", { device: { key: device, name, ...(await forDevice(opened, owner, device)) } }, owner)
  } finally {
    opened.fill(0)
  }
  return true
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

/// What a stage shows, as `SignIn` takes it, whether it asks for a name, and the assistant it asks about.
type Shows = Parameters<typeof SignIn>[0] & { asksName?: boolean; app?: App; elsewhere?: boolean }

/// What each stage shows: its heading, the spinner's line while something's under way, the code, the warning, the
/// problem, the button and what it does, and the one line left once it's answered.
function shows(): Shows {
  const now = stage()
  const failed = now.stage === "failed" ? now.why : undefined
  switch (flow.flow) {
    case "deviceInApp":
      if (how() === "away") return { heading: "Sign in on another device", button: shares ? "Share Link" : copied() ? "Copied" : "Copy Link", onSignIn: share, stop: "Back", onStop: () => setHow("passkey"), elsewhere: true }
      return { heading: "Sign in", note: "Check this code where you sign in", code: shown, problem: why, button: "Sign in", onSignIn: () => location.assign(link.href), stop: "Use Another Device", onStop: () => setHow("away") }
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
        // Done, as Google's and GitHub's device pages are: nothing left to answer here.
        case "approved":
          return { mark: <DoneMark />, heading: `${now.name} is signed in`, done: now.unlocked ? "It holds your key too. You can close this page and go back to it." : "You can close this page and go back to it." }
        case "denied":
          return { heading: `${now.name} wasn't approved`, done: "You can close this page." }
        // A code that has ended is started again only on its device, so there's nothing to try again here.
        default:
          return { heading: "Approve a device", problem: failed, ...(now.stage === "failed" && now.ended ? {} : { button: "Try Again", onSignIn: begin }) }
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
        case "unlocked": {
          const held = [now.name, ...now.servers].filter((name): name is string => !!name)
          return { heading: "Unlock with Passkey", done: held.length ? `${held.join(", ")} ${held.length === 1 ? "holds" : "hold"} ${now.whose} key now. You can close this page.` : `Nothing is waiting for ${now.whose} key. You can close this page.` }
        }
        case "failed":
          return failed === NO_PRF
            ? { heading: "Unlock with Passkey", problem: "This passkey can't unlock your key. Create one that can, as iCloud Keychain, Google Password Manager, 1Password and Windows Hello do.", button: "Create a Passkey", onSignIn: another, stop: "Try Another Passkey", onStop: signIn }
            : { heading: "Unlock with Passkey", problem: failed, button: "Try Again", onSignIn: signIn }
        default:
          return { heading: "Unlock with Passkey", note: flow.device ? "Use a passkey of yours to let the device hold your key." : flow.scope ? `Use your passkey to hand ${whose(flow.scope)} key to the servers waiting for it.` : "Use your passkey to hand your key to the servers waiting for it.", button: "Continue", onSignIn: signIn }
      }
    case "addPasskey":
      switch (now.stage) {
        // One more tap, for its PRF output, which a browser gives only right after one.
        case "confirm":
          return { heading: "Add a passkey", note: "Use your new passkey once, so it can unlock your key on a new device.", button: "Use Passkey", onSignIn: () => withPasskey().then(() => setStage({ stage: "created" })) }
        case "created":
          return {
            heading: "Add a passkey",
            done: shownCode
              ? `Your passkey is created. Go back to the app, and allow it there if it shows this code: ${shownCode}`
              : "Your passkey is created, and you can sign in with it, but it can't unlock your key on a new device. Passkeys saved in iCloud Keychain or Google Password Manager can.",
          }
        default:
          return { heading: "Add a passkey", button: "Create a passkey", onSignIn: signIn }
      }
    case "join":
      return google() ? { heading: "Join", problem: refused, ...withGoogleFirst("Create a Passkey"), asksName: true } : { heading: "Join", button: "Create a passkey", onSignIn: signIn, asksName: true }
    case "signIn":
      return google() ? { heading: "Sign in", problem: refused, ...withGoogleFirst("Use a Passkey") } : { heading: "Sign in", button: "Sign in", onSignIn: signIn }
  }
}

/// The check mark a device page shows once it's done.
function DoneMark() {
  return (
    <View style={look.done}>
      <Icon name="check" />
    </View>
  )
}

/// Continue with Google as the button, and the passkey as the second way, `passkey` saying what that does.
const withGoogleFirst = (passkey: string) => ({ button: "Continue with Google", onSignIn: withGoogle, stop: passkey, onStop: signIn })

/// The app's logo, or its initial on a plain tile when it has none or it doesn't load, joined to the network's mark.
function Logos(props: { app: App }) {
  return (
    <View style={look.logos}>
      <Show
        when={props.app.logo}
        fallback={
          <View style={look.tile}>
            <Text style={look.initial}>{[...props.app.name][0].toUpperCase()}</Text>
          </View>
        }
      >
        {(logo) => <Image src={logo()} alt="" style={look.logo} />}
      </Show>
      <Text style={look.joins} aria-hidden="true">···</Text>
      <Image src={mark} alt="" style={look.logo} />
    </View>
  )
}

/// Who the app would act as, what it will be able to do, as its token allows, a warning only when its metadata
/// doesn't add up, and where it's taken out again.
/// It reaches `/mcp`, every app's tools, as this person, and with `offline_access` it's given a refresh token.
function Consent(props: { app: App }) {
  const stays = params.get("scope")?.split(" ").includes("offline_access")
  return (
    <>
      <Text style={look.account}>{[props.app.person && `as ${props.app.person}`, location.host].filter(Boolean).join(" · ")}</Text>
      <View style={look.can} role="group">
        <Text style={look.heading} role="heading">{props.app.name} will be able to</Text>
        <Can>Use your apps' tools as you, like email, calendar and files</Can>
        <Can>Reach only what you can, nothing more</Can>
        {stays && <Can>Stay connected until you remove it</Can>}
      </View>
      <Show when={props.app.to}>
        <View style={look.warning}>
          <View style={look.tick}>
            <Icon name="warning-circle" />
          </View>
          <Text style={look.grows}>{props.app.name} says it's from {props.app.from} but sends you back to {props.app.to}. Allow only if you trust {props.app.to}.</Text>
        </View>
      </Show>
      <Text style={look.note}>Allow only apps you trust. You can remove {props.app.name} anytime in Devices.</Text>
    </>
  )
}

/// One thing an app will be able to do, with a check mark before it.
function Can(props: { children: string }) {
  return (
    <View style={look.row}>
      <View style={look.tick}>
        <Icon name="check" />
      </View>
      <Text style={look.grows}>{props.children}</Text>
    </View>
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
        <View style={look.elsewhere}>
          <Text style={look.grey}>Open this where you're signed in</Text>
          <Text style={look.link}>{typed}</Text>
          <Text style={look.grey}>It will show</Text>
          <Text style={look.shown}>{shown}</Text>
        </View>
      )}
      {shows().asksName && !shows().done && (
        <Field>
          <Input name="name" aria-label="Your name" autoFocus autoComplete="name" placeholder="Your name" required value={called()} onChangeText={setCalled} />
        </Field>
      )}
    </SignIn>
  ),
)
