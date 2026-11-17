// The push services a notification wakes a device through: Apple's (APNs), Google's (FCM) and a browser's (Web
// Push). Each makes the one request its service documents, with `fetch` given, so a test sees each request as it
// would go out. The keys are the Worker's secrets (README.md); a service whose keys aren't there fails, saying so.
//
// What a device shows is encrypted to it by the server that sent it (`apps/encryption/client/native/src/cipher.rs`, `seal_notification`),
// as `sealed`, and that's all that goes: the push service carries it and reads nothing. On an iPhone, the app's
// notification extension opens it before it shows (`mutable-content`), so Apple has only a fixed line; on Android
// it's a data message, which the app opens and shows itself.
import { buildPushPayload } from "@block65/webcrypto-web-push";
import { importPKCS8, SignJWT } from "jose";

/** What Apple shows on an iPhone that can't open a notification: one too old for the extension. */
export const FIXED = "New notification";

/** The Worker's keys to the push services (README.md), each there only once it's set up. */
export interface Keys {
  APNS_KEY?: string;
  APNS_KEY_ID?: string;
  APNS_TEAM_ID?: string;
  APNS_TOPIC?: string;
  APNS_SANDBOX?: string;
  FCM_ACCOUNT?: string;
  VAPID_PUBLIC_KEY?: string;
  VAPID_PRIVATE_KEY?: string;
  VAPID_SUBJECT?: string;
}

/** A device to wake: which service, its token there, and the notification encrypted to it, as base64. */
export interface Device {
  via: keyof typeof SENDERS;
  token: string;
  sealed: string;
}

/** What a service did: nothing to say, the token's gone, or why it failed. */
type Outcome = { gone?: boolean; failed?: string };
type Fetch = typeof fetch;

/**
 * Wakes each of `devices` (`{ via, token, sealed }`, the notification encrypted to that device, as base64).
 * Gives the tokens whose service says they're gone, which the app forgets, and why each failure failed.
 */
export async function send(fetch: Fetch, env: Keys, devices: Device[]) {
  const gone: string[] = [];
  const failed: { token: string; why: string }[] = [];
  await Promise.all(
    devices.map(async ({ via, token, sealed }) => {
      let outcome: Outcome;
      try {
        outcome = await SENDERS[via](fetch, env, token, sealed);
      } catch (error) {
        outcome = { failed: (error as Error).message };
      }
      if (outcome.gone) gone.push(token);
      if (outcome.failed) failed.push({ token, why: outcome.failed });
    }),
  );
  return { gone, failed };
}

// A sign-in each service gives, kept while it lasts, by whose it is.
const cache = new Map<string, { token: string; until: number }>();
async function token(whose: string, make: () => Promise<{ token: string; lasts: number }>) {
  const had = cache.get(whose);
  if (had && had.until > Date.now()) return had.token;
  const { token, lasts } = await make();
  cache.set(whose, { token, until: Date.now() + lasts });
  return token;
}

/**
 * APNs, with a token from the team's .p8 key: an ES256 JWT, which Apple takes for up to an hour and refuses to
 * have made more often than every 20 minutes, so it's kept 50.
 */
async function apns(fetch: Fetch, env: Keys, device: string, sealed: string): Promise<Outcome> {
  if (!env.APNS_KEY) return { failed: "APNs isn't set up for this network" };
  const bearer = await token("apns", async () => ({
    token: await new SignJWT({ iss: env.APNS_TEAM_ID, iat: Math.floor(Date.now() / 1000) })
      .setProtectedHeader({ alg: "ES256", kid: env.APNS_KEY_ID })
      .sign(await importPKCS8(env.APNS_KEY!, "ES256")),
    lasts: 50 * 60_000,
  }));
  const host = env.APNS_SANDBOX ? "api.sandbox.push.apple.com" : "api.push.apple.com";
  const response = await fetch(`https://${host}/3/device/${device}`, {
    method: "POST",
    headers: {
      authorization: `bearer ${bearer}`,
      "apns-topic": env.APNS_TOPIC ?? "",
      "apns-push-type": "alert",
      "apns-priority": "10",
      "content-type": "application/json",
    },
    body: JSON.stringify({ aps: { alert: { body: FIXED }, "mutable-content": 1, sound: "default" }, n: sealed }),
  });
  if (response.ok) return {};
  const { reason = "" } = await response.json<{ reason?: string }>().catch(() => ({}) as { reason?: string });
  if (response.status === 410 || ["BadDeviceToken", "Unregistered", "DeviceTokenNotForTopic"].includes(reason)) return { gone: true };
  if (["ExpiredProviderToken", "InvalidProviderToken"].includes(reason)) cache.delete("apns");
  return { failed: `APNs answered ${response.status} ${reason}`.trim() };
}

