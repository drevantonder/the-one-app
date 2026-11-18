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
/** @param {Uint8Array} bytes */
const hex = (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")

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

// The page asks for this browser's notification key, as hex, on the port it sends.
worker.addEventListener("message", (event) => {
  if (event.data === "key") event.waitUntil(key().then((secret) => event.ports[0].postMessage(hex(ed25519.getPublicKey(secret)))))
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
