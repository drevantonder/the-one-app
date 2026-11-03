// The auth service: who you are, and nothing more. Better Auth, in a Durable Object: sign-in by passkey, or, on an
// install whose people sign in with their work Google account (`SIGN_IN`), by Google at the install's domain
// (`JOIN_DOMAIN`) too, though never for an admin (`passkey_only`), who signs in with a passkey; and an OAuth server (per
// MCP's authorization spec) that every client signs in through: the iPhone and Mac apps, and MCP clients like Claude
// Code and ChatGPT. Its access tokens are JWTs signed with Ed25519, its keys at /api/auth/jwks, so nodes check them
// offline. It opens nothing: keys are only on people's devices, so faking a sign-in opens nothing either. A passkey's
// PRF output stays in the browser (pages.js).
//
// Nobody signs up uninvited, the install's first person too: an admin's device makes a secret, and tells auth its hash,
// who it's for and until when, signed with its endpoint key (`POST /api/auth/invites`); opening `<address>/invite#
// <secret>` lets that person sign up once, as the id and name the invite gives them. At a workplace too: signing in
// with Google makes someone only with an invite for that Google account's email.
//
// A device signed in leaves a request to join here (`/api/auth/devices/requests`): its endpoint keys, which a device of
// its person (or, for a newcomer's first, the inviting admin's) finds by its join code and approves. A service (`one
// sidecars serve`), which nobody signs in on, leaves one signed with its own key instead, naming the service: whoever
// types its code on one of their devices makes it theirs.
//
// What an endpoint signs (`signedRequest` in shared/crypto.js, domain `one/auth`) names its route (`route: "POST
// /api/auth/invites"`), is taken once, and counts only from one of the install's endpoints, as the relay's list says
// (`RELAY.allowed(key)`), an admin's for what only admins do. A POST's body is the signed request; a GET's or DELETE's
// is in `x-one-request`, as base64url of its JSON.
//
// A try install's kernel (`SIGN_IN` is "try") signs in its first person (`TRY_USER`, `TRY_NAME`) with no passkey and
// no Google: its sign-in page goes straight on. Someone it invites joins by opening their link, again with no passkey,
// and is then who that browser signs in as. It's for testing clients without a person: only `one try` turns it on
// (kernel::Settings::try_person), on a kernel that listens only on its own machine, and `kernel deploy` refuses one,
// so no real install's kernel ever does this.