/** FCM's HTTP v1 API, with an OAuth token for the Firebase service account, from its RS256 JWT. */
async function fcm(fetch: Fetch, env: Keys, device: string, sealed: string): Promise<Outcome> {
  if (!env.FCM_ACCOUNT) return { failed: "FCM isn't set up for this network" };
  const account = JSON.parse(env.FCM_ACCOUNT);
  const bearer = await token("fcm", async () => {
    const at = account.token_uri ?? "https://oauth2.googleapis.com/token";
    const assertion = await new SignJWT({ scope: "https://www.googleapis.com/auth/firebase.messaging" })
      .setProtectedHeader({ alg: "RS256", typ: "JWT" })
      .setIssuer(account.client_email)
      .setAudience(at)
      .setIssuedAt()
      .setExpirationTime("1h")
      .sign(await importPKCS8(account.private_key, "RS256"));
    const grant = { grant_type: "urn:ietf:params:oauth:grant-type:jwt-bearer", assertion };
    const response = await fetch(at, { method: "POST", body: new URLSearchParams(grant) });
    const given = await response.json<{ access_token?: string; expires_in: number; error_description?: string }>().catch(() => ({}) as { access_token?: undefined; expires_in: number; error_description?: string });
    if (!given.access_token) throw new Error(`Google gave FCM no token: ${given.error_description ?? response.status}`);
    return { token: given.access_token, lasts: (given.expires_in - 300) * 1000 };
  });
  const message = { token: device, data: { n: sealed }, android: { priority: "HIGH", ttl: "86400s" } };
  const response = await fetch(`https://fcm.googleapis.com/v1/projects/${account.project_id}/messages:send`, {
    method: "POST",
    headers: { authorization: `Bearer ${bearer}`, "content-type": "application/json" },
    body: JSON.stringify({ message }),
  });
  if (response.ok) return {};
  type ErrorBody = { error?: { status?: string; details?: { errorCode?: string }[] } };
  const { error = {} } = await response.json<ErrorBody>().catch(() => ({}) as ErrorBody);
  const code = error.details?.find((detail) => detail.errorCode)?.errorCode ?? error.status ?? "";
  if (response.status === 404 || code === "UNREGISTERED") return { gone: true };
  if (response.status === 401) cache.delete("fcm");
  return { failed: `FCM answered ${response.status} ${code}`.trim() };
}

/** Web Push: RFC 8291's encryption to the browser's subscription, which is the token, and RFC 8292's VAPID. */
async function webpush(fetch: Fetch, env: Keys, device: string, sealed: string): Promise<Outcome> {
  if (!env.VAPID_PRIVATE_KEY) return { failed: "Web Push isn't set up for this network" };
  const subscription = JSON.parse(device);
  const vapid = { subject: env.VAPID_SUBJECT ?? "", publicKey: env.VAPID_PUBLIC_KEY ?? "", privateKey: env.VAPID_PRIVATE_KEY };
  const message = { data: JSON.stringify({ n: sealed }), options: { ttl: 86_400, urgency: "high" as const } };
  const response = await fetch(subscription.endpoint, await buildPushPayload(message, subscription, vapid));
  if (response.ok) return {};
  if (response.status === 404 || response.status === 410) return { gone: true };
  return { failed: `the browser's push service answered ${response.status}` };
}

const SENDERS = { apns, fcm, webpush };
