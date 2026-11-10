// Auth: who you are, and nothing more. Better Auth, in a Durable Object: signing in by passkey, or with any OIDC provider
// the network has registered with (`PROVIDERS`, its settings: Google is one like any other), at the network's domain if it
// has one (`JOIN_DOMAIN`, read from the provider's signed token); and an OAuth server (per
// MCP's authorization spec) that every client signs in through: the iPhone and Mac apps, and MCP clients like Claude
// Code and ChatGPT. Its access tokens are JWTs signed with Ed25519, its keys at /api/auth/jwks, so nodes check them
// offline. It opens nothing: keys are only on users' devices, so faking a sign-in opens nothing either.
//
// Signing in only lets you ask: someone already in the network says yes. Nobody signs up uninvited, the network's first
// user too: an admin's device makes a secret, and tells auth its hash, who it's for and until when, signed with its
// endpoint key (`POST /api/auth/invites`); opening `<address>/invite#<secret>` lets that user sign up once, as the id and
// name the invite gives them. An invite that names an email needs no link: signing in with a provider's account for that
// email makes them, as the id it gives. At the network's domain (`JOIN_DOMAIN`, read from the provider's signed token)
// anyone may sign in and ask, which writes nothing anywhere but here; anyone else is refused before they can.
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
// hex), so whoever answers its request can check it themselves (src/onboarding/sign_in_proof.rs): with a provider, auth asks for an ID token whose `nonce` names it in this network (`joinNonce`, with `ROOT`),
// and keeps the token it gave; with a passkey, the prompt's challenge names it, the network and when
// (`joinChallenge`), and auth keeps the assertion. Either is kept (`proofs`) and passed on unchanged with that device's
// request to join (`sign_in`), never checked here, and only to who may answer it: an admin's device, or one of that
// user's own. Refusing a request forgets its proof too. A throwaway network's auth signs its own ID token, with
// nothing else to show.
//
// Only a device that signed in itself goes straight in by an invite: a sign-in finished from a link opened on another
// device (any link sent to the user) could have been started by anyone, who'd then hold the
// token it gave. Auth tells them apart by the client whose token asks to join: one whose every sign-in comes back to the
// device it was made on (the web client's page, the native apps' `<cli>://signed-in/` or loopback) signed in itself;
// any other's (a site) came back elsewhere, and its proof is passed on as `{ by_link: proof }`,
// which is approved only by a tap.
//
// It pushes (`PUSH`, the push Worker's entrypoint for the network's own Workers) to the user's devices, never the one asking: "<device>
// wants in · <place>", with the action `{ join: <code> }`, as soon as it asks (`WANTS_IN` for the one a device lets in with
// no tap), or to the network's admins if the user has no device left to ask; and "<device> joined · <place>" once one is approved, opening
// Settings › Devices. It knows a user's devices as those that
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
// no provider: its sign-in page goes straight on. Someone it invites joins by opening their link, again with no passkey,
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
import { genericOAuth, jwt } from "better-auth/plugins";
import { b64u, behind, checkRequest, discoveryUrl, hex, isKey, joinChallenge, joinCode, joinNonce, random, sha256 } from "../shared/crypto.js";
import { json } from "../shared/http.js";
import { AUTH_DOMAIN, HEADER, ID_TOKEN_ALGS, updateRefusal } from "../shared/protocol.js";
import { bearer, verify } from "../shared/token.js";
import { pages } from "../onboarding/pages.js";
import css from "../web/screen/ui/auth.css" with { type: "text" };
import drawn from "../web/screen/ui/auth.json";
import { DurableObjectDialect } from "./sql.js";

const { invitePage, linkPage, passkeysPage, signInPage, throwawayPage } = pages({ css, ...drawn });

const hex16 = () => hex.encode(random(8));
/// Better Auth's users all have an email, unique and never empty, and a user whose only identity is a passkey has none:
/// theirs is this, at a domain no one can receive mail at. It's never shown, never matched and never counts as verified.
const noEmail = (id) => `${id}@people.invalid`;
const isNoEmail = (email) => String(email || "").endsWith("@people.invalid");
/// Where a client's sign-in goes, in words: an app on the user's device (one listening on loopback, or the network's own
/// apps' `<cli>://signed-in/`, which anyone else's link can't be), or the site or other scheme it really goes to.
function goesTo(uri, cli) {
  let url;
  try { url = new URL(uri); } catch { return String(uri); }
  if (uri === `${cli}://signed-in/`) return ON_DEVICE;
  if (url.protocol !== "https:" && url.protocol !== "http:") return `${url.protocol}//${url.host}`;
  if (["localhost", "127.0.0.1", "[::1]"].includes(url.hostname)) return ON_DEVICE;
  return url.host;
}
const ON_DEVICE = "an app on this device";

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
/// How long a request waits for someone to answer it: long enough for an admin who is asleep.
const REQUEST = 3 * 86_400_000;
/// How long an invited newcomer's first device waits for an admin's device that's online to let it in (`node`'s
/// `invites_loop`, every few seconds) before anyone is pushed "wants in".
const WANTS_IN = 10_000;
/// The most devices of one user pushed to, the newest.
const PUSHED_TO = 20;
/// How lately someone signed in for one more identity to be added to their account: a session taken from them adds none,
/// since an added identity signs in for good.
const FRESH = 5 * 60_000;
/// How long an invite's secret waits in a cookie for a provider to answer.
const INVITE_COOKIE = 15 * 60;
const USER = /^[0-9a-f]{16}$/;
/// A device a sign-in is for: its key and seal key, hex.
const JOIN = /^[0-9a-f]{128}$/;