import { DurableObject } from "cloudflare:workers";
import { oauthProvider } from "@better-auth/oauth-provider";
import { getAuthenticatorName, passkey } from "@better-auth/passkey";
import { betterAuth } from "better-auth";
import { APIError, createAuthEndpoint, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { setSessionCookie } from "better-auth/cookies";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { b64u, checkRequest } from "../shared/crypto.js";
import { bearer, verify } from "../shared/token.js";
import { codePage, invitePage, passkeysPage, signInPage, tryPage } from "./pages.js";
import { DurableObjectDialect } from "./sql.js";

const json = (v, status = 200) => Response.json(v, { status });
const hex16 = () => [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
/// Where a client's sign-in goes, in words: the site it goes back to, or an app on the person's device (one listening on
/// it, or its own link scheme), or the device that showed them a code.
function goesTo(uri, origin) {
  let url;
  try { url = new URL(uri); } catch { return ON_DEVICE; }
  if (url.protocol !== "https:" && url.protocol !== "http:") return ON_DEVICE;
  if (url.origin === origin && url.pathname === "/sign-in/code/done") return BY_CODE;
  if (["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) return ON_DEVICE;
  return url.host;
}
const [ON_DEVICE, BY_CODE] = ["an app on this device", "where you got your code"];

/// Refuses, as Better Auth does, since some refusals are made inside its routes.
const refuse = (status, message) => {
  throw new APIError({ 400: "BAD_REQUEST", 401: "UNAUTHORIZED", 403: "FORBIDDEN", 404: "NOT_FOUND", 409: "CONFLICT", 503: "SERVICE_UNAVAILABLE" }[status], { message });
};
/// The longest an invite works.
const INVITE = 30 * 86_400_000;
/// How long a device's request to join waits.
const REQUEST = 15 * 60_000;
// How many services' requests may wait at once: anyone may leave one.
const SERVICES_WAITING = 16;
/// How long a sign-in code works.
const CODE = 10 * 60_000;
/// How lately someone signed in for a client to be added as them, or a passkey added.
const FRESH = 5 * 60_000;
/// How long a Google sign-in started for a client's authorization, or to join, goes on to it once it's back.
const STARTED = 15 * 60_000;
/// A sign-in code's letters: no vowels, so it spells no word, and none that look alike.
const LETTERS = "BCDFGHJKLMNPQRSTVWXZ";
/// What endpoints sign for auth.
const DOMAIN = "one/auth";
/// Where Better Auth makes a session with a passkey: the only places an admin's is made.
const BY_PASSKEY = ["/passkey/verify-authentication", "/passkey/verify-registration"];
const KEY = /^[0-9a-f]{64}$/;
const USER = /^[0-9a-f]{16}$/;

/// A SHA-256 hash, as hex, of a secret kept only that way.
async function hash(secret) {
  return [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(secret)))].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/// A passkey's name, for the person's list of them: its maker's, if it says, a security key's, or the device it was
/// made on.
function passkeyName(aaguid, transports = [], agent = "") {
  const known = getAuthenticatorName(aaguid);
  if (known) return known;
  if (transports.length && transports.every((t) => ["usb", "nfc", "ble"].includes(t))) return "Security key";
  const device = [["iPhone", "iPhone"], ["iPad", "iPad"], ["Android", "Android"], ["Macintosh", "Mac"], ["Windows", "Windows"], ["CrOS", "Chromebook"], ["Linux", "Linux"]].find(([k]) => agent.includes(k));
  return device ? `Passkey on ${device[1]}` : "Passkey";
}

/// A cookie this address set, by name: `__Host-` first on https.
function cookieName(env, name) {
  return `${env.ADDRESS.startsWith("https:") ? "__Host-" : ""}${name}`;
}

function cookieOf(headers, name) {
  const prefix = `${name}=`;
  return (headers?.get("cookie") || "").split(/;\s*/).find((c) => c.startsWith(prefix))?.slice(prefix.length) || null;
}

function setCookie(env, name, value, seconds) {
  const secure = env.ADDRESS.startsWith("https:");
  return `${cookieName(env, name)}=${value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${seconds}${secure ? "; Secure" : ""}`;
}

export class Auth extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    const sql = ctx.storage.sql;
    // Invites, by their secret's hash, and `by`, the endpoint that made one; kept once used, for the requests of the
    // device its person joined with.
    sql.exec("create table if not exists invites (id text primary key, name text not null, user text not null unique, email text, until integer not null, by text not null, used integer)");
    // Devices' requests to join, by their key: `invite` is the invite of a newcomer's first device, with its `proof`.
    sql.exec("create table if not exists requests (key text primary key, seal text not null, name text not null, user text not null, proof text, invite text, made integer not null)");
    // Services' requests to join, by their key, signed by it: nobody's until one of the install's devices approves it.
    sql.exec("create table if not exists services (key text primary key, seal text not null, name text not null, service text not null, made integer not null)");
    // The people who sign in only with a passkey: admins.
    sql.exec("create table if not exists passkey_only (user text primary key)");
    // Signed requests taken, until they're too old to take again.
    sql.exec("create table if not exists signed (sig text primary key, at integer not null)");
    sql.exec("create table if not exists codes (code text primary key, state text not null unique, query text not null, until integer not null, back text)");
    sql.exec("create table if not exists google_starts (id text primary key, query text not null, made integer not null)");
    this.ready = ctx.blockConcurrencyWhile(async () => {
      const options = this.options();
      const { runMigrations } = await getMigrations(options);
      await runMigrations();
      this.auth = betterAuth(options);
    });
  }

  options() {
    const env = this.env;
    const url = new URL(env.ADDRESS);
    // Each address, and its MCP endpoint, which is how MCP names a server.
    const resources = JSON.parse(env.RESOURCES || "[]").flatMap((r) => [r, `${r}/mcp`]);
    const sql = this.ctx.storage.sql;
    const self = this;
    const google = env.SIGN_IN === "google";
    const tried = env.SIGN_IN === "try";
    return {
      appName: env.NAME || "The One App",
      baseURL: url.origin,
      basePath: "/api/auth",
      secret: env.AUTH_SECRET,
      database: { dialect: new DurableObjectDialect(sql), type: "sqlite", transaction: false },
      trustedOrigins: [url.origin],
      telemetry: { enabled: false },
      advanced: { database: { generateId: hex16 } },
      socialProviders: google ? { google: { clientId: env.GOOGLE_CLIENT_ID, clientSecret: env.GOOGLE_CLIENT_SECRET, hd: env.JOIN_DOMAIN, prompt: "select_account" } } : {},
      databaseHooks: {
        user: {
          create: {
            // Only someone invited signs up, as who the invite says they are in the install, so devices know them by
            // the same id: with a passkey, as it's registered, or a try install's link (`admit`); with Google, as
            // Google says the email is theirs, with the invite this browser opened to join.
            async before(user, ctx) {
              if (self.admitting && user.id === self.admitting) return;
              if (tried) return user.id === env.TRY_USER ? undefined : false;
              if (!google) return false;
              const secret = cookieOf(ctx?.headers, cookieName(env, "one-invite"));
              const invite = secret && sql.exec("select * from invites where id = ?", await hash(secret)).toArray()[0];
              const email = String(user.email || "").toLowerCase();
              if (!invite || invite.used || invite.until <= Date.now() || !user.emailVerified || invite.email !== email) return false;
              sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
              return { data: { ...user, id: invite.user, name: invite.name } };
            },
          },
        },
        session: {
          create: {
            // An admin signs in only with a passkey, which resists phishing.
            async before(session, ctx) {
              if (tried || BY_PASSKEY.includes(ctx?.path)) return;
              if (sql.exec("select 1 from passkey_only where user = ?", session.userId).toArray().length) refuse(403, "You're an admin, so you sign in with your passkey.");
            },
          },
        },
      },
      hooks: {
        before: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/authorize" && ctx.query) {
            if (!ctx.query.resource && resources[0]) ctx.query.resource = resources[0];
            // Every client gets a refresh token, asked for or not, so it stays signed in.
            const scopes = String(ctx.query.scope || "").split(" ").filter(Boolean);
            if (!scopes.includes("offline_access")) ctx.query.scope = [...scopes, "offline_access"].join(" ");
          }
          if (ctx.path === "/oauth2/token" && ctx.body && !ctx.body.resource && resources[0]) ctx.body.resource = resources[0];
        }),
      },
      plugins: [
        jwt({ jwks: { keyPairConfig: { alg: "EdDSA", crv: "Ed25519" } }, jwt: { issuer: `${url.origin}/api/auth`, expirationTime: "1h" } }),
        ...(tried ? [tryInstall(self, env.TRY_USER, env.TRY_NAME)] : [passkey({
          rpID: url.hostname,
          rpName: env.NAME || "The One App",
          origin: url.origin,
          registration: {
            // Someone signed in adds a passkey; someone with an invite link (its secret as the context) makes their first.
            requireSession: false,
            async resolveUser({ context }) {
              const invite = await self.openInvite(context);
              return { id: invite.user, name: invite.name, displayName: invite.name };
            },
            // Someone signed in adds one only just after signing in, so a session taken from them adds none.
            async afterVerification({ ctx, verification, user, clientData, context }) {
              const session = await getSessionFromCtx(ctx);
              if (session && !fresh(session)) refuse(403, "Sign in again, then add it.");
              if (!session) await self.admit(await self.openInvite(context), user);
              return { name: passkeyName(verification.registrationInfo?.aaguid, clientData.response?.transports || [], ctx.headers?.get("user-agent") || "") };
            },
          },
        })]),
        oauthProvider({
          // Signed in or not, a client's authorization is the sign-in page, which asks for the passkey (or Google)
          // each time.
          loginPage: "/sign-in",
          consentPage: "/sign-in",
          scopes: ["openid", "profile", "offline_access"],
          allowDynamicClientRegistration: true,
          allowUnauthenticatedClientRegistration: true,
          // Claude Code and other local MCP clients register http://localhost redirects without saying they're native.
          clientRegistrationDefaultApplicationType: "infer",
          resources,
          enforcePerClientResources: false,
          accessTokenExpiresIn: 3600,
          refreshTokenReuseInterval: 0,
          storeTokens: "hashed",
          // Who a token is for, by name, so a node lets someone new in as who they are.
          customAccessTokenClaims: ({ user }) => (user ? { name: user.name } : {}),
        }),
      ],
    };
  }

  async fetch(request) {
    await this.ready;
    const url = new URL(request.url);
    const path = url.pathname;
    try {
      const ours = await this.route(request, url);
      if (ours) return ours;
      if (path === "/api/auth/oauth2/register" && request.method === "POST") await this.notNamedAsUs(await request.clone().json().catch(() => ({})));
      if (path.startsWith("/api/auth/")) return await this.auth.handler(request);
      if (path.startsWith("/.well-known/oauth-authorization-server") || path.startsWith("/.well-known/openid-configuration")) {
        const which = path.includes("openid") ? "/.well-known/openid-configuration" : "/.well-known/oauth-authorization-server";
        const meta = await this.auth.handler(new Request(`${url.origin}/api/auth${which}`, { headers: request.headers }));
        return new Response(meta.body, { status: meta.status, headers: { "content-type": "application/json", "access-control-allow-origin": "*" } });
      }
      const name = this.auth.options.appName;
      // What its pages say: the white label's name, and what a person joins ("your One App").
      const look = { name, yours: this.env.YOURS };
      const google = this.env.SIGN_IN === "google" ? this.env.JOIN_DOMAIN : null;
      const tried = this.env.SIGN_IN === "try";
      if (path === "/sign-in" && request.method === "GET") return html(tried ? tryPage({ ...look, person: this.env.TRY_NAME }) : signInPage({ ...look, google }));
      if (path === "/sign-in/about") return json(await this.about(request, url.searchParams));
      if (path === "/sign-in/google" && request.method === "POST" && google) return await this.startGoogle(request, await request.json());
      if (path === "/sign-in/code" && request.method === "POST") return json(await this.codeFor(await request.json()));
      if (path === "/sign-in/code" && request.method === "GET") return this.codeOpened(url.searchParams.get("c"), look, google);
      if (path === "/sign-in/code/done" && request.method === "GET") return html(codePage({ ...look, google, done: await this.codeDone(url.searchParams) }));
      if (path === "/sign-in/code/take" && request.method === "POST") return this.codeTaken(await request.json());
      if (path === "/sign-in/accept" && request.method === "POST") return json(await this.accept(request, await request.json()));
      if (path === "/invite" && request.method === "GET") return html(invitePage({ ...look, tried, google }));
      if (path === "/invite/about" && request.method === "POST") return json(await this.inviteAbout(await request.json()));
      if (path === "/invite/google" && request.method === "POST" && google) return await this.joinWithGoogle(request, await request.json());
      if (path === "/passkeys" && request.method === "GET" && !tried) return html(passkeysPage(look), { framed: true });
      return new Response("not found", { status: 404 });
    } catch (err) {
      // Our own refusals have a numeric status; Better Auth's say theirs in statusCode.
      const status = typeof err.status === "number" ? err.status : err.statusCode;
      if (status >= 400 && status < 500 || status === 503) return json({ error: err.body?.error_description || err.body?.message || err.message }, status);
      console.error(err.stack || err);
      return json({ error: "the auth service failed" }, 500);
    }
  }

  /// The routes of auth's own under /api/auth/: invites, admins and devices' requests to join. → their answer, or
  /// nothing for any other path.
  async route(request, url) {
    const path = url.pathname;
    const method = request.method;
    if (path === "/api/auth/invites" && method === "POST") return json(await this.invite(request));
    if (path === "/api/auth/people/admin" && method === "POST") return json(await this.setAdmin(request));
    if (path === "/api/auth/devices/requests" && method === "POST") return json(await this.ask(request));
    if (path === "/api/auth/devices/requests" && method === "GET") return json(await this.requests(request));
    const one = path.match(/^\/api\/auth\/devices\/requests\/([0-9a-f]{64})$/);
    if (one && method === "GET") return json(await this.request(request, one[1]));
    if (one && method === "DELETE") return json(await this.unask(request, one[1]));
    return null;
  }

  /// What an endpoint signed for this request's route, if it brought one (its body, or `x-one-request`), checked:
  /// fresh, taken once, and from one of the install's endpoints. → `{ by, fields, admin }`, or null.
  async signed(request, body) {
    const route = `${request.method} ${new URL(request.url).pathname}`;
    let asked = body;
    if (asked === undefined) {
      const header = request.headers.get("x-one-request");
      if (!header) return null;
      try { asked = JSON.parse(new TextDecoder().decode(b64u.decode(header))); } catch { refuse(400, "x-one-request isn't a signed request"); }
    }
    const { by, fields } = await checkRequest(DOMAIN, asked).catch((err) => refuse(403, `That request isn't signed as it should be: ${err.message}.`));
    if (fields.route !== route) refuse(403, `That request was signed for ${fields.route}, not ${route}.`);
    const sql = this.ctx.storage.sql;
    sql.exec("delete from signed where at <= ?", Date.now() - 2 * 60_000);
    this.taken(asked.sig);
    if (!this.env.RELAY) refuse(503, "The install has no relay to say which devices are its own.");
    const allowed = await this.env.RELAY.allowed(by);
    if (!allowed) refuse(403, "That device isn't one of the install's.");
    return { by, fields, admin: allowed.admin === true };
  }

  /// Takes a signed request's signature once: one sent again is refused.
  taken(sig) {
    const sql = this.ctx.storage.sql;
    sql.exec("delete from signed where at <= ?", Date.now() - 2 * 60_000);
    if (sql.exec("select 1 from signed where sig = ?", sig).toArray().length) refuse(403, "That request was sent already.");
    sql.exec("insert into signed (sig, at) values (?, ?)", sig, Date.now());
  }

  /// Someone signed in: by a session here, or an access token auth signed. → their user id, or null.
  async person(request) {
    const token = bearer(request);
    if (token) {
      const origin = new URL(this.env.ADDRESS).origin;
      const jwks = await (await this.auth.handler(new Request(`${origin}/api/auth/jwks`))).json();
      const claims = await verify(token, jwks, `${origin}/api/auth`).catch((err) => refuse(401, `That access token won't do: ${err.message}.`));
      return claims.sub;
    }
    return (await this.auth.api.getSession({ headers: request.headers }))?.user.id || null;
  }

  /// An invite, made by an admin's device: `{ id, name, user, email?, until }`, `id` the sha256 (hex) of the secret
  /// its link carries, `user` the id they'll have. On an install whose people sign in with Google, `email` is the
  /// Google account they sign up with. Another invite for the same person, not yet used, replaces it.
  async invite(request) {
    const signed = await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")));
    if (!signed?.admin) refuse(403, "Only an admin's device invites someone.");
    const { id, name, user, email, until } = signed.fields;
    if (!KEY.test(id || "")) refuse(400, "an invite's id is the sha256 of its secret, in hex");
    if (typeof name !== "string" || !name.trim()) refuse(400, "give the person's name");
    if (!USER.test(user || "")) refuse(400, "a user id is 16 hex characters");
    if (!(Number.isSafeInteger(until) && until > Date.now() && until <= Date.now() + INVITE)) refuse(400, "an invite works for up to 30 days");
    if (email != null && (typeof email !== "string" || !/^[^@\s]+@[^@\s]+$/.test(email))) refuse(400, `${email} isn't an email`);
    if (this.env.SIGN_IN === "google" && !String(email || "").toLowerCase().endsWith(`@${this.env.JOIN_DOMAIN}`)) refuse(400, `People here sign in with Google: give their email at ${this.env.JOIN_DOMAIN}.`);
    const sql = this.ctx.storage.sql;
    if (sql.exec("select 1 from user where id = ?", user).toArray().length) refuse(409, `${name.trim()} has joined already.`);
    sql.exec("delete from invites where until <= ? and used is null", Date.now());
    sql.exec("insert or replace into invites (id, name, user, email, until, by, used) values (?, ?, ?, ?, ?, ?, null)", id, name.trim(), user, email ? email.toLowerCase() : null, until, signed.by);
    return { until };
  }

  /// Whether someone signs in only with a passkey, as an admin does: `{ user, admin }`, from an admin's device, as it
  /// writes their role. Someone with no passkey yet isn't made one, or they couldn't sign in.
  async setAdmin(request) {
    const signed = await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")));
    if (!signed?.admin) refuse(403, "Only an admin's device says who's an admin.");
    const { user, admin } = signed.fields;
    if (!USER.test(user || "") || typeof admin !== "boolean") refuse(400, "give user, and admin as true or false");
    const sql = this.ctx.storage.sql;
    if (admin && this.env.SIGN_IN !== "try" && !sql.exec("select 1 from passkey where userId = ?", user).toArray().length) refuse(409, "They have no passkey yet, which an admin signs in with: they add one on their Passkeys page first.");
    if (admin) sql.exec("insert or ignore into passkey_only (user) values (?)", user);
    else sql.exec("delete from passkey_only where user = ?", user);
    return {};
  }

  /// Forgets requests to join that waited too long.
  expire() {
    this.ctx.storage.sql.exec("delete from requests where made <= ?", Date.now() - REQUEST);
    this.ctx.storage.sql.exec("delete from services where made <= ?", Date.now() - REQUEST);
  }

  /// A device signed in asks to join: `{ key, seal, name, proof? }`, its endpoint keys (hex), its name, and for a
  /// newcomer's first device, `keys::invite_proof` of its invite's secret, key and seal key, kept with that invite for the
  /// inviting admin's device to check. → until when it waits.
  async ask(request) {
    const body = await request.json().catch(() => refuse(400, "the body isn't JSON"));
    if (body?.service != null) return this.askAsService(request, body);
    const user = (await this.person(request)) || refuse(401, "Sign in first.");
    const { key, seal, name, proof } = body;
    if (!KEY.test(key || "") || !KEY.test(seal || "")) refuse(400, "key and seal are endpoint keys, 64 hex characters each");
    if (typeof name !== "string" || !name.trim() || name.length > 64) refuse(400, "give the device's name, up to 64 characters");
    if (proof != null && !KEY.test(proof)) refuse(400, "a proof is 64 hex characters");
    this.expire();
    const sql = this.ctx.storage.sql;
    const [had] = sql.exec("select user from requests where key = ?", key).toArray();
    if (had && had.user !== user) refuse(409, "Another person's device asked with that key.");
    let invite = null;
    if (proof != null) {
      [invite] = sql.exec("select id from invites where user = ? and used is not null", user).toArray();
      if (!invite) refuse(400, "Only a newcomer's first device brings an invite's proof.");
    }
    const made = Date.now();
    sql.exec("insert or replace into requests (key, seal, name, user, proof, invite, made) values (?, ?, ?, ?, ?, ?, ?)", key, seal, name.trim(), user, proof ?? null, invite?.id ?? null, made);
    return { until: made + REQUEST };
  }

  /// A service asks to join, signed with its own key: `{ key, seal, name, service }`, `name` its machine's. A few wait
  /// at once, since anyone may ask. → until when it waits.
  async askAsService(request, body) {
    const { by, fields } = await checkRequest(DOMAIN, body).catch((err) => refuse(403, `That request isn't signed as it should be: ${err.message}.`));
    if (fields.route !== `${request.method} ${new URL(request.url).pathname}`) refuse(403, `That request was signed for ${fields.route}.`);
    const { key, seal, name, service } = fields;
    if (key !== by) refuse(403, "A service asks with the key it signs with.");
    if (!KEY.test(seal || "")) refuse(400, "seal is an endpoint key, 64 hex characters");
    if (typeof name !== "string" || !name.trim() || name.length > 64) refuse(400, "give the machine's name, up to 64 characters");
    if (typeof service !== "string" || !/^[a-z0-9]+(-[a-z0-9]+)*$/.test(service) || service.length > 64) refuse(400, "a service's name is lowercase words joined by -");
    this.taken(body.sig);
    this.expire();
    const sql = this.ctx.storage.sql;
    const waiting = sql.exec("select count(*) as n from services where key != ?", key).toArray()[0].n;
    if (waiting >= SERVICES_WAITING) refuse(429, "Too many services are waiting to be approved: try again in a few minutes.");
    const made = Date.now();
    sql.exec("insert or replace into services (key, seal, name, service, made) values (?, ?, ?, ?, ?)", key, seal, name.trim(), service, made);
    return { until: made + REQUEST };
  }

  /// Requests to join, as they're shown: with the person's name.
  shown(rows) {
    const sql = this.ctx.storage.sql;
    return rows.map((r) => {
      const person = sql.exec("select name from user where id = ?", r.user).toArray()[0]?.name || "";
      return { key: r.key, seal: r.seal, name: r.name, user: r.user, person, ...(r.proof ? { proof: r.proof } : {}), made: r.made, until: r.made + REQUEST };
    });
  }

  /// Requests waiting: the signed-in person's own devices', or, for a signed request from one of the install's
  /// devices, every one: a device approves only its own person's, or a newcomer's whose invite its person keeps, as
  /// the entries it writes count only then.
  async requests(request) {
    this.expire();
    const sql = this.ctx.storage.sql;
    const signed = await this.signed(request);
    if (signed) {
      const services = sql.exec("select * from services order by made").toArray().map((r) => ({ key: r.key, seal: r.seal, name: r.name, user: "", person: "", service: r.service, made: r.made, until: r.made + REQUEST }));
      return { requests: [...this.shown(sql.exec("select * from requests order by made").toArray()), ...services] };
    }
    const user = (await this.person(request)) || refuse(401, "Sign in first.");
    return { requests: this.shown(sql.exec("select * from requests where user = ? order by made", user).toArray()) };
  }

  /// A request by its device's key, if the asker may see it: its person, or one of the install's devices.
  async mayTouch(request, key) {
    this.expire();
    const sql = this.ctx.storage.sql;
    const signed = await this.signed(request);
    const [row] = sql.exec("select * from requests where key = ?", key).toArray();
    if (!row && signed) {
      const [service] = sql.exec("select * from services where key = ?", key).toArray();
      if (service) return { key: service.key, seal: service.seal, name: service.name, user: "", service: service.service, made: service.made };
    }
    const ok = row && (signed || row.user === ((await this.person(request)) || refuse(401, "Sign in first.")));
    if (!ok) refuse(404, "No request waits with that key: it was approved, refused or waited too long.");
    return row;
  }

  /// One request, for its new device to see it still waits.
  async request(request, key) {
    const row = await this.mayTouch(request, key);
    return row.service ? { ...row, person: "", until: row.made + REQUEST } : this.shown([row])[0];
  }

  /// Forgets a request, approved or refused.
  async unask(request, key) {
    await this.mayTouch(request, key);
    this.ctx.storage.sql.exec("delete from requests where key = ?", key);
    this.ctx.storage.sql.exec("delete from services where key = ?", key);
    return {};
  }

  async session(request) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    if (!session) throw Object.assign(new Error("sign in first"), { status: 401 });
    return session;
  }

  async client(clientId) {
    const client = clientId && (await (await this.auth.$context).adapter.findOne({ model: "oauthClient", where: [{ field: "clientId", value: clientId }] }));
    if (!client) throw Object.assign(new Error("no such client"), { status: 404 });
    return client;
  }

  /// A client names itself, so none may take the install's name, as signing in to it would read as signing in here;
  /// nor its apps' ("<install> on <device>"), unless it comes back to the device, as they do.
  notNamedAsUs(body) {
    const plain = (s) => String(s || "").toLowerCase().replace(/[^\p{L}\p{N}]/gu, "").replace(/^the/, "");
    const [name, ours] = [plain(body.client_name), plain(this.auth.options.appName)];
    const origin = new URL(this.env.ADDRESS).origin;
    const toSites = [].concat(body.redirect_uris || []).some((u) => ![ON_DEVICE, BY_CODE].includes(goesTo(u, origin)));
    if (name === ours || (name.startsWith(ours) && toSites)) refuse(400, `a client can't be called ${body.client_name}: that's the name of what it signs in to`);
  }

  /// What signing in to a client (its authorization's `query`) says: its name (the install's, for the web client, which
  /// is at its address), where its sign-in goes (its redirect_uri, if it's one of the client's, else its first), since
  /// the name is only what it calls itself, the person's, if they're signed in here already, and whether they're back
  /// from the Google sign-in this browser started for it, which goes straight on.
  async about(request, query) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    const client = await this.client(query.get("client_id"));
    const redirect = query.get("redirect_uri");
    const origin = new URL(this.env.ADDRESS).origin;
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const web = uris.length > 0 && uris.every((u) => u === `${origin}/callback`);
    const goes = web ? null : goesTo(uris.includes(redirect) ? redirect : uris[0], origin);
    return { client: web ? this.auth.options.appName : client.name || client.clientId, web, goes, person: session?.user.name || null, fresh: !!(await this.started(request, session, query)) };
  }

  // Signing in to a client with Google goes straight on once Google says yes, with no second tap, but only for the
  // authorization this browser started it for: a sign-in with Google made for another client (the web client's, a
  // minute ago) gives nothing to a client whose link the person was sent. Starting one keeps a secret in a cookie only
  // this address sets, with a hash of the authorization's query; coming back, the two must match, once.

  /// Starts signing in with Google for a client's authorization (`oauth_query`), on the sign-in page.
  async startGoogle(request, { oauth_query }) {
    this.fromOurPages(request);
    const query = new URLSearchParams(typeof oauth_query === "string" ? oauth_query : "");
    await this.client(query.get("client_id"));
    const secret = b64u.encode(crypto.getRandomValues(new Uint8Array(32)));
    const sql = this.ctx.storage.sql;
    sql.exec("delete from google_starts where made <= ?", Date.now() - STARTED);
    sql.exec("insert into google_starts (id, query, made) values (?, ?, ?)", await hash(secret), await hash(query.toString()), Date.now());
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": setCookie(this.env, "one-google-start", secret, STARTED / 1000) } });
  }

  /// Refuses what only this address's own pages ask.
  fromOurPages(request) {
    if (request.headers.get("origin") !== new URL(this.env.ADDRESS).origin) refuse(403, "only the sign-in page asks that");
  }

  /// The Google sign-in this browser started for the authorization `query`, if it's back from it just now: → its id.
  async started(request, session, query) {
    if (!session || !fresh(session)) return null;
    const secret = cookieOf(request.headers, cookieName(this.env, "one-google-start"));
    if (!secret) return null;
    const id = await hash(secret);
    const [row] = this.ctx.storage.sql.exec("select query, made from google_starts where id = ? and made > ?", id, Date.now() - STARTED).toArray();
    const signedIn = new Date(session.session.createdAt).getTime();
    return row && row.query === (await hash(query.toString())) && row.made <= signedIn ? id : null;
  }

  // Signing in with a code, for a client with no browser of its own (the CLI on a device with no screen): it makes an authorization
  // request coming back to /sign-in/code/done and gets a short code for it; the person opens /sign-in/code on any
  // device, types the code (or follows its link), and signs in there with their passkey; the client takes the
  // authorization code by its state and turns it into tokens with its PKCE verifier, which only it has.

  /// A code for an authorization request: `{ query }`, the request's query, coming back here. → `{ code, url, until }`.
  async codeFor({ query }) {
    const q = new URLSearchParams(typeof query === "string" ? query : "");
    const origin = new URL(this.env.ADDRESS).origin;
    if (q.get("redirect_uri") !== `${origin}/sign-in/code/done`) refuse(400, `a sign-in code's request comes back to ${origin}/sign-in/code/done`);
    if ((q.get("state") || "").length < 32 || !q.get("code_challenge")) refuse(400, "a sign-in code's request has a state of 32 characters or more, and PKCE");
    await this.client(q.get("client_id"));
    const sql = this.ctx.storage.sql;
    sql.exec("delete from codes where until <= ?", Date.now());
    const until = Date.now() + CODE;
    for (;;) {
      const code = [...crypto.getRandomValues(new Uint8Array(8))].map((b) => LETTERS[b % LETTERS.length]).join("");
      if (sql.exec("select code from codes where code = ?", code).toArray().length) continue;
      sql.exec("insert into codes (code, state, query, until) values (?, ?, ?, ?)", code, await hash(q.get("state")), q.toString(), until);
      return { code: `${code.slice(0, 4)}-${code.slice(4)}`, url: `${origin}/sign-in/code?c=${code}`, until };
    }
  }

  /// The page a code is typed into, or with one (`c`), on to its sign-in.
  codeOpened(typed, look, google) {
    if (typed == null) return html(codePage({ ...look, google }));
    const code = String(typed).toUpperCase().replace(/[^A-Z]/g, "");
    const [row] = this.ctx.storage.sql.exec("select query from codes where code = ? and until > ? and back is null", code, Date.now()).toArray();
    if (!row) return html(codePage({ ...look, google, error: "That code doesn't work: it may have expired. Start signing in again where you got it." }));
    return Response.redirect(`${new URL(this.env.ADDRESS).origin}/api/auth/oauth2/authorize?${row.query}`, 302);
  }

  /// Where a code's sign-in comes back: the authorization code (or why there's none) is kept for its client to take.
  /// → whether it's signed in.
  async codeDone(params) {
    const state = params.get("state");
    const back = params.get("code") ? JSON.stringify({ code: params.get("code") }) : JSON.stringify({ error: params.get("error_description") || params.get("error") || "Signing in didn't work." });
    if (!state) return false;
    const changed = this.ctx.storage.sql.exec("update codes set back = ? where state = ? and until > ? and back is null", back, await hash(state), Date.now()).rowsWritten;
    return changed > 0 && !!params.get("code");
  }

  /// A client takes what its code's sign-in gave, once: `{ state }` → `{ code }` or `{ error }`; 202 while it waits.
  async codeTaken({ state }) {
    const sql = this.ctx.storage.sql;
    const [row] = sql.exec("select code, back from codes where state = ? and until > ?", await hash(String(state || "")), Date.now()).toArray();
    if (!row) return json({ error: "That sign-in code has expired. Sign in again." }, 404);
    if (!row.back) return json({ waiting: true }, 202);
    sql.exec("delete from codes where code = ?", row.code);
    return json(JSON.parse(row.back));
  }

  /// The invite a link's secret opens, while it works.
  async openInvite(secret) {
    if (!secret) refuse(403, "Joining takes an invite link: ask an admin for one.");
    if (typeof secret !== "string" || secret.length < 32) refuse(404, "That invite link isn't whole: open the link you were sent as it is.");
    const [invite] = this.ctx.storage.sql.exec("select * from invites where id = ?", await hash(secret)).toArray();
    if (!invite) refuse(404, "That invite link doesn't work. Ask for a new one.");
    if (invite.used) refuse(404, "That invite link has been used. Ask for a new one.");
    if (invite.until <= Date.now()) refuse(404, "That invite link has expired. Ask for a new one.");
    return invite;
  }

  /// Makes the invite's person, as their first passkey is registered (or as a try install's link is opened), and
  /// spends the invite: the database hook lets only them in.
  async admit(invite, user) {
    if (user.id !== invite.user) refuse(400, "That passkey was made for someone else.");
    const context = await this.auth.$context;
    if (!(await context.internalAdapter.findUserById(invite.user))) {
      this.admitting = invite.user;
      try {
        await context.internalAdapter.createUser({ id: invite.user, name: invite.name, email: `${invite.user}@people.invalid`, emailVerified: false });
      } finally {
        this.admitting = null;
      }
    }
    this.ctx.storage.sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
  }

  /// What an invite page shows: whose it is, and on an install whose people sign in with Google, the email it's for.
  async inviteAbout({ secret }) {
    const invite = await this.openInvite(secret);
    return { person: invite.name, until: invite.until, email: invite.email };
  }

  /// Joining with Google: the invite's secret kept in a cookie for Google's answer, whose email must be the invite's.
  async joinWithGoogle(request, { secret }) {
    this.fromOurPages(request);
    await this.openInvite(secret);
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": setCookie(this.env, "one-invite", secret, STARTED / 1000) } });
  }

  /// Signing in to a client (`oauth_query`) as whoever just signed in here: by tapping their passkey on its page
  /// (`passkey`), or back from the Google sign-in started for it; on a try install, whoever is signed in. → where the
  /// client goes on to.
  async accept(request, { oauth_query, passkey }) {
    this.fromOurPages(request);
    const signedIn = await this.session(request);
    const query = new URLSearchParams(typeof oauth_query === "string" ? oauth_query : "");
    if (this.env.SIGN_IN !== "try") {
      const started = await this.started(request, signedIn, query);
      if (started) this.ctx.storage.sql.exec("delete from google_starts where id = ?", started);
      else if (!fresh(signedIn) || (this.env.SIGN_IN === "google" && passkey !== true)) refuse(403, "Sign in again.");
    }
    const client = await this.client(query.get("client_id"));
    const headers = new Headers(request.headers);
    headers.set("content-type", "application/json");
    headers.set("accept", "application/json");
    const resp = await this.auth.handler(new Request(new URL("/api/auth/oauth2/consent", request.url), { method: "POST", headers, body: JSON.stringify({ accept: true, oauth_query }) }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error_description || out.message || "signing in didn't work"), { status: resp.status });
    // Every authorization asks for the passkey (or Google): none is kept for next time.
    this.ctx.storage.sql.exec("delete from oauthConsent where clientId = ? and userId = ?", client.clientId, signedIn.user.id);
    return { url: out.url };
  }
}

