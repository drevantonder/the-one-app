// Auth: who you are, and nothing more. Better Auth, in a Durable Object: signing in by passkey, or, on a
// network whose users sign in with their work Google account (`SIGN_IN`), by Google at the network's domain
// (`JOIN_DOMAIN`) too, though never for an admin (`passkey_only`), who signs in with a passkey; and an OAuth server (per
// MCP's authorization spec) that every client signs in through: the iPhone and Mac apps, and MCP clients like Claude
// Code and ChatGPT. Its access tokens are JWTs signed with Ed25519, its keys at /api/auth/jwks, so nodes check them
// offline. It opens nothing: keys are only on users' devices, so faking a sign-in opens nothing either. A passkey's
// PRF output stays in the browser (pages.js).
//
// Nobody signs up uninvited, the network's first user too: an admin's device makes a secret, and tells auth its hash,
// who it's for and until when, signed with its endpoint key (`POST /api/auth/invites`); opening `<address>/invite#
// <secret>` lets that user sign up once, as the id and name the invite gives them. At a workplace too: signing in
// with Google makes someone only with an invite for that Google account's email.
//
// A device signed in leaves a request to join here (`/api/auth/devices/requests`): its endpoint keys, which a device of
// its user (or, for a newcomer's first, the inviting admin's) finds by its join code and approves, then clears with
// `?approved`, which keeps it, marked, for the new device to see while it waits (it asks no node until it's in), with
// `&root=`, the network's root as the approving device holds it: the only network the new device then takes, so no
// node it reaches first can show it another; and once its who's who is written as steps, `&welcome=`,
// where the approving device read them to, which the new device starts from (src/database/steps.rs `Welcome`). Set
// once, with the approval. A service asks nothing here:
// its device's oned lists it.
//
// A sign-in for a client names the device it's for (`join` in its authorization's query: its key and seal key, 128
// hex), so a device of the user's can check it themselves (src/onboarding/sign_in_proof.rs) and let the device in with no one
// tapping: with Google, auth asks for an ID token whose `nonce` names it in this network (`joinNonce`, with `ROOT`),
// and keeps the token Google gave; with a passkey, the prompt's challenge names it, the network and when
// (`joinChallenge`), and auth keeps the assertion. Either is kept (`proofs`) and passed on unchanged with that device's
// request to join (`sign_in`), never checked here: only the user's devices count it, and only for an account their
// own devices recorded. Refusing a request forgets its proof too. A throwaway network's auth signs its own ID token, with
// nothing else to show.
//
// Only a device that signed in itself is let in by its sign-in: a sign-in finished from a link opened on another device
// (the CLI's sign-in link, or any link sent to the user) could have been started by anyone, who'd then hold the token
// it gave. Auth tells them apart by the client whose token asks to join: one whose every sign-in comes back to the
// device it was made on (the web client's page, the native apps' `<cli>://signed-in/` or loopback) signed in itself;
// any other's (the sign-in code's page, a site) came back elsewhere, and its proof is passed on as `{ by_link: proof }`,
// which the user's devices approve only by a tap.
//
// It pushes (`PUSH`, the push Worker's entrypoint for the network's own Workers) to the user's devices, never the one asking: "<device>
// wants in · <place>", with the action `{ join: <code> }`, when nothing approved a request within `WANTS_IN`; and
// "<device> joined · <place>" once one is approved, opening Settings › Devices. It knows a user's devices as those that
// asked to join as them here, or said they're theirs as they asked which wait (`devices`), and those that registered
// with the push Worker as theirs, by a request each signed (`PUSH.theirs`): only for whom to push to, since a device
// opens a note, and acts on its code, only as the network's signed steps say.
//
// What an endpoint signs (`signedRequest` in shared/crypto.js, domain `one/auth`) names its route (`route: "POST
// /api/auth/invites"`), is taken once, and counts only from one of the network's endpoints, as the relay's list says
// (`RELAY.allowed(key)`), an admin's for what only admins do. A POST's body is the signed request; a GET's or DELETE's
// is in `x-one-request`, as base64url of its JSON.
//
// A throwaway network's auth (`SIGN_IN` is "throwaway") lets in its first user (`THROWAWAY_USER`, `THROWAWAY_NAME`) with no passkey and
// no Google: its sign-in page goes straight on. Someone it invites joins by opening their link, again with no passkey,
// and is then who that browser signs in as. It's for testing clients without a user: only `one-dev throwaway` turns it on
// (cloudflare::Settings::throwaway_user), on Workers that listen only on their own machine, and `cloudflare deploy` refuses it,
// so no real network's auth ever does this.

