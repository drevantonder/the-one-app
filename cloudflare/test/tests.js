// The Workers' own tests, run by `workerd test` (tests/cloudflare.rs runs them with cargo test).
import { b64u, databaseKey, idOf, keyPair, openAppKey, openSealed, openUnder, ed25519Public, random, READS, sealAppKey, sealTo, sealUnder, signedRequest } from "../shared/crypto.js";
import signer from "../signer/worker.js";
import { presign } from "../shared/s3.js";
import { verify } from "../shared/token.js";
import { Copies } from "../mcp/copies.js";
import { apns, encrypt, fcm, webPush, wns } from "../push/senders.js";

/// A Durable Object's storage, in memory.
class Store {
  constructor() {
    this.kept = new Map();
  }
  async get(k) {
    return structuredClone(this.kept.get(k));
  }
  async put(k, v) {
    this.kept.set(k, structuredClone(v));
  }
  async delete(k) {
    for (const one of [k].flat()) this.kept.delete(one);
  }
  async list({ prefix }) {
    return new Map([...this.kept].filter(([k]) => k.startsWith(prefix)));
  }
}

function equal(a, b, what) {
  if (JSON.stringify(a) !== JSON.stringify(b)) throw new Error(`${what}: ${JSON.stringify(a)} isn't ${JSON.stringify(b)}`);
}

async function fails(f, what) {
  try {
    await f();
  } catch {
    return;
  }
  throw new Error(`${what} didn't fail`);
}

export default {
  async test() {
    // AWS's own example of a presigned URL (Signature Version 4, "Authenticating Requests: Using Query Parameters").
    const url = await presign({
      method: "GET", host: "examplebucket.s3.amazonaws.com", path: "/test.txt", region: "us-east-1",
      keyId: "AKIAIOSFODNN7EXAMPLE", secret: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY", expires: 86400,
      date: new Date("2013-05-24T00:00:00Z"),
    });
    equal(new URL(url).searchParams.get("X-Amz-Signature"), "aeeed9bbccd4d02ee5c0109b86d86835f995330da4c265957d157751f604d404", "AWS's example signature");
    const create = new URL(await presign({ method: "PUT", host: "h", path: "/b/k", region: "auto", keyId: "k", secret: "s", expires: 60, headers: { "If-None-Match": "*" } }));
    equal(create.searchParams.get("X-Amz-SignedHeaders"), "host;if-none-match", "a create URL signs If-None-Match in");

    // An app's key opens with its scope's key, under its own name only; a database's derives from its app's.
    const scope = random(32);
    const app = random(32);
    const sealedApp = await sealAppKey(scope, "tally", app);
    equal((await openAppKey(scope, "tally", sealedApp)).join(), app.join(), "opening an app's key");
    await fails(() => openAppKey(scope, "tallys", sealedApp), "opening it as another app's");
    await fails(() => openAppKey(random(32), "tally", sealedApp), "opening it with another scope's key");
    if ((await databaseKey(app, "main")).join() === app.join()) throw new Error("a database has its app's key");

    // Sealing to a key pair opens only with it.
    const me = await keyPair(), them = await keyPair();
    const sealed = await sealTo(me.public, scope);
    equal((await openSealed(me, sealed)).join(), scope.join(), "opening what was sealed to me");
    await fails(() => openSealed(them, sealed), "opening what was sealed to someone else");

    // Sealing under a secret opens only with it, and its id names it without opening it.
    const under = await sealUnder("a refresh token, long enough", scope);
    equal((await openUnder("a refresh token, long enough", under)).join(), scope.join(), "opening under the secret");
    await fails(() => openUnder("another refresh token, long", under), "opening under another secret");
    if ((await idOf("one secret here")) === (await idOf("another secret"))) throw new Error("two secrets share an id");

    // A token checks only with the key that signed it, from its issuer, before it expires.
    const pair = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    const jwk = { ...(await crypto.subtle.exportKey("jwk", pair.publicKey)), kid: "k1" };
    const sign = async (claims, kid = "k1") => {
      const enc = (v) => b64u.encode(new TextEncoder().encode(JSON.stringify(v)));
      const signing = `${enc({ alg: "EdDSA", kid })}.${enc(claims)}`;
      return `${signing}.${b64u.encode(await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, new TextEncoder().encode(signing)))}`;
    };
    const exp = Math.floor(Date.now() / 1000) + 60;
    equal((await verify(await sign({ iss: "https://a", sub: "u", exp }), { keys: [jwk] }, "https://a")).sub, "u", "a good token");
    await fails(async () => verify(await sign({ iss: "https://b", sub: "u", exp }), { keys: [jwk] }, "https://a"), "another issuer's token");
    await fails(async () => verify(await sign({ iss: "https://a", sub: "u", exp: exp - 120 }), { keys: [jwk] }, "https://a"), "an expired token");
    await fails(async () => verify(await sign({ iss: "https://a", sub: "u", exp }, "k2"), { keys: [jwk] }, "https://a"), "a token from an unknown key");
    await remoteMcpClientsSecrets();
    await browsersReadStepsFromTheBucket();
    await stepsAreCreatedInTurn();
    await anOlderBuildWritesNothing();
    await pushRequests();
    const token = await sign({ iss: "https://a", sub: "u", exp });
    const forged = token.slice(0, token.lastIndexOf(".") + 1) + b64u.encode(new Uint8Array(64));
    await fails(() => verify(forged, { keys: [jwk] }, "https://a"), "a forged token");
  },
};

