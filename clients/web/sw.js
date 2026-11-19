// The web client's service worker: what shows a notification in a browser, as a phone's notification extension does
// (`apps/notifications/client/native/src/lib.rs`). It holds this browser's notification key, made here once, which
// the page registers with its push subscription (`host.js`), and a push carries the notification encrypted to it, as
// a server sealed it (`apps/encryption/client/native/src/cipher.rs`): HPKE's base mode with X25519, HKDF-SHA256 and
// ChaCha20-Poly1305, to the key's X25519 form. A tap opens the app it's from, in a window of this address. `edge.js`
// builds it, with what it imports.
/// <reference lib="webworker" />
import { Chacha20Poly1305 } from "@hpke/chacha20poly1305"
import { CipherSuite, HkdfSha256 } from "@hpke/core"
import { DhkemX25519HkdfSha256 } from "@hpke/dhkem-x25519"
import { ed25519 } from "@noble/curves/ed25519.js"
import { bytesToHex as hex } from "@noble/curves/utils.js"
import { get, set } from "idb-keyval"

const worker = /** @type {ServiceWorkerGlobalScope} */ (/** @type {unknown} */ (self))

/** What a browser shows of a notification that doesn't open: Apple's line for one on an iPhone (`senders.ts`). */
const FIXED = "New notification"

/** This browser's notification key, as Ed25519's secret key, made the first time it's asked for. */
async function key() {
  /** @type {Uint8Array | undefined} */
  let secret = await get("notification key")
  if (!secret) await set("notification key", (secret = ed25519.utils.randomSecretKey()))
  return secret
}

const suite = new CipherSuite({ kem: new DhkemX25519HkdfSha256(), kdf: new HkdfSha256(), aead: new Chacha20Poly1305() })

/** A notification as this browser opens it, from what its push carried (`n`, base64): `{title, text, app}`. @param {string} sealed */
async function opened(sealed) {
  const secret = await key()
  const bytes = Uint8Array.from(atob(sealed), (char) => char.charCodeAt(0))
  const recipientKey = await suite.kem.importKey("raw", ed25519.utils.toMontgomerySecret(secret).slice().buffer, false)
  const info = new TextEncoder().encode(`one/notifications/1/notification\n${hex(ed25519.getPublicKey(secret))}`)
  const plain = await suite.open({ recipientKey, enc: bytes.slice(0, 32).buffer, info: info.buffer }, bytes.slice(32).buffer)
  return JSON.parse(new TextDecoder().decode(plain))
}

// It's the newest at once, as it keeps nothing a page could have loaded with the one before.
worker.addEventListener("install", () => worker.skipWaiting())

/// Gives the network this browser's push subscription, made with the network's VAPID key (`/notifications/key`), with
/// its notification key, as a phone gives its push token: registering again is an upsert. A subscription made with
/// another VAPID key is made again. `own` is what the page fills in of the call (`device`, `name`), kept to register
/// again, as the push service replaces the subscription, with no page open.
/** @param {Record<string, string>} own */
async function register(own) {
  const { vapid } = await (await fetch("/notifications/key")).json()
  if (!vapid) throw new Error("This network can't wake browsers yet.")
  const { pushManager } = worker.registration
  let subscription = await pushManager.getSubscription()
  const made = subscription?.options.applicationServerKey
  if (made && btoa(String.fromCharCode(...new Uint8Array(made))).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "") !== vapid) {
    await subscription?.unsubscribe()
    subscription = null
  }
  subscription ??= await pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: vapid })
  const call = { name: "notifications_register", arguments: { ...own, via: "webpush", token: JSON.stringify(subscription), key: hex(ed25519.getPublicKey(await key())) }, _meta: { "io.modelcontextprotocol/clientInfo": { name: "one-web" } } }
  const response = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: call }) })
  if (!response.ok) throw new Error(response.status === 401 ? "Sign in again." : "The network didn't answer.")
  const { result, error } = await response.json()
  if (error || result?.isError) throw new Error(error?.message ?? result?.content?.[0]?.text ?? "The network didn't take it.")
  await set("registration", own)
}

// The page has this browser register, on the port it sends, and hears how that went.
worker.addEventListener("message", (event) => {
  if (event.data?.register) event.waitUntil(register(event.data.register).then(() => event.ports[0].postMessage({}), (error) => event.ports[0].postMessage({ error: error.message })))
})

// The push service replaced this browser's subscription (it expired, or it was reset), which the network holds the
// old of: subscribe again and register that, as the page last registered, with no page open, as MDN says to. A browser
// that never registered here has nothing to replace.
worker.addEventListener("pushsubscriptionchange", (event) => {
  const again = async () => {
    const own = await get("registration")
    if (own) await register(own)
  }
  event.waitUntil(again().catch((error) => console.error("a new subscription wasn't registered", error)))
})

// Every push shows a notification, as a browser requires: one that doesn't open shows the fixed line.
worker.addEventListener("push", (event) => {
  const show = async () => {
    /** @type {{ title: string, text?: string, app?: string }} */
    let notification = { title: FIXED }
    try {
      notification = await opened(event.data?.json().n)
    } catch (error) {
      console.error("a notification didn't open", error)
    }
    await worker.registration.showNotification(notification.title, { body: notification.text, icon: "/Logo.png", data: { app: notification.app } })
  }
  event.waitUntil(show())
})

// A tap opens the app it's from (`<app>[/<page>][#<fragment>]`) in a window of this address that's open, or a new one,
// as the web client opens a link to an app (`/?open=`).
worker.addEventListener("notificationclick", (event) => {
  event.notification.close()
  /** @type {string | undefined} */
  const app = event.notification.data?.app
  const open = async () => {
    const [window] = await worker.clients.matchAll({ type: "window", includeUncontrolled: true })
    if (!window) return void (await worker.clients.openWindow(app ? `/?open=${encodeURIComponent(app)}` : "/"))
    if (app) window.postMessage({ open: app })
    await window.focus()
  }
  event.waitUntil(open())
})