import { DurableObject } from "cloudflare:workers";
import { oauthProvider } from "@better-auth/oauth-provider";
import { getAuthenticatorName, passkey } from "@better-auth/passkey";
import { betterAuth } from "better-auth";
import { APIError, createAuthEndpoint, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { setSessionCookie } from "better-auth/cookies";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { b64u, behind, checkRequest, hex, isKey, joinChallenge, joinCode, joinNonce, random, sha256 } from "../shared/crypto.js";
import { json } from "../shared/http.js";
import { AUTH_DOMAIN, HEADER, updateRefusal } from "../shared/protocol.js";
import { bearer, verify } from "../shared/token.js";
import { pages } from "../onboarding/pages.js";
import css from "../web/screen/ui/auth.css" with { type: "text" };
import drawn from "../web/screen/ui/auth.json";
import { DurableObjectDialect } from "./sql.js";

const { codePage, invitePage, passkeysPage, signInPage, throwawayPage } = pages({ css, ...drawn });

const hex16 = () => hex.encode(random(8));
/// Where a client's sign-in goes, in words: the site it goes back to, or an app on the user's device (one listening on
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

function basicClient(headers) {
  const basic = /^basic (.+)$/i.exec(headers?.get?.("authorization") || "");
  try {
    return basic ? decodeURIComponent(atob(basic[1].trim()).split(":")[0]) : null;
  } catch {
    return null;
  }
}

/// Refuses, as Better Auth does, since some refusals are made inside its routes.
const refuse = (status, message) => {
  throw new APIError({ 400: "BAD_REQUEST", 401: "UNAUTHORIZED", 403: "FORBIDDEN", 404: "NOT_FOUND", 409: "CONFLICT", 503: "SERVICE_UNAVAILABLE" }[status], { message });
};
const INVITE = 30 * 86_400_000;
const REQUEST = 15 * 60_000;
/// How long a request waits for a device to let it in by itself before its user is pushed "wants in".
const WANTS_IN = 10_000;
/// The most devices of one user pushed to, the newest.
const PUSHED_TO = 20;
const CODE = 10 * 60_000;
/// How lately someone signed in for a client to be added as them, or a passkey added.
const FRESH = 5 * 60_000;
/// How long a Google sign-in started for a client's authorization, or to join, goes on to it once it's back.
const STARTED = 15 * 60_000;
/// A sign-in code's letters: no vowels, so it spells no word, and none that look alike.
const LETTERS = "BCDFGHJKLMNPQRSTVWXZ";
/// Where Better Auth makes a session with a passkey: the only places an admin's is made.
const BY_PASSKEY = ["/passkey/verify-authentication", "/passkey/verify-registration"];
const USER = /^[0-9a-f]{16}$/;
/// A device a sign-in is for: its key and seal key, hex.
const JOIN = /^[0-9a-f]{128}$/;

/// The device an authorization's query says it signs in for, if it says: its key ‖ seal key, hex.
function joinOf(query) {
  const join = new URLSearchParams(typeof query === "string" ? query : "").get("join");
  return JOIN.test(join || "") ? join : null;
}

const endpointOf = (join) => [hex.decode(join.slice(0, 64)), hex.decode(join.slice(64))];

/// The network's root (`ROOT`, hex, as the device deploying to Cloudflare holds it), which a sign-in for a device
/// names beside it: without it, a sign-in names no device, and its device is approved by its code.
const rootOf = (env) => (isKey(env.ROOT) ? hex.decode(env.ROOT) : null);

const asB64u = (s) => String(s || "").replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");

async function hash(secret) {
  return hex.encode(await sha256(secret));
}

/// A passkey's name, for the user's list of them: its maker's, if it says, a security key's, or the device it was
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
    /// Only a test binds `TEST` (tests/cloudflare.rs on the Workers it serves itself; a network's bindings come from its
    /// settings, which hold none): the wait for "wants in" is judged by a clock a day behind, which `POST /api/auth/test`
    /// moves on, so it passes when a test says and not by itself.
    this.skew = env.TEST ? -86_400_000 : 0;
    const sql = ctx.storage.sql;
    // Invites, by their secret's hash, and `by`, the endpoint that made one; kept once used, for the requests of the
    // device its user joined with.
    sql.exec("create table if not exists invites (id text primary key, name text not null, user text not null unique, email text, until integer not null, by text not null, used integer)");
    // Devices' requests to join, by their key: `invite` is the invite of a newcomer's first device, with its `proof`;
    // `approved`, when a device said it let it in, `root`, the network it said it let it into, and `welcome`, where
    // it said to start reading who's who from; `place`, the city it asked from; `device`, the device it's part of; `platform`, what it says it's built for;
    // `by_link`, whether its sign-in was finished from a link opened elsewhere (`signsInHere`); and `told`, when its
    // user was pushed that it wants in.
    sql.exec("create table if not exists requests (key text primary key, seal text not null, name text not null, user text not null, proof text, invite text, made integer not null, approved integer, root text, welcome text, place text, device text, by_link integer, told integer)");
    // A Durable Object keeps its table from before this column.
    if (!sql.exec("select 1 from pragma_table_info('requests') where name = 'platform'").toArray().length) sql.exec("alter table requests add column platform text");
    // A user's devices, by key, as they asked to join here or said they're theirs, for pushes.
    sql.exec("create table if not exists devices (key text primary key, user text not null, at integer not null)");
    // Devices whose request was turned down, whose user isn't pushed again when they ask again.
    sql.exec("create table if not exists refused (key text primary key, at integer not null)");
    // The users who sign in only with a passkey: admins.
    sql.exec("create table if not exists passkey_only (user text primary key)");
    // Signed requests taken, until they're too old to take again.
    sql.exec("create table if not exists signed (sig text primary key, at integer not null)");
    sql.exec("create table if not exists codes (code text primary key, state text not null unique, query text not null, until integer not null, back text)");
    sql.exec("create table if not exists google_starts (id text primary key, query text not null, made integer not null, join_for text)");
    // What a user's sign-in for a device showed (`{ id_token }` or `{ passkey }`), by that device's key and the user,
    // for its request to join to carry.
    sql.exec("create table if not exists proofs (key text not null, user text not null, proof text not null, made integer not null, primary key (key, user))");
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
    const throwaway = env.SIGN_IN === "throwaway";
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
            // Only someone invited signs up, as who the invite says they are in the network, so devices know them by
            // the same id: with a passkey, as it's registered, or a throwaway network's link (`admit`); with Google, as
            // Google says the email is theirs, with the invite this browser opened to join.
            async before(user, ctx) {
              if (self.admitting && user.id === self.admitting) return;
              if (throwaway) return user.id === env.THROWAWAY_USER ? undefined : false;
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
              if (throwaway || BY_PASSKEY.includes(ctx?.path)) return;
              // With a code, Better Auth sends the browser back to its page with the reason, not a bare 403.
              if (sql.exec("select 1 from passkey_only where user = ?", session.userId).toArray().length) throw new APIError("FORBIDDEN", { code: "ADMIN_SIGNS_IN_WITH_A_PASSKEY", message: "You're an admin, so you sign in with your passkey." });
            },
          },
        },
      },
      hooks: {
        before: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/authorize" && ctx.query) {
            ctx.query.resource = await self.audience(ctx.query.client_id);
            const scopes = String(ctx.query.scope || "").split(" ").filter(Boolean);
            if (!scopes.includes("offline_access")) ctx.query.scope = [...scopes, "offline_access"].join(" ");
          }
          if (ctx.path === "/oauth2/token" && ctx.body) ctx.body.resource = await self.audience(ctx.body.client_id || basicClient(ctx.headers));
        }),
      },
      plugins: [
        jwt({ jwks: { keyPairConfig: { alg: "EdDSA", crv: "Ed25519" } }, jwt: { issuer: `${url.origin}/api/auth`, expirationTime: "1h" } }),
        ...(throwaway ? [throwawayNetwork(self, env.THROWAWAY_USER, env.THROWAWAY_NAME)] : [passkey({
          rpID: url.hostname,
          rpName: env.NAME || "The One App",
          origin: url.origin,
          authentication: {
            async afterVerification({ clientData }) {
              self.keepAssertion(clientData);
            },
          },
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
          // A refresh token used again within a minute of its rotation gets the same answer again (kept sealed under
          // auth's secret), so an answer lost on the way (a dropped connection, a tab closed, a proxy that failed after
          // auth rotated) signs no one out; used again after that, it's taken as stolen, and its whole family ends.
          refreshTokenReuseInterval: 60,
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
      if (path === "/api/auth/passkey/generate-authenticate-options") return await this.namingDevice(await this.auth.handler(request), url.searchParams.get("join"));
      if (path === "/api/auth/sign-in/social" && request.method === "POST") return await this.withNonce(request, await this.auth.handler(request));
      if (path.startsWith("/api/auth/")) return await this.auth.handler(request);
      if (path.startsWith("/.well-known/oauth-authorization-server") || path.startsWith("/.well-known/openid-configuration")) {
        const which = path.includes("openid") ? "/.well-known/openid-configuration" : "/.well-known/oauth-authorization-server";
        const meta = await this.auth.handler(new Request(`${url.origin}/api/auth${which}`, { headers: request.headers }));
        return new Response(meta.body, { status: meta.status, headers: { "content-type": "application/json", "access-control-allow-origin": "*" } });
      }
      const name = this.auth.options.appName;
      // What its pages say: the custom build's name, and what a user joins ("your One App network").
      const look = { name, yours: this.env.YOURS, logo: "/logo.png" };
      const google = this.env.SIGN_IN === "google" ? this.env.JOIN_DOMAIN : null;
      const throwaway = this.env.SIGN_IN === "throwaway";
      if (path === "/sign-in" && request.method === "GET") return html(throwaway ? throwawayPage({ ...look, user: this.env.THROWAWAY_NAME }) : signInPage({ ...look, google }));
      if (path === "/sign-in/about") return json(await this.about(request, url.searchParams));
      if (path === "/sign-in/google" && request.method === "POST" && google) return await this.startGoogle(request, await request.json());
      if (path === "/sign-in/code" && request.method === "POST") return json(await this.codeFor(await request.json()));
      if (path === "/sign-in/code" && request.method === "GET") return this.codeOpened(url.searchParams.get("c"), look, google);
      if (path === "/sign-in/code/done" && request.method === "GET") return html(codePage({ ...look, google, done: await this.codeDone(url.searchParams) }));
      if (path === "/sign-in/code/take" && request.method === "POST") return this.codeTaken(await request.json());
      if (path === "/sign-in/accept" && request.method === "POST") return json(await this.accept(request, await request.json()));
      if (path === "/invite" && request.method === "GET") return html(invitePage({ ...look, throwaway, google }));
      if (path === "/invite/about" && request.method === "POST") return json(await this.inviteAbout(await request.json()));
      if (path === "/invite/google" && request.method === "POST" && google) return await this.joinWithGoogle(request, await request.json());
      if (path === "/passkeys" && request.method === "GET" && !throwaway) return html(passkeysPage(look), { framed: true });
      return new Response("not found", { status: 404 });
    } catch (err) {
      // Our own refusals have a numeric status; Better Auth's say theirs in statusCode.
      const status = typeof err.status === "number" ? err.status : err.statusCode;
      if (status >= 400 && status < 500 || status === 503) return json({ error: err.body?.error_description || err.body?.message || err.message }, status);
      console.error(err.stack || err);
      return json({ error: "the sign-in failed" }, 500);
    }
  }

  /// The routes of auth's own under /api/auth/: invites, admins and devices' requests to join. → their answer, or
  /// nothing for any other path.
  async route(request, url) {
    const path = url.pathname;
    const method = request.method;
    if (path === "/api/auth/test" && method === "POST" && this.env.TEST) {
      this.skew += (await request.json()).advance;
      await this.alarm();
      return json({});
    }
    if (path === "/api/auth/invites" && method === "POST") return json(await this.invite(request));
    if (path === "/api/auth/users/admin" && method === "POST") return json(await this.setAdmin(request));
    if (path === "/api/auth/devices/requests" && method === "POST") return json(await this.ask(request));
    if (path === "/api/auth/devices/requests" && method === "GET") return json(await this.requests(request));
    const one = path.match(/^\/api\/auth\/devices\/requests\/([0-9a-f]{64})$/);
    if (one && method === "GET") return json(await this.request(request, one[1]));
    if (one && method === "DELETE") return json(await this.unask(request, one[1], url.searchParams.has("approved"), url.searchParams.get("root"), url.searchParams.get("welcome")));
    return null;
  }

  /// What an endpoint signed for this request's route, if it brought one (its body, or `x-one-request`), checked:
  /// fresh, taken once, and from one of the network's endpoints. → `{ by, fields, admin }`, or null.
  async signed(request, body) {
    const route = `${request.method} ${new URL(request.url).pathname}`;
    let asked = body;
    if (asked === undefined) {
      const header = request.headers.get(HEADER.request);
      if (!header) return null;
      try { asked = JSON.parse(new TextDecoder().decode(b64u.decode(header))); } catch { refuse(400, `${HEADER.request} isn't a signed request`); }
    }
    const { by, fields } = await checkRequest(AUTH_DOMAIN, asked).catch((err) => refuse(403, `That request isn't signed as it should be: ${err.message}.`));
    if (fields.route !== route) refuse(403, `That request was signed for ${fields.route}, not ${route}.`);
    const sql = this.ctx.storage.sql;
    sql.exec("delete from signed where at <= ?", Date.now() - 2 * 60_000);
    this.taken(asked.sig);
    if (!this.env.RELAY) refuse(503, "The network has no relay to say which devices are its own.");
    const allowed = await this.env.RELAY.allowed(by);
    if (!allowed) refuse(403, "That device isn't one of the network's.");
    return { by, fields, admin: allowed.admin === true };
  }

  taken(sig) {
    const sql = this.ctx.storage.sql;
    sql.exec("delete from signed where at <= ?", Date.now() - 2 * 60_000);
    if (sql.exec("select 1 from signed where sig = ?", sig).toArray().length) refuse(403, "That request was sent already.");
    sql.exec("insert into signed (sig, at) values (?, ?)", sig, Date.now());
  }

  /// Someone signed in: by a session here, or an access token auth signed. → their user id, or null.
  async user(request) {
    return (await this.asker(request))?.user || null;
  }

  /// Who's asking, signed in: `{ user, here }`, `here` whether they signed in on the device asking, as a session here
  /// is (it's this browser's) and a token is of a client that `signsInHere`. → null if no one.
  async asker(request) {
    const token = bearer(request);
    if (token) {
      const origin = new URL(this.env.ADDRESS).origin;
      const jwks = await (await this.auth.handler(new Request(`${origin}/api/auth/jwks`))).json();
      const claims = await verify(token, jwks, `${origin}/api/auth`).catch((err) => refuse(401, `That access token won't do: ${err.message}.`));
      const client = await this.client(String(claims.azp || claims.client_id || "")).catch(() => null);
      return { user: claims.sub, here: !!client && this.signsInHere(client) };
    }
    const user = (await this.auth.api.getSession({ headers: request.headers }))?.user.id;
    return user ? { user, here: true } : null;
  }

  /// An invite, made by an admin's device: `{ id, name, user, email?, until }`, `id` the sha256 (hex) of the secret
  /// its link carries, `user` the id they'll have. On a network whose users sign in with Google, `email` is the
  /// Google account they sign up with. Another invite for the same user, not yet used, replaces it.
  async invite(request) {
    const signed = await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")));
    if (!signed?.admin) refuse(403, "Only an admin's device invites someone.");
    const { id, name, user, email, until } = signed.fields;
    if (!isKey(id)) refuse(400, "an invite's id is the sha256 of its secret, in hex");
    if (typeof name !== "string" || !name.trim()) refuse(400, "give the user's name");
    if (!USER.test(user || "")) refuse(400, "a user id is 16 hex characters");
    if (!(Number.isSafeInteger(until) && until > Date.now() && until <= Date.now() + INVITE)) refuse(400, "an invite works for up to 30 days");
    if (email != null && (typeof email !== "string" || !/^[^@\s]+@[^@\s]+$/.test(email))) refuse(400, `${email} isn't an email`);
    if (this.env.SIGN_IN === "google" && !String(email || "").toLowerCase().endsWith(`@${this.env.JOIN_DOMAIN}`)) refuse(400, `Users here sign in with Google: give their email at ${this.env.JOIN_DOMAIN}.`);
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
    if (admin && this.env.SIGN_IN !== "throwaway" && !sql.exec("select 1 from passkey where userId = ?", user).toArray().length) refuse(409, "They have no passkey yet, which an admin signs in with: they add one on their Passkeys page first.");
    if (admin) sql.exec("insert or ignore into passkey_only (user) values (?)", user);
    else sql.exec("delete from passkey_only where user = ?", user);
    return {};
  }

  expire() {
    // An approved one is kept as long again from its approval, for its device to see.
    this.ctx.storage.sql.exec("delete from requests where coalesce(approved, made) <= ?", Date.now() - REQUEST);
    this.ctx.storage.sql.exec("delete from proofs where made <= ?", Date.now() - REQUEST);
  }

  keepProof(join, user, proof) {
    this.ctx.storage.sql.exec("insert or replace into proofs (key, user, proof, made) values (?, ?, ?, ?)", join.slice(0, 64), user, JSON.stringify(proof), Date.now());
  }

  /// A passkey's options for a sign-in for the device `join` names (128 hex): its challenge is that device's, and when,
  /// kept as the one its assertion must sign. Any other is as Better Auth made it, and so is one whose challenge it
  /// couldn't change: that sign-in shows nothing, and its device is approved by its code.
  async namingDevice(resp, join) {
    const root = rootOf(this.env);
    if (!JOIN.test(join || "") || !resp.ok || !root) return resp;
    const out = await resp.clone().json();
    const ours = b64u.encode(joinChallenge(root, ...endpointOf(join), Date.now()));
    const [was, now] = [`"expectedChallenge":"${out.challenge}"`, `"expectedChallenge":"${ours}"`];
    const sql = this.ctx.storage.sql;
    sql.exec("update verification set value = replace(value, ?, ?) where instr(value, ?) > 0", was, now, was);
    // Counted by what's there, as rowsWritten counts an index's writes too.
    if (sql.exec("select count(*) as n from verification where instr(value, ?) > 0", now).one().n !== 1) {
      console.error("a passkey's challenge wasn't changed to name its device");
      return resp;
    }
    out.challenge = ours;
    const headers = new Headers(resp.headers);
    headers.delete("content-length");
    return new Response(JSON.stringify(out), { status: resp.status, headers });
  }

  /// A passkey's assertion Better Auth just checked: kept if its challenge names a device (`namingDevice`), with the
  /// passkey's public key, which counts only if a user approves that device by its code.
  keepAssertion(resp) {
    try {
      this.keepAssertionNow(resp);
    } catch (err) {
      console.error(`keeping a passkey's assertion: ${err.stack || err}`);
    }
  }

  keepAssertionNow(resp) {
    const r = resp?.response || {};
    let challenge;
    try { challenge = b64u.decode(JSON.parse(new TextDecoder().decode(b64u.decode(r.clientDataJSON))).challenge); } catch { return; }
    const prefix = new TextEncoder().encode("one/join\0");
    if (challenge.length !== prefix.length + 104 || prefix.some((b, i) => challenge[i] !== b)) return;
    const join = hex.encode(challenge.slice(prefix.length + 32, prefix.length + 96));
    const [row] = this.ctx.storage.sql.exec("select userId, publicKey from passkey where credentialID = ?", resp.id).toArray();
    if (!row) return;
    this.keepProof(join, row.userId, { passkey: { credential: resp.id, authenticator_data: r.authenticatorData, client_data: r.clientDataJSON, signature: r.signature, public_key: asB64u(row.publicKey) } });
  }

  /// Google's sign-in started for a client's sign-in for a device (`startGoogle`): asks Google for an ID token whose
  /// nonce names that device.
  async withNonce(request, resp) {
    const secret = cookieOf(request.headers, cookieName(this.env, "one-google-start"));
    const [row] = secret ? this.ctx.storage.sql.exec("select join_for from google_starts where id = ?", await hash(secret)).toArray() : [];
    const root = rootOf(this.env);
    if (!row?.join_for || !resp.ok || !root) return resp;
    const out = await resp.clone().json().catch(() => null);
    if (!out) return resp;
    const headers = new Headers(resp.headers);
    headers.delete("content-length");
    if (out.url) {
      const to = new URL(out.url);
      to.searchParams.set("nonce", await joinNonce(root, ...endpointOf(row.join_for)));
      out.url = to.toString();
      if (headers.has("location")) headers.set("location", out.url);
    }
    return new Response(JSON.stringify(out), { status: resp.status, headers });
  }

  /// A device signed in asks to join: `{ key, seal, name, reads, proof?, device?, platform? }`, its endpoint keys (hex), its name,
  /// what its build reads of who's who (`READS`), for a
  /// newcomer's first device, `keys::invite_proof` of its invite's secret, key and seal key, kept with that invite for the
  /// inviting admin's device to check, and for a native app, the device it's on (64 hex). → until when it waits.
  async ask(request) {
    const body = await request.json().catch(() => refuse(400, "the body isn't JSON"));
    const { user, here } = (await this.asker(request)) || refuse(401, "Sign in first.");
    const { key, seal, name, proof, device, platform, reads } = body;
    if (!isKey(key) || !isKey(seal)) refuse(400, "key and seal are endpoint keys, 64 hex characters each");
    // A device whose build reads less of who's who than this one (an older one says nothing) would be let in to what it
    // can't read: it's told to update first.
    if (behind(reads)) refuse(426, updateRefusal(this.env.YOURS));
    if (typeof name !== "string" || !name.trim() || name.length > 64) refuse(400, "give the device's name, up to 64 characters");
    if (proof != null && !isKey(proof)) refuse(400, "a proof is 64 hex characters");
    if (device != null && !isKey(device)) refuse(400, "a device is 64 hex characters");
    if (platform != null && (typeof platform !== "string" || platform.length > 16)) refuse(400, "a platform is a short word");
    this.expire();
    const sql = this.ctx.storage.sql;
    const [had] = sql.exec("select user, seal, approved, told from requests where key = ?", key).toArray();
    if (had && had.user !== user) refuse(409, "Another user's device asked with that key.");
    // Asked again once it's approved (it restarted while waiting): still approved, for it to see.
    if (had?.approved && had.seal === seal) return { until: had.approved + REQUEST };
    let invite = null;
    if (proof != null) {
      [invite] = sql.exec("select id from invites where user = ? and used is not null", user).toArray();
      if (!invite) refuse(400, "Only a newcomer's first device brings an invite's proof.");
    }
    const made = Date.now();
    // Where it asked from, as Cloudflare places it: its city, shown beside its code.
    const place = typeof request.cf?.city === "string" ? request.cf.city.slice(0, 64) : null;
    // Asked again while it waits: its user isn't pushed again.
    const told = had && had.user === user && had.seal === seal ? had.told : null;
    sql.exec("insert or replace into requests (key, seal, name, user, proof, invite, made, approved, place, device, by_link, told, platform) values (?, ?, ?, ?, ?, ?, ?, null, ?, ?, ?, ?, ?)", key, seal, name.trim(), user, proof ?? null, invite?.id ?? null, made, place, device ?? null, here ? null : 1, told, platform ?? null);
    this.theirs(key, user);
    if (!told) await this.wantsInBy(made + WANTS_IN);
    return { until: made + REQUEST };
  }

  theirs(key, user) {
    this.ctx.storage.sql.exec("insert or replace into devices (key, user, at) values (?, ?, ?)", key, user, Date.now());
  }

  async wantsInBy(at) {
    const now = await this.ctx.storage.getAlarm();
    if (now == null || now > at) await this.ctx.storage.setAlarm(at);
  }

  async alarm() {
    this.expire();
    const sql = this.ctx.storage.sql;
    const due = Date.now() + this.skew - WANTS_IN;
    for (const r of sql.exec("select * from requests where approved is null and told is null and made <= ?", due).toArray()) {
      sql.exec("update requests set told = ? where key = ?", Date.now(), r.key);
      if (sql.exec("select 1 from refused where key = ?", r.key).toArray().length) continue;
      await this.push(r, { title: `${r.name} wants in${r.place ? ` · ${r.place}` : ""}`, action: { join: await joinCode(hex.decode(r.key), hex.decode(r.seal)) } });
    }
    const [next] = sql.exec("select min(made) as at from requests where approved is null and told is null").toArray();
    if (next?.at != null) await this.ctx.storage.setAlarm(Math.max(next.at + WANTS_IN, Date.now() + 1000));
  }

  /// Pushes a note about the request `r` to its user's devices, but the one asking: tapping it opens Settings ›
  /// Devices. Never fails what it's said with.
  async push(r, note) {
    if (!this.env.PUSH) return;
    const known = this.ctx.storage.sql.exec("select key from devices where user = ? order by at desc limit ?", r.user, PUSHED_TO).toArray().map((d) => d.key);
    const plain = { id: hex16(), app: "settings", screen: "devices", ...note };
    try {
      const registered = await this.env.PUSH.theirs(r.user, PUSHED_TO);
      const to = [...new Set([...registered, ...known])].filter((key) => key !== r.key).slice(0, PUSHED_TO);
      if (to.length) await this.env.PUSH.send(to.map((key) => ({ to: key, plain })));
    } catch (err) {
      console.error(`pushing "${note.title}": ${err.stack || err}`);
    }
  }

  shown(rows) {
    const sql = this.ctx.storage.sql;
    return rows.map((r) => {
      const user_name = sql.exec("select name from user where id = ?", r.user).toArray()[0]?.name || "";
      const [signed] = sql.exec("select proof from proofs where key = ? and user = ?", r.key, r.user).toArray();
      const signIn = signed && (r.by_link ? { by_link: JSON.parse(signed.proof) } : JSON.parse(signed.proof));
      return { key: r.key, seal: r.seal, name: r.name, user: r.user, user_name, ...(r.proof ? { proof: r.proof } : {}), ...(signIn ? { sign_in: signIn } : {}), ...(r.approved ? { approved: true } : {}), ...(r.root ? { root: r.root } : {}), ...(r.welcome ? { welcome: r.welcome } : {}), ...(r.place ? { place: r.place } : {}), ...(r.device ? { device: r.device } : {}), ...(r.platform ? { platform: r.platform } : {}), made: r.made, until: (r.approved || r.made) + REQUEST };
    });
  }

  /// Requests waiting: the signed-in user's own devices', or, for a signed request from one of the network's
  /// devices, every one: a device approves only its own user's, or a newcomer's whose invite its user keeps, as
  /// the steps it writes count only then. Signed in and signed both, it's their device asking: kept as theirs, for
  /// pushes, and shown their own.
  async requests(request) {
    this.expire();
    const sql = this.ctx.storage.sql;
    const user = bearer(request) ? await this.user(request) : null;
    const signed = user ? await this.signed(request).catch(() => null) : await this.signed(request);
    if (signed && !user) {
      return { requests: this.shown(sql.exec("select * from requests where approved is null order by made").toArray()) };
    }
    if (signed) this.theirs(signed.by, user);
    const theirs = user || (await this.user(request)) || refuse(401, "Sign in first.");
    return { requests: this.shown(sql.exec("select * from requests where user = ? and approved is null order by made", theirs).toArray()) };
  }

  async mayTouch(request, key) {
    this.expire();
    const sql = this.ctx.storage.sql;
    const signed = await this.signed(request);
    const [row] = sql.exec("select * from requests where key = ?", key).toArray();
    const ok = row && (signed || row.user === ((await this.user(request)) || refuse(401, "Sign in first.")));
    if (!ok) refuse(404, "No request waits with that key: it was approved, refused or waited too long.");
    return row;
  }

  async request(request, key) {
    const row = await this.mayTouch(request, key);
    return this.shown([row])[0];
  }

  /// Forgets a request, refused; or one `approved`, marked so until it would have expired, for its device to see, with
  /// the network's `root` (hex) it was let into and its `welcome`, if the approving device said.
  async unask(request, key, approved, root, welcome) {
    const row = await this.mayTouch(request, key);
    // Approved again (a retry, or a second approving device): marked once, never forgotten by it.
    if (approved) {
      if (!row.approved) {
        const welcomed = /^\d{1,16}\.\d{1,16}\.[0-9a-f]{64}$/.test(welcome || "") ? welcome : null;
        this.ctx.storage.sql.exec("update requests set approved = ?, root = ?, welcome = ? where key = ?", Date.now(), isKey(root) ? root : null, welcomed, key);
        await this.push(row, { title: `${row.name} joined${row.place ? ` · ${row.place}` : ""}` });
      }
      return {};
    }
    // Refused: what its sign-in showed goes too, so asking again shows nothing a device lets it in by, nor pushes.
    this.ctx.storage.sql.exec("delete from requests where key = ?", key);
    this.ctx.storage.sql.exec("delete from proofs where key = ?", key);
    this.ctx.storage.sql.exec("delete from devices where key = ?", key);
    this.ctx.storage.sql.exec("delete from refused where at <= ?", Date.now() - 30 * 86_400_000);
    this.ctx.storage.sql.exec("insert or replace into refused (key, at) values (?, ?)", key, Date.now());
    return {};
  }

  async session(request) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    if (!session) throw Object.assign(new Error("sign in first"), { status: 401 });
    return session;
  }

  /// What a client's tokens are for, whatever it asks: the network's address (a sign-in at its nodes, through the
  /// gateway) only for its own apps, each a client whose every redirect comes back to one of them (`firstParty`);
  /// any other, a remote MCP client, gets `<address>/mcp`, the remote MCP server's, where it's a device of its user's,
  /// which nodes refuse.
  async audience(clientId) {
    const address = JSON.parse(this.env.RESOURCES || "[]")[0] || this.env.ADDRESS;
    const client = clientId && (await this.client(String(clientId)).catch(() => null));
    return client && this.firstParty(client) ? address : `${address}/mcp`;
  }

  /// Whether a client's every sign-in comes back to the device it was made on: the web client's page at the network's
  /// address (in the browser that signed in), an app's own link scheme or loopback. Not the sign-in code's page, where
  /// a sign-in finished in any browser is taken by whichever device made the code, nor any site.
  signsInHere(client) {
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const origin = new URL(this.env.ADDRESS).origin;
    // The network's own address is on loopback too where it's tried (`one-dev throwaway`), so its pages are told apart first.
    const here = (u) => (URL.canParse(u) && new URL(u).origin === origin ? u === `${origin}/callback` : goesTo(u, origin) === ON_DEVICE);
    return uris.length > 0 && uris.every(here);
  }

  /// Whether a client is one of the network's own apps, by where its sign-ins come back: the web client's page, the
  /// page a machine's sign-in code is shown on, the native apps' `<cli>://signed-in/`, or Windows' loopback.
  firstParty(client) {
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const origin = new URL(this.env.ADDRESS).origin;
    const ours = [`${origin}/callback`, `${origin}/sign-in/code/done`, `${this.env.CLI || "one"}://signed-in/`, "http://127.0.0.1/callback"];
    return uris.length > 0 && uris.every((u) => ours.includes(u));
  }

  async client(clientId) {
    const client = clientId && (await (await this.auth.$context).adapter.findOne({ model: "oauthClient", where: [{ field: "clientId", value: clientId }] }));
    if (!client) throw Object.assign(new Error("no such client"), { status: 404 });
    return client;
  }

  /// A client names itself, so none may take the network's name, as signing in to it would read as signing in here;
  /// nor its apps' ("<network> on <device>"), unless it comes back to the device, as they do.
  notNamedAsUs(body) {
    const plain = (s) => String(s || "").toLowerCase().replace(/[^\p{L}\p{N}]/gu, "").replace(/^the/, "");
    const [name, ours] = [plain(body.client_name), plain(this.auth.options.appName)];
    const origin = new URL(this.env.ADDRESS).origin;
    const toSites = [].concat(body.redirect_uris || []).some((u) => ![ON_DEVICE, BY_CODE].includes(goesTo(u, origin)));
    if (name === ours || (name.startsWith(ours) && toSites)) refuse(400, `a client can't be called ${body.client_name}: that's the name of what it signs in to`);
  }

  /// What signing in to a client (its authorization's `query`) says: its name (the network's, for the web client, which
  /// is at its address), where its sign-in goes (its redirect_uri, if it's one of the client's, else its first), since
  /// the name is only what it calls itself, the user's, if they're signed in here already, and whether they're back
  /// from the Google sign-in this browser started for it, which goes straight on.
  async about(request, query) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    const client = await this.client(query.get("client_id"));
    const redirect = query.get("redirect_uri");
    const origin = new URL(this.env.ADDRESS).origin;
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const web = uris.length > 0 && uris.every((u) => u === `${origin}/callback`);
    const goes = web ? null : goesTo(uris.includes(redirect) ? redirect : uris[0], origin);
    return { client: web ? this.auth.options.appName : client.name || client.clientId, web, goes, user: session?.user.name || null, fresh: !!(await this.started(request, session, query)) };
  }

  // Signing in to a client with Google goes straight on once Google says yes, with no second tap, but only for the
  // authorization this browser started it for: a sign-in with Google made for another client (the web client's, a
  // minute ago) gives nothing to a client whose link the user was sent. Starting one keeps a secret in a cookie only
  // this address sets, with a hash of the authorization's query; coming back, the two must match, once.

  async startGoogle(request, { oauth_query }) {
    this.fromOurPages(request);
    const query = new URLSearchParams(typeof oauth_query === "string" ? oauth_query : "");
    if (query.get("client_id")) await this.client(query.get("client_id"));
    const secret = b64u.encode(crypto.getRandomValues(new Uint8Array(32)));
    const sql = this.ctx.storage.sql;
    sql.exec("delete from google_starts where made <= ?", Date.now() - STARTED);
    sql.exec("insert into google_starts (id, query, made, join_for) values (?, ?, ?, ?)", await hash(secret), await hash(query.toString()), Date.now(), joinOf(query.toString()));
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": setCookie(this.env, "one-google-start", secret, STARTED / 1000) } });
  }

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
  // request coming back to /sign-in/code/done and gets a short code for it; the user opens /sign-in/code on any
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

  async openInvite(secret) {
    if (!secret) refuse(403, "Joining takes an invite link: ask an admin for one.");
    if (typeof secret !== "string" || secret.length < 32) refuse(404, "That invite link isn't whole: open the link you were sent as it is.");
    const [invite] = this.ctx.storage.sql.exec("select * from invites where id = ?", await hash(secret)).toArray();
    if (!invite) refuse(404, "That invite link doesn't work. Ask for a new one.");
    if (invite.used) refuse(404, "That invite link has been used. Ask for a new one.");
    if (invite.until <= Date.now()) refuse(404, "That invite link has expired. Ask for a new one.");
    return invite;
  }

  /// Makes the invite's user, as their first passkey is registered (or as a throwaway network's link is opened), and
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

  async inviteAbout({ secret }) {
    const invite = await this.openInvite(secret);
    return { user: invite.name, until: invite.until, email: invite.email };
  }

  /// Joining with Google: the invite's secret kept in a cookie for Google's answer, whose email must be the invite's.
  async joinWithGoogle(request, { secret }) {
    this.fromOurPages(request);
    await this.openInvite(secret);
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": setCookie(this.env, "one-invite", secret, STARTED / 1000) } });
  }

  /// Signing in to a client (`oauth_query`) as whoever just signed in here: by tapping their passkey on its page
  /// (`passkey`), or back from the Google sign-in started for it; on a throwaway network, whoever is signed in. → where the
  /// client goes on to.
  async accept(request, { oauth_query, passkey }) {
    this.fromOurPages(request);
    const signedIn = await this.session(request);
    const query = new URLSearchParams(typeof oauth_query === "string" ? oauth_query : "");
    if (this.env.SIGN_IN !== "throwaway") {
      const started = await this.started(request, signedIn, query);
      if (!started && (!fresh(signedIn) || (this.env.SIGN_IN === "google" && passkey !== true))) refuse(403, "Sign in again.");
      if (started) this.ctx.storage.sql.exec("delete from google_starts where id = ?", started);
      // Back from Google for a device: the ID token Google just gave, its nonce that device's, is kept for its request.
      const join = joinOf(oauth_query);
      if (started && join) {
        const [account] = this.ctx.storage.sql.exec("select idToken from account where userId = ? and providerId = 'google'", signedIn.user.id).toArray();
        if (account?.idToken) this.keepProof(join, signedIn.user.id, { id_token: account.idToken });
      }
    }
    const client = await this.client(query.get("client_id"));
    const headers = new Headers(request.headers);
    headers.set("content-type", "application/json");
    headers.set("accept", "application/json");
    const resp = await this.auth.handler(new Request(new URL("/api/auth/oauth2/consent", request.url), { method: "POST", headers, body: JSON.stringify({ accept: true, oauth_query }) }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error_description || out.message || "signing in didn't work"), { status: resp.status });
    this.ctx.storage.sql.exec("delete from oauthConsent where clientId = ? and userId = ?", client.clientId, signedIn.user.id);
    return { url: out.url };
  }
}

