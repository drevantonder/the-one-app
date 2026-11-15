// Each sender against a fake of its push service: what it asks, and what it makes of each answer. Nothing here
// reaches Apple, Google or a browser's push service.
import { expect, test } from "bun:test";
import { decodeProtectedHeader, importSPKI, jwtVerify } from "jose";
import { send } from "./senders.js";

const pem = (der, kind) => `-----BEGIN ${kind}-----\n${Buffer.from(der).toString("base64")}\n-----END ${kind}-----`;
const base64url = (bytes) => Buffer.from(bytes).toString("base64url");

const ec = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
const rsa = await crypto.subtle.generateKey(
  { name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" },
  true,
  ["sign", "verify"],
);
const vapid = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
// A browser's subscription: its own key pair and secret.
const browser = await crypto.subtle.generateKey({ name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]);
const subscription = {
  endpoint: "https://fcm.googleapis.com/fcm/send/abc",
  keys: { p256dh: base64url(await crypto.subtle.exportKey("raw", browser.publicKey)), auth: base64url(crypto.getRandomValues(new Uint8Array(16))) },
};

const env = {
  APNS_KEY: pem(await crypto.subtle.exportKey("pkcs8", ec.privateKey), "PRIVATE KEY"),
  APNS_KEY_ID: "KEY123",
  APNS_TEAM_ID: "TEAM123",
  APNS_TOPIC: "app.theone.client",
  FCM_ACCOUNT: JSON.stringify({
    project_id: "the-one-app",
    client_email: "push@the-one-app.iam.gserviceaccount.com",
    private_key: pem(await crypto.subtle.exportKey("pkcs8", rsa.privateKey), "PRIVATE KEY"),
  }),
  VAPID_PUBLIC_KEY: base64url(await crypto.subtle.exportKey("raw", vapid.publicKey)),
  VAPID_PRIVATE_KEY: (await crypto.subtle.exportKey("jwk", vapid.privateKey)).d,
  VAPID_SUBJECT: "mailto:owner@example.com",
};

/** A fetch that answers each request with what `answer` gives for it, and keeps what was asked. */
function fake(answer) {
  const asked = [];
  const fetch = async (url, init) => {
    asked.push({ url: String(url), ...init });
    return answer(String(url), init);
  };
  return { fetch, asked };
}

const notification = { title: "Sam approved your claim", text: "Lunch with Sam, $42", app: "claims" };

test("APNs is asked with the team's token, for the app, with what it shows", async () => {
  const { fetch, asked } = fake(() => new Response(null, { status: 200 }));
  expect(await send(fetch, env, [{ via: "apns", token: "a1b2" }], notification)).toEqual({ gone: [], failed: [] });
  const [{ url, headers, body }] = asked;
  expect(url).toBe("https://api.push.apple.com/3/device/a1b2");
  expect(headers["apns-topic"]).toBe("app.theone.client");
  const bearer = headers.authorization.slice("bearer ".length);
  expect(decodeProtectedHeader(bearer)).toEqual({ alg: "ES256", kid: "KEY123" });
  const key = await importSPKI(pem(await crypto.subtle.exportKey("spki", ec.publicKey), "PUBLIC KEY"), "ES256");
  expect((await jwtVerify(bearer, key)).payload.iss).toBe("TEAM123");
  expect(JSON.parse(body)).toEqual({ aps: { alert: { title: notification.title, body: notification.text }, sound: "default" }, app: "claims" });
});

test("an iPhone APNs says is gone is gone, and another failure says why", async () => {
  const { fetch } = fake((url) =>
    url.endsWith("/gone")
      ? Response.json({ reason: "Unregistered" }, { status: 410 })
      : Response.json({ reason: "TooManyRequests" }, { status: 429 }),
  );
  const devices = [{ via: "apns", token: "gone" }, { via: "apns", token: "busy" }];
  expect(await send(fetch, env, devices, notification)).toEqual({ gone: ["gone"], failed: [{ token: "busy", why: "APNs answered 429 TooManyRequests" }] });
});

test("FCM is asked with a token Google gave for the service account", async () => {
  const { fetch, asked } = fake((url) =>
    url === "https://oauth2.googleapis.com/token" ? Response.json({ access_token: "google-token", expires_in: 3600 }) : Response.json({ name: "sent" }),
  );
  expect(await send(fetch, env, [{ via: "fcm", token: "phone" }], notification)).toEqual({ gone: [], failed: [] });
  const [grant, sent] = asked;
  const assertion = new URLSearchParams(grant.body).get("assertion");
  const key = await importSPKI(pem(await crypto.subtle.exportKey("spki", rsa.publicKey), "PUBLIC KEY"), "RS256");
  const { payload } = await jwtVerify(assertion, key, { audience: "https://oauth2.googleapis.com/token" });
  expect(payload.iss).toBe("push@the-one-app.iam.gserviceaccount.com");
  expect(sent.url).toBe("https://fcm.googleapis.com/v1/projects/the-one-app/messages:send");
  expect(sent.headers.authorization).toBe("Bearer google-token");
  expect(JSON.parse(sent.body)).toEqual({
    message: { token: "phone", notification: { title: notification.title, body: notification.text }, data: { app: "claims" } },
  });
});

test("a browser is sent what it shows, encrypted to its subscription and signed with the network's VAPID key", async () => {
  const { fetch, asked } = fake(() => new Response(null, { status: 201 }));
  expect(await send(fetch, env, [{ via: "webpush", token: JSON.stringify(subscription) }], notification)).toEqual({ gone: [], failed: [] });
  const [{ url, headers, body }] = asked;
  expect(url).toBe(subscription.endpoint);
  expect(headers["content-encoding"]).toBe("aes128gcm");
  expect(headers.authorization).toStartWith("vapid t=");
  expect(headers.authorization).toEndWith(`k=${env.VAPID_PUBLIC_KEY}`);
  expect(new TextDecoder().decode(body)).not.toContain(notification.title);
});

test("a service with no keys fails, saying so", async () => {
  const { fetch, asked } = fake(() => new Response(null, { status: 200 }));
  const devices = [{ via: "apns", token: "a" }, { via: "fcm", token: "b" }, { via: "webpush", token: JSON.stringify(subscription) }];
  expect(await send(fetch, {}, devices, notification)).toEqual({
    gone: [],
    failed: [
      { token: "a", why: "APNs isn't set up for this network" },
      { token: "b", why: "FCM isn't set up for this network" },
      { token: JSON.stringify(subscription), why: "Web Push isn't set up for this network" },
    ],
  });
  expect(asked).toEqual([]);
});