/// A browser reads a database's steps from the bucket itself, through the URL signer's `get`, as the bucket holds them
/// (sealed: it opens nothing), each by its name and null past the last: never as a node passes them on, which could
/// hand it junk for a real step. Only a device of the install's reaches it.
async function browsersReadStepsFromTheBucket() {
  const objects = new Map([["global/node/scope/000000000001", new Uint8Array([1, 2, 3])], ["global/node/scope/000000000002", new Uint8Array([4])]]);
  const R2 = { get: async (name) => (objects.has(name) ? { arrayBuffer: async () => objects.get(name).buffer } : null) };
  const device = random(32);
  const env = { R2, BUCKET: JSON.stringify({ endpoint: "https://s3.test", bucket: "b", region: "auto", key_id: "k", secret: "s" }), RELAY: { allowed: async () => ({}) } };
  const ask = async (fields, e = env, key = device) => {
    const body = await signedRequest(key, await ed25519Public(key), "one/urls", { reads: READS, ...fields }, Date.now());
    return signer.fetch(new Request("https://one.test/urls", { method: "POST", body: JSON.stringify(body) }), e);
  };
  const r = await ask({ get: [{ steps: "global/node/scope", from: 1, to: 3 }] });
  const answer = await r.json();
  if (!answer.get) throw new Error(`the signer said ${r.status}: ${JSON.stringify(answer)}`);
  const got = answer.get;
  equal(Object.keys(got), ["global/node/scope/000000000001", "global/node/scope/000000000002", "global/node/scope/000000000003"], "a range's names");
  equal([b64u.decode(got["global/node/scope/000000000001"]).join(), b64u.decode(got["global/node/scope/000000000002"]).join(), got["global/node/scope/000000000003"]], ["1,2,3", "4", null], "each step's bytes as the bucket holds them, and null past the last");
  equal((await ask({ get: [{ steps: "global/node/scope", from: 1, to: 33 }] })).status, 400, "at most 32 at once");
  equal((await ask({ get: ["global/node/scope/000000000001"] }, { ...env, RELAY: { allowed: async () => null } }, random(32))).status, 403, "only the install's devices");
}

/// The URL signer lets a device whose build reads less of who's who (an older one says nothing) create, replace or
/// remove nothing, and a client alone read nothing; a machine that runs things still reads, so it takes the release it
/// updates to.
async function anOlderBuildWritesNothing() {
  const R2 = { head: async () => null };
  const env = (role) => ({ R2, BUCKET: JSON.stringify({ endpoint: "https://s3.test", bucket: "b", region: "auto", key_id: "k", secret: "s" }), RELAY: { allowed: async () => ({ admin: false, role }) } });
  // A new device each time: the signer keeps what the relay said of a key for a few seconds.
  const ask = async (fields, e) => {
    const device = random(32);
    return (await signer.fetch(new Request("https://one.test/urls", { method: "POST", body: JSON.stringify(await signedRequest(device, await ed25519Public(device), "one/urls", fields, Date.now())) }), e)).status;
  };
  const write = { create: ["s/a/blobs/x"] }, read = { read: ["s/a/blobs/x"] };
  equal([await ask(write, env("compute")), await ask(write, env("client compute"))], [426, 426], "an older node writes nothing");
  equal(await ask(read, env("compute")), 200, "and still reads");
  equal(await ask(read, env("client")), 426, "an older client reads nothing");
  equal([await ask({ ...write, reads: READS }, env("compute")), await ask({ ...read, reads: READS }, env("client"))], [200, 200], "a build that reads it does both");
}