/// A throwaway network's sign-in, with no passkey: whoever is signed in here already (someone who joined in this browser),
/// or `user` by name (someone who joined; of two with a name, the one a device was let in for), or else its first user, made the first time; and joining with an
/// invite link, which signs its user in here.
function throwawayNetwork(auth, id, name) {
  const signIn = async (ctx, user) => {
    await setSessionCookie(ctx, { session: await ctx.context.internalAdapter.createSession(user.id), user });
    await tryProof(ctx, user);
    return ctx.json({ user: user.name });
  };
  // For a sign-in for a device (`join`), an ID token of auth's own, as a provider's would be, naming that device:
  // so the user's devices are seen to let it in by its sign-in. `spoil` names another device instead, as a token they
  // must refuse.
  const tryProof = async (ctx, user) => {
    const join = JOIN.test(ctx.body?.join || "") ? ctx.body.join : null;
    const root = rootOf(auth.env);
    if (!join || !root) return;
    const origin = new URL(auth.env.ADDRESS).origin;
    const named = ctx.body.spoil === true ? "0".repeat(128) : join;
    const payload = { iss: `${origin}/api/auth`, aud: "one-throwaway", sub: user.id, iat: Math.floor(Date.now() / 1000), nonce: await joinNonce(root, ...endpointOf(named)) };
    const { token } = await auth.auth.api.signJWT({ body: { payload } });
    auth.keepProof(join, user.id, { id_token: token });
  };
  const first = async (adapter) => (await adapter.findUserById(id)) || (await adapter.createUser({ id, name, email: `${id}@people.invalid`, emailVerified: false }));
  return {
    id: "throwaway",
    endpoints: {
      signInToThrowaway: createAuthEndpoint("/throwaway/sign-in", { method: "POST" }, async (ctx) => {
        const adapter = ctx.context.internalAdapter;
        const wanted = typeof ctx.body?.user === "string" ? ctx.body.user.trim().toLowerCase() : "";
        if (wanted) {
          // Invited again, a name is two users here: the one with a device let in is who joined, else the latest to open a link.
          const [row] = auth.ctx.storage.sql.exec("select id from user where lower(name) = ? order by exists (select 1 from requests where requests.user = user.id and approved is not null) desc, createdAt desc limit 1", wanted).toArray();
          if (!row && wanted !== name.toLowerCase()) refuse(404, `Nobody called ${ctx.body.user} has joined.`);
          return signIn(ctx, row ? await adapter.findUserById(row.id) : await first(adapter));
        }
        const already = await getSessionFromCtx(ctx);
        if (already) {
          await tryProof(ctx, already.user);
          return ctx.json({ user: already.user.name });
        }
        return signIn(ctx, await first(adapter));
      }),
      joinThrowaway: createAuthEndpoint("/throwaway/join", { method: "POST" }, async (ctx) => {
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
