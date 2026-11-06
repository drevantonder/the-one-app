// The push Worker's four senders, one per push service: each takes a device's address as it registered and the bytes
// to carry (a note sealed to that device, which nobody on the way opens), and makes the one request its service
// documents. `fetch` is given, so tests see each request as it would go out. Each says `sent`, `gone` (the address no
// longer works: the app was deleted, say, so it's forgotten) or `failed` with why.
//
// - APNs (iPhone, iPad, Mac): token-based provider sign-in, an ES256 JWT of the team's .p8 key, kept 50 minutes (Apple
//   refuses one older than an hour, or made more often than every 20). A note goes as a visible alert with
//   `mutable-content`, so the app's Notification Service Extension opens it before it shows; what Apple sees is only
//   the network's name and a fixed line. A quiet one is a background push that wakes the app.
// - FCM HTTP v1 (Android): an OAuth token from the service account's RS256 JWT, a data message only (FCM shows
//   nothing itself), high priority unless quiet.
// - WNS (Windows): an OAuth token for the app's Microsoft Entra registration (client credentials, scope
//   https://wns.windows.com/.default), a raw notification, its body the sealed bytes as they are.
// - Web Push (browsers): RFC 8291's aes128gcm to the browser's p256dh and auth, and RFC 8292's VAPID, an ES256 JWT of
//   the network's VAPID key for the push service's origin, good for 12 hours (push services refuse more than 24). The
//   note goes as base64url text: Edge's push service drops a payload that isn't text.

import { b64u, concat } from "../shared/crypto.js";
import { NOTE_FALLBACK } from "../shared/protocol.js";

const text = new TextEncoder();
const json64 = (v) => b64u.encode(text.encode(JSON.stringify(v)));

export function pem(textOf) {
  const m = /-----BEGIN [^-]+-----([\s\S]*?)-----END /.exec(String(textOf || ""));
  if (!m) throw new Error("not a PEM key");
  return Uint8Array.from(atob(m[1].replace(/\s+/g, "")), (c) => c.charCodeAt(0));
}

export async function jwt(header, claims, key) {
  const signing = `${json64(header)}.${json64(claims)}`;
  const algorithm = header.alg === "ES256" ? { name: "ECDSA", hash: "SHA-256" } : { name: "RSASSA-PKCS1-v1_5" };
  return `${signing}.${b64u.encode(await crypto.subtle.sign(algorithm, key, text.encode(signing)))}`;
}

const tokens = new Map();
async function kept(whose, ttl, make) {
  const now = Date.now();
  const had = tokens.get(whose);
  if (had && had.until > now) return had.token;
  const made = await make();
  const token = typeof made === "string" ? made : made.token;
  tokens.set(whose, { token, until: now + (typeof made === "string" ? ttl : Math.min(ttl, made.ttl)) });
  return token;
}
const forget = (whose) => tokens.delete(whose);