/// The URL signer creates a step only once the one before it is there (or the copy up to it), and a copy only once its
/// step is: an endpoint without the keys can't leave a gap in a database's or log's steps, nor plant one past their
/// end. A writer that asks for the positions after its own in the same trip, as an older one does, is given its own.
async function stepsAreCreatedInTurn() {
  const there = new Set(["s/a/node/log/000000000001", "s/a/node/log/000000000002", "s/a/db/copy/000000000005"]);
  const R2 = { head: async (name) => (there.has(name) ? {} : null) };
  const device = random(32);
  const env = { R2, BUCKET: JSON.stringify({ endpoint: "https://s3.test", bucket: "b", region: "auto", key_id: "k", secret: "s" }), RELAY: { allowed: async () => ({}) } };
  const ask = async (fields) => {
    const body = await signedRequest(device, await ed25519Public(device), "one/urls", { reads: READS, ...fields }, Date.now());
    return signer.fetch(new Request("https://one.test/urls", { method: "POST", body: JSON.stringify(body) }), env);
  };
  const created = async (create) => {
    const r = await ask({ create });
    return r.ok ? Object.keys((await r.json()).create).sort() : r.status;
  };
  equal(await created([{ steps: "s/a/node/log", from: 3, to: 6 }]), ["s/a/node/log/000000000003"], "the next step, and none after it yet");
  equal(await created(["s/a/node/log/000000000004"]), 409, "not one past the end");
  equal(await created(["s/a/node/scope/000000000001", "s/a/db/000000000006", "s/a/node/log/000000000003.sig", "s/a/blobs/x"]), ["s/a/blobs/x", "s/a/db/000000000006", "s/a/node/log/000000000003.sig", "s/a/node/scope/000000000001"], "a first step, one after its copy, a signature and anything else");
  equal(await created(["s/a/node/log/copy/000000000002", "s/a/node/log/copy/000000000003"]), ["s/a/node/log/copy/000000000002"], "a copy only after its step");
}

/// A remote MCP client's device secret opens only with its current refresh token or a live access token; a refresh
/// token used once opens nothing after, nor rotates anything.
async function remoteMcpClientsSecrets() {
  let now = 1_000_000;
  const copies = new Copies(new Store(), () => now);
  const secret = random(64);
  const same = (got, what) => equal(got && [...got].join(), [...secret].join(), what);
  await copies.start(secret, "refresh token one", "access token one", now + 3_600_000);
  same(await copies.open("refresh token one"), "the refresh token opens it");
  same(await copies.open("access token one"), "the access token opens it");
  equal(await copies.open("someone else's token"), null, "another token opens it");

  now += 600_000;
  same(await copies.rotate("refresh token one", "refresh token two", "access token two", now + 3_600_000), "a refresh opens it with the old refresh token");
  same(await copies.open("refresh token two"), "the new refresh token opens it");
  same(await copies.open("access token two"), "the new access token opens it");
  same(await copies.open("access token one"), "an access token opens it until it expires");
  equal(await copies.open("refresh token one"), null, "the old refresh token opens it once used");
  same(await copies.rotate("refresh token one", "refresh token two", "access token two", now + 3_600_000), "sign-in's answer to a refresh, given again, opens it");
  equal(await copies.rotate("refresh token one", "refresh token stolen", "access token stolen", now + 3_600_000), null, "an old refresh token reused rotates it");
  equal(await copies.open("refresh token stolen"), null, "a reused refresh token's new tokens open it");

  now += 3_600_000;
  equal(await copies.open("access token one"), null, "an expired access token opens it");
  equal(await copies.open("access token two"), null, "an expired access token opens it");
  same(await copies.open("refresh token two"), "the current refresh token opens it");

  // A sign-in again starts over: no token from before opens what it seals.
  await copies.start(random(64), "refresh token three", "access token three", now + 3_600_000);
  equal(await copies.open("refresh token two"), null, "a refresh token from before a new sign-in opens it");
}