/// A try install's sign-in, with no passkey: whoever is signed in here already (someone who joined in this browser),
/// or `person` by name (someone who joined), or else its first person, made the first time; and joining with an
/// invite link, which signs its person in here.
function tryInstall(auth, id, name) {
  const signIn = async (ctx, user) => {
    await setSessionCookie(ctx, { session: await ctx.context.internalAdapter.createSession(user.id), user });
    return ctx.json({ person: user.name });
  };
  const first = async (adapter) => (await adapter.findUserById(id)) || (await adapter.createUser({ id, name, email: `${id}@people.invalid`, emailVerified: false }));
  return {
    id: "try",
    endpoints: {
      signInToTry: createAuthEndpoint("/try/sign-in", { method: "POST" }, async (ctx) => {
        const adapter = ctx.context.internalAdapter;
        const person = typeof ctx.body?.person === "string" ? ctx.body.person.trim().toLowerCase() : "";
        if (person) {
          const [row] = auth.ctx.storage.sql.exec("select id from user where lower(name) = ?", person).toArray();
          if (!row && person !== name.toLowerCase()) refuse(404, `Nobody called ${ctx.body.person} has joined.`);
          return signIn(ctx, row ? await adapter.findUserById(row.id) : await first(adapter));
        }
        const already = await getSessionFromCtx(ctx);
        if (already) return ctx.json({ person: already.user.name });
        return signIn(ctx, await first(adapter));
      }),
      joinTry: createAuthEndpoint("/try/join", { method: "POST" }, async (ctx) => {
        const invite = await auth.openInvite(ctx.body?.secret);
        await auth.admit(invite, { id: invite.user });
        return signIn(ctx, await ctx.context.internalAdapter.findUserById(invite.user));
      }),
    },
  };
}

/// Whether a sign-in here was made just now: one a client is added with, or a passkey.
function fresh(session) {
  return Date.now() - new Date(session.session.createdAt).getTime() < FRESH;
}

/// A page, framed by nothing, or with `framed`, only by this site's own (the web client shows Passkeys beside its sidebar).
function html(text, { framed = false } = {}) {
  return new Response(text, { headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": `default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-ancestors ${framed ? "'self'" : "'none'"}` } });
}

export default {
  async fetch(request, env) {
    return env.AUTH.get(env.AUTH.idFromName("auth")).fetch(request);
  },
};