/// `config`: `{ team, key_id, key }` (the .p8's text). `device`: `{ address, env, topic }`. `note`: the sealed bytes.
export async function apns(fetch, config, device, { note, quiet, name }) {
  if (!config?.key) return { failed: "APNs isn't set up for this network" };
  const whose = `apns:${config.team}/${config.key_id}`;
  const token = await kept(whose, 50 * 60_000, async () => {
    const key = await crypto.subtle.importKey("pkcs8", pem(config.key), { name: "ECDSA", namedCurve: "P-256" }, false, ["sign"]);
    return jwt({ alg: "ES256", kid: config.key_id }, { iss: config.team, iat: Math.floor(Date.now() / 1000) }, key);
  });
  const body = quiet
    ? { aps: { "content-available": 1 }, n: b64u.encode(note) }
    : { aps: { alert: { title: name, body: NOTE_FALLBACK }, "mutable-content": 1, sound: "default" }, n: b64u.encode(note) };
  const host = device.env === "sandbox" ? "api.sandbox.push.apple.com" : "api.push.apple.com";
  const resp = await fetch(`https://${host}/3/device/${device.address}`, {
    method: "POST",
    headers: {
      authorization: `bearer ${token}`,
      "apns-topic": device.topic,
      "apns-push-type": quiet ? "background" : "alert",
      "apns-priority": quiet ? "5" : "10",
      "apns-expiration": String(Math.floor(Date.now() / 1000) + 86_400),
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (resp.status === 200) return { sent: true };
  const reason = (await resp.json().catch(() => ({}))).reason || "";
  if (resp.status === 410 || reason === "BadDeviceToken" || reason === "Unregistered" || reason === "DeviceTokenNotForTopic") return { gone: true };
  if (reason === "ExpiredProviderToken" || reason === "InvalidProviderToken") forget(whose);
  return { failed: `APNs said ${resp.status} ${reason}`.trim() };
}

/// `account`: the Firebase service account's JSON key (`{ project_id, client_email, private_key, token_uri }`).
export async function fcm(fetch, account, device, { note, quiet }) {
  if (!account?.private_key) return { failed: "FCM isn't set up for this network" };
  const whose = `fcm:${account.client_email}`;
  const token = await kept(whose, 50 * 60_000, async () => {
    const key = await crypto.subtle.importKey("pkcs8", pem(account.private_key), { name: "RSASSA-PKCS1-v1_5", hash: "SHA-256" }, false, ["sign"]);
    const now = Math.floor(Date.now() / 1000);
    const uri = account.token_uri || "https://oauth2.googleapis.com/token";
    const assertion = await jwt({ alg: "RS256", typ: "JWT" }, { iss: account.client_email, scope: "https://www.googleapis.com/auth/firebase.messaging", aud: uri, iat: now, exp: now + 3600 }, key);
    const resp = await fetch(uri, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer", assertion }).toString() });
    const said = await resp.json().catch(() => ({}));
    if (!resp.ok || !said.access_token) throw new Error(`Google didn't give FCM a token: ${said.error_description || said.error || resp.status}`);
    return { token: said.access_token, ttl: Math.max(60, (said.expires_in || 3600) - 300) * 1000 };
  });
  const message = { message: { token: device.address, data: { n: b64u.encode(note) }, android: { priority: quiet ? "NORMAL" : "HIGH", ttl: "86400s" } } };
  const resp = await fetch(`https://fcm.googleapis.com/v1/projects/${account.project_id}/messages:send`, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json" },
    body: JSON.stringify(message),
  });
  if (resp.ok) return { sent: true };
  const error = (await resp.json().catch(() => ({}))).error || {};
  const code = (error.details || []).map((d) => d.errorCode).find(Boolean) || error.status || "";
  if (resp.status === 404 || code === "UNREGISTERED") return { gone: true };
  if (resp.status === 401) forget(whose);
  return { failed: `FCM said ${resp.status} ${code}`.trim() };
}

/// `app`: `{ tenant, client_id, client_secret }`, the Windows app's Entra registration. `device.address`: its channel URI.
export async function wns(fetch, app, device, { note }) {
  if (!app?.client_secret) return { failed: "WNS isn't set up for this network" };
  if (!isWnsChannel(device.address)) return { gone: true };
  const whose = `wns:${app.tenant}/${app.client_id}`;
  const token = await kept(whose, 50 * 60_000, async () => {
    const resp = await fetch(`https://login.microsoftonline.com/${encodeURIComponent(app.tenant)}/oauth2/v2.0/token`, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body: new URLSearchParams({ grant_type: "client_credentials", client_id: app.client_id, client_secret: app.client_secret, scope: "https://wns.windows.com/.default" }).toString(),
    });
    const said = await resp.json().catch(() => ({}));
    if (!resp.ok || !said.access_token) throw new Error(`Microsoft didn't give WNS a token: ${said.error_description || said.error || resp.status}`);
    return { token: said.access_token, ttl: Math.max(60, (said.expires_in || 3600) - 300) * 1000 };
  });
  const resp = await fetch(device.address, {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/octet-stream", "x-wns-type": "wns/raw", "x-wns-cache-policy": "cache", "x-wns-ttl": "86400" },
    body: note,
  });
  if (resp.ok) return { sent: true };
  if (resp.status === 404 || resp.status === 410) return { gone: true };
  if (resp.status === 401) forget(whose);
  return { failed: `WNS said ${resp.status} ${resp.headers.get("x-wns-error-description") || ""}`.trim() };
}

export const isWnsChannel = (address) => {
  try {
    const url = new URL(address);
    return url.protocol === "https:" && (url.hostname === "notify.windows.com" || url.hostname.endsWith(".notify.windows.com"));
  } catch {
    return false;
  }
};

/// The push services browsers subscribe with, by host: Chrome's and Edge's, Firefox's and Safari's. A browser's
/// endpoint is anywhere else only if it isn't a browser.
const WEB_PUSH_HOSTS = [/^fcm\.googleapis\.com$/, /^updates\.push\.services\.mozilla\.com$/, /(^|\.)push\.apple\.com$/, /(^|\.)notify\.windows\.com$/];

export const isWebPushEndpoint = (address) => {
  try {
    const url = new URL(address);
    return url.protocol === "https:" && WEB_PUSH_HOSTS.some((h) => h.test(url.hostname));
  } catch {
    return false;
  }
};

/// A VAPID key: its private half as base64url PKCS#8 → `{ key, public }`, the CryptoKey and its public point (65 bytes).
export async function vapidKey(pkcs8) {
  const der = b64u.decode(pkcs8);
  const key = await crypto.subtle.importKey("pkcs8", der, { name: "ECDSA", namedCurve: "P-256" }, true, ["sign"]);
  const jwk = await crypto.subtle.exportKey("jwk", key);
  return { key, public: concat(new Uint8Array([4]), b64u.decode(jwk.x), b64u.decode(jwk.y)) };
}

/// RFC 8291: `plain` encrypted for a browser's subscription (`p256dh`, its 65-byte public key, and `auth`, its 16-byte
/// secret), as one aes128gcm record. `ephemeral` and `salt` are fresh unless a test gives them.
export async function encrypt(plain, p256dh, auth, { ephemeral, salt = crypto.getRandomValues(new Uint8Array(16)) } = {}) {
  const pair = ephemeral || (await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]));
  const asPublic = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const uaPublic = await crypto.subtle.importKey("raw", p256dh, { name: "ECDH", namedCurve: "P-256" }, false, []);
  const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "ECDH", public: uaPublic }, pair.privateKey, 256));
  const hkdf = async (salt, ikm, info, bits) =>
    new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info }, await crypto.subtle.importKey("raw", ikm, "HKDF", false, ["deriveBits"]), bits));
  const ikm = await hkdf(auth, shared, concat(text.encode("WebPush: info\0"), p256dh, asPublic), 256);
  const cek = await hkdf(salt, ikm, text.encode("Content-Encoding: aes128gcm\0"), 128);
  const nonce = await hkdf(salt, ikm, text.encode("Content-Encoding: nonce\0"), 96);
  const key = await crypto.subtle.importKey("raw", cek, "AES-GCM", false, ["encrypt"]);
  // One record, the last: its padding delimiter is 2.
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce }, key, concat(plain, new Uint8Array([2]))));
  const rs = new Uint8Array(4);
  new DataView(rs.buffer).setUint32(0, 4096);
  return concat(salt, rs, new Uint8Array([asPublic.length]), asPublic, sealed);
}