/// Each push service gets the request it documents, carrying the sealed note as it is; and Web Push's encryption is
/// RFC 8291's, as its own example says.
async function pushRequests() {
  const text = new TextEncoder();
  const asked = [];
  const answer = (status, body = {}) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fetch = async (url, init) => {
    asked.push({ url: String(url), ...init });
    if (String(url).includes("oauth2.googleapis.com") || String(url).includes("login.microsoftonline.com")) return answer(200, { access_token: "a token", expires_in: 3600 });
    if (String(url).includes("/gone")) return answer(410, { reason: "Unregistered" });
    return answer(200);
  };
  const pemOf = (der, label) => `-----BEGIN ${label}-----\n${btoa(String.fromCharCode(...new Uint8Array(der))).replace(/(.{64})/g, "$1\n")}\n-----END ${label}-----\n`;
  const jwtParts = (token) => token.split(".").map((p, i) => (i < 2 ? JSON.parse(new TextDecoder().decode(b64u.decode(p))) : b64u.decode(p)));
  const note = random(120);

  // APNs: a visible alert the app's extension opens, to the token's environment, with a JWT of the team's key.
  const ec = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const config = { team: "TEAM123456", key_id: "KEY1234567", key: pemOf(await crypto.subtle.exportKey("pkcs8", ec.privateKey), "PRIVATE KEY") };
  equal(await apns(fetch, config, { address: "ab12", env: "sandbox", topic: "app.one.phone" }, { note, name: "The One App" }), { sent: true }, "APNs takes a note");
  let a = asked.pop();
  equal(a.url, "https://api.sandbox.push.apple.com/3/device/ab12", "APNs' URL");
  equal([a.headers["apns-topic"], a.headers["apns-push-type"], a.headers["apns-priority"]], ["app.one.phone", "alert", "10"], "APNs' headers");
  const payload = JSON.parse(a.body);
  equal([payload.aps["mutable-content"], payload.aps.alert.title, b64u.decode(payload.n).join()], [1, "The One App", note.join()], "APNs' payload carries the note, and nothing of it");
  const [header, claims, sig] = jwtParts(a.headers.authorization.replace("bearer ", ""));
  equal([header.alg, header.kid, claims.iss], ["ES256", "KEY1234567", "TEAM123456"], "APNs' token");
  const signing = a.headers.authorization.replace("bearer ", "").split(".").slice(0, 2).join(".");
  if (!(await crypto.subtle.verify({ name: "ECDSA", hash: "SHA-256" }, ec.publicKey, sig, text.encode(signing)))) throw new Error("APNs' token isn't signed with the team's key");
  await apns(fetch, config, { address: "cd34", env: "production", topic: "app.one.phone" }, { note, quiet: true });
  a = asked.pop();
  equal([a.url, a.headers["apns-push-type"], a.headers["apns-priority"], JSON.parse(a.body).aps], ["https://api.push.apple.com/3/device/cd34", "background", "5", { "content-available": 1 }], "a quiet push wakes the app");
  equal(await apns(fetch, config, { address: "gone", env: "production", topic: "t" }, { note }), { gone: true }, "a token APNs says is gone");
  asked.length = 0;

  // FCM HTTP v1: an OAuth token from the service account, then a data message.
  const rsa = await crypto.subtle.generateKey({ name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["sign", "verify"]);
  const account = { project_id: "the-project", client_email: "push@the-project.iam.gserviceaccount.com", private_key: pemOf(await crypto.subtle.exportKey("pkcs8", rsa.privateKey), "PRIVATE KEY") };
  equal(await fcm(fetch, account, { address: "fcm-token" }, { note }), { sent: true }, "FCM takes a note");
  const [token, send] = asked.splice(0);
  equal(token.url, "https://oauth2.googleapis.com/token", "Google's token endpoint");
  const form = new URLSearchParams(token.body);
  equal(form.get("grant_type"), "urn:ietf:params:oauth:grant-type:jwt-bearer", "a JWT bearer grant");
  const [h, c] = jwtParts(form.get("assertion"));
  equal([h.alg, c.iss, c.scope, c.aud], ["RS256", account.client_email, "https://www.googleapis.com/auth/firebase.messaging", "https://oauth2.googleapis.com/token"], "the service account's assertion");
  equal(send.url, "https://fcm.googleapis.com/v1/projects/the-project/messages:send", "FCM's URL");
  equal(send.headers.authorization, "Bearer a token", "FCM's token");
  const message = JSON.parse(send.body).message;
  equal([message.token, b64u.decode(message.data.n).join(), message.android.priority, message.notification], ["fcm-token", note.join(), "HIGH", undefined], "a high-priority data message, nothing for FCM to show");

  // WNS: an Entra token for the app's registration, then a raw notification to the channel.
  const channel = "https://wns2-par02p.notify.windows.com/w/?token=abc";
  const app = { tenant: "the-tenant", client_id: "the-client", client_secret: "the-secret" };
  equal(await wns(fetch, app, { address: channel }, { note }), { sent: true }, "WNS takes a note");
  const [wnsToken, raw] = asked.splice(0);
  equal(wnsToken.url, "https://login.microsoftonline.com/the-tenant/oauth2/v2.0/token", "Entra's token endpoint");
  const wnsForm = new URLSearchParams(wnsToken.body);
  equal([wnsForm.get("grant_type"), wnsForm.get("client_id"), wnsForm.get("scope")], ["client_credentials", "the-client", "https://wns.windows.com/.default"], "client credentials for WNS");
  equal([raw.url, raw.headers["x-wns-type"], raw.headers["content-type"], raw.headers.authorization, new Uint8Array(raw.body).join()], [channel, "wns/raw", "application/octet-stream", "Bearer a token", note.join()], "a raw notification of the note's bytes");
  equal(await wns(fetch, app, { address: "https://evil.example/w" }, { note }), { gone: true }, "a channel anywhere but WNS");
  asked.length = 0;

  // Web Push: RFC 8291's example, byte for byte.
  const d = (s) => b64u.decode(s);
  const asPublic = d("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8");
  const jwkOf = (pub, priv) => ({ kty: "EC", crv: "P-256", x: b64u.encode(pub.slice(1, 33)), y: b64u.encode(pub.slice(33)), d: priv });
  const ephemeral = {
    privateKey: await crypto.subtle.importKey("jwk", jwkOf(asPublic, "yfWPiYE-n46HLnH0KqZOF1fJJU3MYrct3AELtAQ-oRw"), { name: "ECDH", namedCurve: "P-256" }, true, ["deriveBits"]),
    publicKey: await crypto.subtle.importKey("raw", asPublic, { name: "ECDH", namedCurve: "P-256" }, true, []),
  };
  const ua = d("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4");
  const body = await encrypt(d("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24"), ua, d("BTBZMqHH6r4Tts7J_aSIgg"), { ephemeral, salt: d("DGv6ra1nlYgDCS1FRnbzlw") });
  equal(b64u.encode(body), "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN", "RFC 8291's example");

  // And its request: VAPID for the push service's origin, aes128gcm, only to a browser's push service.
  const vapid = await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, ["sign", "verify"]);
  const vapidPublic = new Uint8Array(await crypto.subtle.exportKey("raw", vapid.publicKey));
  const sub = { address: "https://fcm.googleapis.com/fcm/send/abc", keys: { p256dh: b64u.encode(ua), auth: "BTBZMqHH6r4Tts7J_aSIgg" } };
  equal(await webPush(fetch, { key: vapid.privateKey, public: vapidPublic, subject: "https://one.example" }, sub, { note }), { sent: true }, "a browser's push service takes a note");
  const w = asked.pop();
  equal([w.url, w.headers["content-encoding"], w.headers.urgency, w.body.length], [sub.address, "aes128gcm", "high", 103 + b64u.encode(note).length], "Web Push's request, the note as text");
  const [, t, k] = /^vapid t=([^,]+), k=(.+)$/.exec(w.headers.authorization);
  equal([jwtParts(t)[1].aud, jwtParts(t)[1].sub, k], ["https://fcm.googleapis.com", "https://one.example", b64u.encode(vapidPublic)], "VAPID's token and key");
  equal(await webPush(fetch, { key: vapid.privateKey, public: vapidPublic, subject: "x" }, { ...sub, address: "https://evil.example/push" }, { note }), { gone: true }, "an endpoint that isn't a browser's push service");
}