const endpointOf = (join) => [hex.decode(join.slice(0, 64)), hex.decode(join.slice(64))];

/// The network's root (`ROOT`, hex, as the device deploying to Cloudflare holds it), which a sign-in for a device
/// names beside it: without it, a sign-in names no device, and its device is approved by its code.
const rootOf = (env) => (isKey(env.ROOT) ? hex.decode(env.ROOT) : null);

/// The OIDC providers the network has registered with: `{ id, name, issuer, client_id, client_secret, domain_claim?,
/// domain_hint? }`, and only these show a button.
const providersOf = (env) => JSON.parse(env.PROVIDERS || "[]");

/// Whether an ID token, which Better Auth has checked against its provider's keys, says its account is at `domain`: by
/// the claim the provider says that with, or else a verified email there. The hint a provider's page is given decides
/// nothing, as the page is the user's to change.
function atDomain(domain, provider, claims) {
  if (!domain) return true;
  if (provider.domain_claim) return claims[provider.domain_claim] === domain;
  return claims.email_verified === true && String(claims.email || "").toLowerCase().endsWith(`@${domain}`);
}

/// A provider, for Better Auth's generic OAuth: found by its issuer, its ID tokens checked against the keys it publishes.
function oidcProvider(provider, domain) {
  return {
    providerId: provider.id,
    name: provider.name,
    clientId: provider.client_id,
    clientSecret: provider.client_secret,
    discoveryUrl: discoveryUrl(provider.issuer),
    requireIdTokenVerification: true,
    // A sign-in for a device sends a `nonce` naming it (`withNonce`), which only the user's devices check.
    disableIdTokenNonceBinding: true,
    scopes: ["openid", "email", "profile"],
    prompt: "select_account",
    authorizationUrlParams: domain && provider.domain_hint ? { [provider.domain_hint]: domain } : {},
    // From the ID token itself: what its userinfo says isn't signed.
    getUserInfo: async ({ idToken }) => {
      if (!idToken) return null;
      // Devices check only these: one signed otherwise would sign someone in whom no device can check.
      if (!ID_TOKEN_ALGS.includes(JSON.parse(new TextDecoder().decode(b64u.decode(idToken.split(".")[0]))).alg)) return null;
      const claims = JSON.parse(new TextDecoder().decode(b64u.decode(idToken.split(".")[1])));
      if (!claims.sub || !atDomain(domain, provider, claims)) return null;
      return { ...claims, id: claims.sub, emailVerified: claims.email_verified === true, image: claims.picture };
    },
  };
}

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
    // Signed requests taken, until they're too old to take again.
    sql.exec("create table if not exists signed (sig text primary key, at integer not null)");
    // Goes once every network's auth has run this: sign-in codes, which the CLI once signed in with.
    sql.exec("drop table if exists codes");
    // A Durable Object keeps this from when a provider's sign-in was started here: goes once every network's auth has run this.
    sql.exec("drop table if exists provider_starts");
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
    const providers = providersOf(env);
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
      // Someone's next identity is linked only from a device already signed in: an email a provider says is theirs
      // never joins it to a user by itself, and it needn't be the email of the identity they're signed in with.
      account: { accountLinking: { enabled: true, disableImplicitLinking: true, allowDifferentEmails: true, trustedProviders: providers.map((p) => p.id) } },
      databaseHooks: {
        user: {
          create: {
            // Someone invited signs up as who the invite says they are in the network, so devices know them by the same
            // id: with a passkey, as it's registered, or a throwaway network's link (`admit`); with a provider, as it says
            // the email is theirs, by the invite this browser opened to join or the invite naming it. Anyone else only
            // at the network's domain, which the provider's token said (`atDomain`), and only to ask.
            async before(user, ctx) {
              if (self.admitting && user.id === self.admitting) return;
              if (throwaway) return user.id === env.THROWAWAY_USER ? undefined : false;
              if (!providers.length) return false;
              const email = String(user.email || "").toLowerCase();
              const secret = cookieOf(ctx?.headers, cookieName(env, "one-invite"));
              const opened = secret && sql.exec("select * from invites where id = ?", await hash(secret)).toArray()[0];
              const invite = opened || sql.exec("select * from invites where email = ? and used is null and until > ?", email, Date.now()).toArray()[0];
              if (invite && !invite.used && invite.until > Date.now() && user.emailVerified && invite.email === email) {
                sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
                return { data: { ...user, id: invite.user, name: invite.name } };
              }
              if (!env.JOIN_DOMAIN) return false;
            },
          },
        },
      },
      hooks: {
        before: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/authorize" && ctx.query) {
            // Better Auth then gives a client's authorization a code only for a sign-in made after it was asked for: a
            // session already open, or taken from someone, gives a client nothing. Not when it runs this again itself
            // (`authorizeSettings`, which only it sets), having found that sign-in.
            if (!ctx.authorizeSettings) ctx.query.prompt = "login";
            ctx.query.resource = await self.audience(ctx.query.client_id);
            const scopes = String(ctx.query.scope || "").split(" ").filter(Boolean);
            if (!scopes.includes("offline_access")) ctx.query.scope = [...scopes, "offline_access"].join(" ");
          }
          // One more identity is linked only just after signing in, so a session taken from someone adds none.
          if (ctx.path === "/link-social") {
            const session = await getSessionFromCtx(ctx);
            if (!session || !fresh(session)) refuse(403, "Sign in again, then link it.");
          }
          // An ID token only ever comes from the provider, by its own redirect: these routes would otherwise take one
          // from anyone holding it and sign them in as its account.
          if ((ctx.path === "/sign-in/social" || ctx.path === "/link-social") && ctx.body?.idToken) refuse(403, "Sign in at the provider.");
          // Clients register at /oauth2/register alone, as nothing else here says what they may do.
          if (ctx.path === "/oauth2/create-client") refuse(404, "Register at /oauth2/register.");
          if (ctx.path === "/oauth2/token" && ctx.body) ctx.body.resource = await self.audience(ctx.body.client_id || basicClient(ctx.headers));
        }),
      },
      plugins: [
        ...(providers.length ? [genericOAuth({ config: providers.map((p) => oidcProvider(p, env.JOIN_DOMAIN)) })] : []),
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
            // Someone signed in adds a passkey to their account; someone with an invite link (its secret as the context)
            // makes their first, as their account is made. An invite that names a provider's account is for that account,
            // which a passkey made by whoever holds the link couldn't prove.
            requireSession: false,
            async resolveUser({ context }) {
              const invite = await self.openInvite(context);
              if (invite.email && providers.length) refuse(403, "This invite is for signing in with your account, not a new passkey.");
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
          // Signed in or not, a client's authorization is the sign-in page, which asks for the passkey (or a provider)
          // each time.
          loginPage: "/sign-in",
          consentPage: "/sign-in",
          scopes: ["openid", "profile", "offline_access"],
          allowDynamicClientRegistration: true,
          allowUnauthenticatedClientRegistration: true,
          // Where a client's sign-ins come back says whether they're on the device asking (`signsInHere`), so a client can
          // be registered and never changed.
          clientPrivileges: ({ action }) => action === "create",
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
      if (path === "/api/auth/passkey/generate-register-options") return await this.namedAsUser(request, await this.auth.handler(request));
      if (path === "/api/auth/passkey/generate-authenticate-options") return await this.namingDevice(await this.auth.handler(request), url.searchParams.get("join"));
      if ((path === "/api/auth/sign-in/social" || path === "/api/auth/link-social") && request.method === "POST") return await this.withNonce(await this.auth.handler(request), url.searchParams.get("join"));
      if (path.startsWith("/api/auth/")) return await this.auth.handler(request);
      if (path.startsWith("/.well-known/oauth-authorization-server") || path.startsWith("/.well-known/openid-configuration")) {
        const which = path.includes("openid") ? "/.well-known/openid-configuration" : "/.well-known/oauth-authorization-server";
        const meta = await this.auth.handler(new Request(`${url.origin}/api/auth${which}`, { headers: request.headers }));
        return new Response(meta.body, { status: meta.status, headers: { "content-type": "application/json", "access-control-allow-origin": "*" } });
      }
      const name = this.auth.options.appName;
      // What its pages say: the custom build's name, and what a user joins ("your One App network").
      const look = { name, yours: this.env.YOURS, logo: "/logo.png" };
      const providers = providersOf(this.env).map(({ id, name }) => ({ id, name }));
      const signIn = { ...look, providers, provider: providers[0]?.name, domain: this.env.JOIN_DOMAIN };
      const throwaway = this.env.SIGN_IN === "throwaway";
      if (path === "/sign-in" && request.method === "GET") return html(throwaway ? throwawayPage({ ...look, user: this.env.THROWAWAY_NAME }) : signInPage(signIn));
      if (path === "/sign-in/link" && request.method === "GET" && !throwaway) return await this.linkOpened(request, url, signIn);
      if (path === "/sign-in/link/done" && request.method === "POST") return json(await this.linkDone(request, await request.json()));
      if (path === "/sign-in/about") return json(await this.about(request, url.searchParams));
      if (path === "/invite" && request.method === "GET") return html(invitePage({ ...look, throwaway, providers }));
      if (path === "/invite/about" && request.method === "POST") return json(await this.inviteAbout(await request.json()));
      if (path === "/invite/provider" && request.method === "POST" && providers.length) return await this.joinWithProvider(request, await request.json());
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
    // Only a test binds `TEST` (tests/cloudflare.rs; a network's bindings come from its settings, which hold none).
    if (path === "/api/auth/test" && method === "POST" && this.env.TEST) {
      const { keep_proof: kept } = await request.json();
      this.ctx.storage.sql.exec("insert or replace into proofs (key, user, proof, made) values (?, ?, ?, ?)", kept.key, kept.user, JSON.stringify(kept.proof), Date.now());
      return json({});
    }
    if (path === "/api/auth/invites" && method === "POST") return json(await this.invite(request));
    const invited = path.match(/^\/api\/auth\/invites\/([0-9a-f]{16})$/);
    if (invited && method === "DELETE") return json(await this.withdrawInvite(request, invited[1]));
    if (path === "/api/auth/devices/proof" && method === "GET") return json(await this.proofsFor(request));
    if (path === "/api/auth/devices/forget-sign-in" && method === "POST") return json(await this.forgetSignIn(request));
    if (path === "/api/auth/devices/theirs" && method === "POST") return json(await this.pairedAs(request));
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
  /// its link carries, `user` the id they'll have. `email` is the account they sign up with, on a provider. Another invite for the same user, not yet used, replaces it.
  async invite(request) {
    const signed = await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")));
    if (!signed?.admin) refuse(403, "Only an admin's device invites someone.");
    const { id, name, user, email, until } = signed.fields;
    if (!isKey(id)) refuse(400, "an invite's id is the sha256 of its secret, in hex");
    if (typeof name !== "string" || !name.trim()) refuse(400, "give the user's name");
    if (!USER.test(user || "")) refuse(400, "a user id is 16 hex characters");
    if (!(Number.isSafeInteger(until) && until > Date.now() && until <= Date.now() + INVITE)) refuse(400, "an invite works for up to 30 days");
    if (email != null && (typeof email !== "string" || !/^[^@\s]+@[^@\s]+$/.test(email))) refuse(400, `${email} isn't an email`);
    if (this.env.JOIN_DOMAIN && !String(email || "").toLowerCase().endsWith(`@${this.env.JOIN_DOMAIN}`)) refuse(400, `Give their email at ${this.env.JOIN_DOMAIN}.`);
    const sql = this.ctx.storage.sql;
    if (sql.exec("select 1 from user where id = ?", user).toArray().length) refuse(409, `${name.trim()} has joined already.`);
    // Someone who has asked already keeps the id they asked as, which the invite then names.
    const invited = (email && sql.exec("select id from user where lower(email) = ?", email.toLowerCase()).toArray()[0]?.id) || user;
    sql.exec("delete from invites where until <= ? and used is null", Date.now());
    sql.exec("insert or replace into invites (id, name, user, email, until, by, used) values (?, ?, ?, ?, ?, ?, null)", id, name.trim(), invited, email ? email.toLowerCase() : null, until, signed.by);
    return { until, user: invited };
  }

  /// Withdraws the invite naming `user`, not taken up yet, for an admin's device: its email signs no one in now.
  async withdrawInvite(request, user) {
    const signed = await this.signed(request);
    if (!signed?.admin) refuse(403, "Only an admin's device withdraws an invite.");
    this.ctx.storage.sql.exec("delete from invites where user = ? and used is null", user);
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

  /// What a sign-in for the device asking (by its signature) showed, by whose it was: how it checks one more identity it
  /// has just linked.
  async proofsFor(request) {
    const signed = (await this.signed(request)) || refuse(401, "Sign in first.");
    this.expire();
    return { proofs: this.ctx.storage.sql.exec("select user, proof from proofs where key = ?", signed.by).toArray().map((p) => ({ user: p.user, proof: JSON.parse(p.proof) })) };
  }

  /// The ID token an account at `provider` last gave `user`: of two at one provider, the one just signed in with.
  lastIdToken(user, provider) {
    return this.ctx.storage.sql.exec("select idToken from account where userId = ? and providerId = ? order by updatedAt desc limit 1", user, provider).toArray()[0]?.idToken;
  }

  /// Takes an identity a user removed off their account: `{ user, account }`, asked by one of their devices or an
  /// admin's. Someone's last way to sign in stays, or they could never ask again.
  async forgetSignIn(request) {
    const signed = (await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")))) || refuse(401, "Sign in first.");
    const { user, account } = signed.fields;
    const sql = this.ctx.storage.sql;
    if (!signed.admin && this.ownerOf(signed.by) !== user) refuse(403, "Only their own device, or an admin's, forgets it.");
    const left = sql.exec("select (select count(*) from account where userId = ?1) + (select count(*) from passkey where userId = ?1) as n", user).one().n;
    if (left <= 1) return { kept: true };
    if (account?.passkey) {
      sql.exec("delete from passkey where userId = ? and credentialID = ?", user, String(account.passkey.credential));
      return {};
    }
    const provider = providersOf(this.env).find((p) => p.issuer.replace(/\/+$/, "") === String(account?.provider?.issuer || "").replace(/\/+$/, ""));
    if (!provider) refuse(404, "No such sign-in.");
    sql.exec("delete from account where userId = ? and providerId = ? and accountId = ?", user, provider.id, String(account.provider.subject));
    return {};
  }

  /// Linking one more identity, for the device `join` names: the sign-in page first, and then this, only just signed in.
  async linkOpened(request, url, look) {
    const join = url.searchParams.get("join");
    if (!JOIN.test(join || "")) return new Response("not found", { status: 404 });
    const session = await this.auth.api.getSession({ headers: request.headers });
    return html(session && fresh(session) ? linkPage(look) : signInPage({ ...look, back: `/sign-in/link?join=${join}` }));
  }

  /// A provider's account just linked for the device `join` names: the ID token it gave, kept for that device to check
  /// and record.
  async linkDone(request, { join, provider }) {
    this.fromOurPages(request);
    const session = await this.session(request);
    if (!JOIN.test(join || "") || !providersOf(this.env).some((p) => p.id === provider)) refuse(400, "That link isn't whole.");
    const token = this.lastIdToken(session.user.id, provider) || refuse(404, "That account wasn't linked. Try again.");
    this.keepProof(join, session.user.id, { id_token: token });
    return {};
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

  /// A new passkey's options, whose `user` Better Auth names by the account's email, for the device's list of them: by
  /// the user's name, for someone who has no email.
  async namedAsUser(request, resp) {
    if (!resp.ok) return resp;
    const out = await resp.clone().json();
    if (!isNoEmail(out.user?.name)) return resp;
    const session = await this.auth.api.getSession({ headers: request.headers });
    if (!session) return resp;
    out.user.name = out.user.displayName = session.user.name;
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

  /// A provider's sign-in for the device `join` names (128 hex): asks it for an ID token whose nonce names that device.
  async withNonce(resp, join) {
    const root = rootOf(this.env);
    if (!JOIN.test(join || "") || !resp.ok || !root) return resp;
    const out = await resp.clone().json().catch(() => null);
    if (!out) return resp;
    const headers = new Headers(resp.headers);
    headers.delete("content-length");
    if (out.url) {
      const to = new URL(out.url);
      to.searchParams.set("nonce", await joinNonce(root, ...endpointOf(join)));
      out.url = to.toString();
      if (headers.has("location")) headers.set("location", out.url);
    }
    return new Response(JSON.stringify(out), { status: resp.status, headers });
  }

  /// A device signed in asks to join: `{ key, seal, name, reads, proof?, device?, platform? }`, its endpoint keys (hex), its name,
  /// what its build reads of who's who (`READS`), for a
  /// newcomer's first device, `keys::invite_proof` of its invite's secret, key and seal key, kept with that invite for the
  /// inviting admin's device to check, and for a native app, the device it's on (64 hex). → until when it waits, and
  /// whether an invite already said yes (`invited`), so it shows no code to approve.
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
    // Whoever asks names any key, so this never changes whose one already is.
    sql.exec("insert or ignore into devices (key, user, at) values (?, ?, ?)", key, user, made);
    await this.keepProviderProof(key, seal, user);
    const invited = here && (proof != null || this.anInviteLetsIn(user, key));
    if (!told) {
      if (invited) await this.wantsInBy(made + WANTS_IN);
      else await this.wantsIn({ key, seal, name: name.trim(), user, place });
    }
    return { until: made + REQUEST, invited };
  }

  /// The ID token a provider last gave `user`, if it names this device: what its request carries for whoever answers it.
  async keepProviderProof(key, seal, user) {
    const root = rootOf(this.env);
    if (!root) return;
    const nonce = await joinNonce(root, hex.decode(key), hex.decode(seal));
    const [row] = this.ctx.storage.sql.exec("select idToken from account where userId = ? and idToken is not null order by updatedAt desc limit 1", user).toArray();
    if (!row) return;
    const claims = JSON.parse(new TextDecoder().decode(b64u.decode(row.idToken.split(".")[1])));
    if (claims.nonce === nonce) this.keepProof(key + seal, user, { id_token: row.idToken });
  }

  /// Whether an admin's device that's online lets this request in by itself, an invite having said yes
  /// (src/settings.rs's `Joining::New`): an invited email's first device, once it has signed in on itself.
  anInviteLetsIn(user, key) {
    const sql = this.ctx.storage.sql;
    return sql.exec("select 1 from invites where user = ? and email is not null and used is not null", user).toArray().length > 0 && !sql.exec("select 1 from devices where user = ? and key != ?", user, key).toArray().length;
  }

  /// Tells a request's user it wants in, once, unless it was turned down before.
  async wantsIn(r) {
    const sql = this.ctx.storage.sql;
    sql.exec("update requests set told = ? where key = ?", Date.now(), r.key);
    if (sql.exec("select 1 from refused where key = ?", r.key).toArray().length) return;
    await this.push(r, { title: `${r.name} wants in${r.place ? ` · ${r.place}` : ""}`, action: { join: await joinCode(hex.decode(r.key), hex.decode(r.seal)) } });
  }

  theirs(key, user) {
    this.ctx.storage.sql.exec("insert or replace into devices (key, user, at) values (?, ?, ?)", key, user, Date.now());
  }

  /// Whose device `key` is, as far as auth knows: it asked to join as them, or was paired by one of their own.
  ownerOf(key) {
    return this.ctx.storage.sql.exec("select user from devices where key = ?", key).toArray()[0]?.user;
  }

  /// A computer let in with a pairing code, as the device that paired it says, which asks nothing here itself:
  /// `{ key, user, name }`, from an admin's device or one of that user's own. Its user's other devices are told it joined.
  async pairedAs(request) {
    const signed = (await this.signed(request, await request.json().catch(() => refuse(400, "the body isn't JSON")))) || refuse(401, "Sign in first.");
    const { key, user, name } = signed.fields;
    if (!isKey(key) || !USER.test(user || "")) refuse(400, "a key is 64 hex characters and a user 16");
    if (typeof name !== "string" || !name.trim() || name.length > 64) refuse(400, "give the computer's name, up to 64 characters");
    if (!signed.admin && this.ownerOf(signed.by) !== user) refuse(403, "Only their own device, or an admin's, says a computer is theirs.");
    const had = this.ownerOf(key);
    if (had && had !== user && !signed.admin) refuse(409, "That key is someone else's device.");
    this.theirs(key, user);
    await this.push({ key, user }, { title: `${name.trim()} joined` });
    return {};
  }

  async wantsInBy(at) {
    const now = await this.ctx.storage.getAlarm();
    if (now == null || now > at) await this.ctx.storage.setAlarm(at);
  }

  async alarm() {
    this.expire();
    const sql = this.ctx.storage.sql;
    for (const r of sql.exec("select * from requests where approved is null and told is null and made <= ?", Date.now() - WANTS_IN).toArray()) await this.wantsIn(r);
    const [next] = sql.exec("select min(made) as at from requests where approved is null and told is null").toArray();
    if (next?.at != null) await this.ctx.storage.setAlarm(Math.max(next.at + WANTS_IN, Date.now() + 1000));
  }

  /// Pushes a note about the request `r` to its user's devices, but the one asking: tapping it opens Settings ›
  /// Devices. Never fails what it's said with.
  async push(r, note) {
    if (!this.env.PUSH) return;
    const known = this.ctx.storage.sql.exec("select key from devices where user = ? order by at desc limit ?", r.user, PUSHED_TO).toArray().map((d) => d.key);
    try {
      // No device registered as theirs (someone new, or who lost every one): an admin answers.
      const registered = (await this.env.PUSH.theirs(r.user, PUSHED_TO)).filter((key) => key !== r.key);
      const admins = registered.length || !this.env.RELAY ? [] : await this.env.RELAY.admins(PUSHED_TO);
      const to = [...new Set([...registered, ...known, ...admins])].filter((key) => key !== r.key).slice(0, PUSHED_TO);
      const plain = { id: hex16(), app: "settings", screen: "devices", ...note };
      if (to.length) await this.env.PUSH.send(to.map((key) => ({ to: key, plain })));
    } catch (err) {
      console.error(`pushing "${note.title}": ${err.stack || err}`);
    }
  }

  /// Each request as a device reads it: what its sign-in showed only to `entitled`, as an ID token is a bearer's, and
  /// never to a sign-in alone, which only asks.
  shown(rows, entitled = () => false) {
    const sql = this.ctx.storage.sql;
    return rows.map((r) => {
      const user_name = sql.exec("select name from user where id = ?", r.user).toArray()[0]?.name || "";
      const [signed] = entitled(r) ? sql.exec("select proof from proofs where key = ? and user = ?", r.key, r.user).toArray() : [];
      const signIn = signed && (r.by_link ? { by_link: JSON.parse(signed.proof) } : JSON.parse(signed.proof));
      return { key: r.key, seal: r.seal, name: r.name, user: r.user, user_name, by_link: !!r.by_link, ...(r.proof ? { proof: r.proof } : {}), ...(signIn ? { sign_in: signIn } : {}), ...(r.approved ? { approved: true } : {}), ...(r.root ? { root: r.root } : {}), ...(r.welcome ? { welcome: r.welcome } : {}), ...(r.place ? { place: r.place } : {}), ...(r.device ? { device: r.device } : {}), ...(r.platform ? { platform: r.platform } : {}), made: r.made, until: (r.approved || r.made) + REQUEST };
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
      return { requests: this.shown(sql.exec("select * from requests where approved is null order by made").toArray(), this.entitled(signed)) };
    }
    if (signed) this.theirs(signed.by, user);
    const theirs = user || (await this.user(request)) || refuse(401, "Sign in first.");
    return { requests: this.shown(sql.exec("select * from requests where user = ? and approved is null order by made", theirs).toArray(), signed ? () => true : undefined) };
  }

  /// Which requests a device of the network may answer and read the sign-ins of: every one, for an admin's; its own
  /// user's (`ownerOf`), for any other.
  entitled(signed) {
    const mine = this.ownerOf(signed.by);
    return (r) => signed.admin || r.user === mine;
  }

  async mayTouch(request, key) {
    this.expire();
    const sql = this.ctx.storage.sql;
    const signed = await this.signed(request);
    const [row] = sql.exec("select * from requests where key = ?", key).toArray();
    const ok = row && (signed || row.user === ((await this.user(request)) || refuse(401, "Sign in first.")));
    if (!ok) refuse(404, "No request waits with that key: it was approved, refused or waited too long.");
    return { row, signed };
  }

  async request(request, key) {
    const { row, signed } = await this.mayTouch(request, key);
    return this.shown([row], signed ? this.entitled(signed) : undefined)[0];
  }

  /// Forgets a request, refused; or one `approved`, marked so until it would have expired, for its device to see, with
  /// the network's `root` (hex) it was let into and its `welcome`, if the approving device said.
  async unask(request, key, approved, root, welcome) {
    const { row, signed } = await this.mayTouch(request, key);
    if (signed ? !this.entitled(signed)(row) : approved) refuse(403, "Only its user's own device, or an admin's, answers that request.");
    // Approved again (a retry, or a second approving device): marked once, never forgotten by it.
    if (approved) {
      if (!row.approved) {
        this.theirs(key, row.user);
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

  /// What a client's tokens are for, whatever it asks: the network's address (asking auth to be let in) only for its own apps, each a client whose every redirect comes back to one of them (`firstParty`);
  /// any other, a remote MCP client, gets `<address>/mcp`, the remote MCP server's, where it's a device of its user's,
  /// which nodes refuse.
  async audience(clientId) {
    const address = JSON.parse(this.env.RESOURCES || "[]")[0] || this.env.ADDRESS;
    const client = clientId && (await this.client(String(clientId)).catch(() => null));
    return client && this.firstParty(client) ? address : `${address}/mcp`;
  }

  /// Whether a client's every sign-in comes back to the device it was made on: the web client's page at the network's
  /// address (in the browser that signed in), an app's own link scheme or loopback. Not any site.
  signsInHere(client) {
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const origin = new URL(this.env.ADDRESS).origin;
    // The network's own address is on loopback too where it's tried (`one-dev throwaway`), so its pages are told apart first.
    const here = (u) => (URL.canParse(u) && new URL(u).origin === origin ? u === `${origin}/callback` : this.goes(u) === ON_DEVICE);
    return uris.length > 0 && uris.every(here);
  }

  /// Where `uri` goes, in words (`goesTo`).
  goes(uri) {
    return goesTo(uri, this.env.CLI || "one");
  }

  /// Whether a client is one of the network's own apps, by where its sign-ins come back: the web client's page, the
  /// native apps' `<cli>://signed-in/`, or Windows' loopback.
  firstParty(client) {
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const origin = new URL(this.env.ADDRESS).origin;
    const ours = [`${origin}/callback`, `${this.env.CLI || "one"}://signed-in/`, "http://127.0.0.1/callback"];
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
    const toSites = [].concat(body.redirect_uris || []).some((u) => this.goes(u) !== ON_DEVICE);
    if (name === ours || (name.startsWith(ours) && toSites)) refuse(400, `a client can't be called ${body.client_name}: that's the name of what it signs in to`);
  }

  /// What signing in to a client (its authorization's `query`) says: its name (the network's, for the web client, which
  /// is at its address), where its sign-in goes (its redirect_uri, if it's one of the client's, else its first), since
  /// the name is only what it calls itself, the user's, if they're signed in here already, and whether that sign-in was
  /// made after this authorization was asked for (Better Auth's signed `ba_iat`), which goes straight on.
  async about(request, query) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    const client = await this.client(query.get("client_id"));
    const redirect = query.get("redirect_uri");
    const origin = new URL(this.env.ADDRESS).origin;
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const web = uris.length > 0 && uris.every((u) => u === `${origin}/callback`);
    const goes = web ? null : this.goes(uris.includes(redirect) ? redirect : uris[0]);
    return { client: web ? this.auth.options.appName : client.name || client.clientId, web, goes, user: session?.user.name || null, fresh: !!session && query.has("ba_iat") && new Date(session.session.createdAt).getTime() >= Number(query.get("ba_iat")) };
  }

  fromOurPages(request) {
    if (request.headers.get("origin") !== new URL(this.env.ADDRESS).origin) refuse(403, "only the sign-in page asks that");
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
        await context.internalAdapter.createUser({ id: invite.user, name: invite.name, email: noEmail(invite.user), emailVerified: false });
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

  /// Joining with a provider: the invite's secret kept in a cookie for its answer, whose email must be the invite's.
  async joinWithProvider(request, { secret }) {
    this.fromOurPages(request);
    await this.openInvite(secret);
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": setCookie(this.env, "one-invite", secret, INVITE_COOKIE) } });
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
  // for whoever answers its request to check. `spoil` names another device instead, as a token they
  // must refuse.
  const tryProof = async (ctx, user) => {
    const join = JOIN.test(ctx.body?.join || "") ? ctx.body.join : null;
    const root = rootOf(auth.env);
    if (!join || !root) return;
    const origin = new URL(auth.env.ADDRESS).origin;
    const named = ctx.body.spoil === true ? "0".repeat(128) : join;
    const email = isNoEmail(user.email) ? {} : { email: user.email, email_verified: true };
    const payload = { iss: `${origin}/api/auth`, aud: "one-throwaway", sub: user.id, iat: Math.floor(Date.now() / 1000), nonce: await joinNonce(root, ...endpointOf(named)), ...email };
    const { token } = await auth.auth.api.signJWT({ body: { payload } });
    auth.keepProof(join, user.id, { id_token: token });
  };
  const first = async (adapter) => (await adapter.findUserById(id)) || (await adapter.createUser({ id, name, email: noEmail(id), emailVerified: false }));
  return {
    id: "throwaway",
    endpoints: {
      signInToThrowaway: createAuthEndpoint("/throwaway/sign-in", { method: "POST" }, async (ctx) => {
        const adapter = ctx.context.internalAdapter;
        const wanted = typeof ctx.body?.user === "string" ? ctx.body.user.trim().toLowerCase() : "";
        if (wanted) {
          // Invited again, a name is two users here: the one with a device let in is who joined, else the latest to open a link.
          const [row] = auth.ctx.storage.sql.exec("select id from user where lower(name) = ? order by exists (select 1 from requests where requests.user = user.id and approved is not null) desc, createdAt desc limit 1", wanted).toArray();
          // Someone named by an email invite signs in as that email's account, with no link.
          const [named] = row ? [] : auth.ctx.storage.sql.exec("select * from invites where lower(name) = ? and email is not null and used is null and until > ?", wanted, Date.now()).toArray();
          if (named) {
            auth.admitting = named.user;
            try {
              await adapter.createUser({ id: named.user, name: named.name, email: named.email, emailVerified: true });
            } finally {
              auth.admitting = null;
            }
            auth.ctx.storage.sql.exec("update invites set used = ? where id = ?", Date.now(), named.id);
            return signIn(ctx, await adapter.findUserById(named.user));
          }
          if (!row && wanted !== name.toLowerCase()) refuse(404, `Nobody called ${ctx.body.user} has joined.`);
          return signIn(ctx, row ? await adapter.findUserById(row.id) : await first(adapter));
        }
        return signIn(ctx, (await getSessionFromCtx(ctx))?.user || (await first(adapter)));
      }),
      joinThrowaway: createAuthEndpoint("/throwaway/join", { method: "POST" }, async (ctx) => {
        const invite = await auth.openInvite(ctx.body?.secret);
        await auth.admit(invite, { id: invite.user });
        return signIn(ctx, await ctx.context.internalAdapter.findUserById(invite.user));
      }),
    },
  };
}

/// Whether a sign-in here was made just now, as one more identity is only added from one.
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