/// `vapid`: `{ key, public, subject }`, the network's VAPID key and who it is (its address). `device`: `{ address,
/// keys: { p256dh, auth } }`, the subscription. `note`: the bytes to carry.
export async function webPush(fetch, vapid, device, { note, quiet }) {
  if (!vapid?.key) return { failed: "Web Push has no VAPID key in this network" };
  if (!isWebPushEndpoint(device.address)) return { gone: true };
  const audience = new URL(device.address).origin;
  const token = await kept(`vapid:${audience}`, 6 * 3_600_000, () =>
    jwt({ typ: "JWT", alg: "ES256" }, { aud: audience, exp: Math.floor(Date.now() / 1000) + 12 * 3600, sub: vapid.subject }, vapid.key),
  );
  const body = await encrypt(text.encode(b64u.encode(note)), b64u.decode(device.keys.p256dh), b64u.decode(device.keys.auth));
  const resp = await fetch(device.address, {
    method: "POST",
    headers: { authorization: `vapid t=${token}, k=${b64u.encode(vapid.public)}`, "content-encoding": "aes128gcm", "content-type": "application/octet-stream", ttl: "86400", urgency: quiet ? "normal" : "high" },
    body,
  });
  if (resp.ok) return { sent: true };
  if (resp.status === 404 || resp.status === 410) return { gone: true };
  if (resp.status === 401 || resp.status === 403) forget(`vapid:${audience}`);
  return { failed: `the browser's push service said ${resp.status}` };
}
