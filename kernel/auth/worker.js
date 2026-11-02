// The auth service: who you are. Better Auth, in a Durable Object: sign-in by passkey only, since only a passkey's PRF
// output opens a person's keys, and an OAuth server (per MCP's authorization spec) that every client signs in through: the iPhone and Mac apps,
// and MCP clients like Claude Code and ChatGPT. Its access tokens are JWTs signed with Ed25519, its keys at
// /api/auth/jwks, so nodes and the vault check them offline.
//
// Keys open only with a passkey. Every client is its person, with everything they can use, so the only question is
// who: signing in to a client with a passkey, its PRF output opens the person's keys at the vault, which seals a copy
// for the client under its authorization code; each time the
// client swaps a code or refresh token for new tokens, the vault moves the copy to the new refresh token and opens a
// session under the new access token. Better Auth keeps refresh tokens only as hashes, so its database opens nothing.
//
// Nobody signs up uninvited, the install's first person too: `one kernel invite` makes a link that works once, for a
// few days, and opening it lets its person make their first passkey, which makes their keys. A link for someone who
// lost every passkey carries their keys, opened with the recovery key, for their new one.

import { DurableObject } from "cloudflare:workers";
import { oauthProvider } from "@better-auth/oauth-provider";
import { getAuthenticatorName, passkey } from "@better-auth/passkey";
import { betterAuth } from "better-auth";
import { APIError, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { NO_PRF, invitePage, passkeysPage, signInPage } from "./pages.js";
import { DurableObjectDialect } from "./sql.js";

const json = (v, status = 200) => Response.json(v, { status });
const hex16 = () => [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
/// Refuses, as Better Auth does, since some refusals are made inside its routes.
const refuse = (status, message) => {
  throw new APIError({ 400: "BAD_REQUEST", 403: "FORBIDDEN", 404: "NOT_FOUND" }[status], { message });
};
/// How long an invite link works.
const INVITE = 3 * 86_400_000;

/// The hash an invite is kept by, so the auth service's database holds no link that works.
async function inviteId(token) {
  if (!token) refuse(403, "Joining takes an invite link: ask whoever runs this install for one.");
  if (typeof token !== "string" || token.length < 32) refuse(404, "That invite link isn't whole: open the link you were sent as it is.");
  return [...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token)))].map((b) => b.toString(16).padStart(2, "0")).join("");
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

export class Auth extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.storage.sql.exec("create table if not exists invites (id text primary key, name text not null, user text not null, until integer not null, used integer)");
    this.ready = ctx.blockConcurrencyWhile(async () => {
      const options = this.options();
      const { runMigrations } = await getMigrations(options);
      await runMigrations();
      this.auth = betterAuth(options);
    });
  }

  /// Asks the vault, as only the auth service may.
  async vault(path, body) {
    const resp = await this.env.VAULT.fetch(new Request(`https://vault/internal/${path}`, {
      method: "POST",
      headers: { "content-type": "application/json", "x-kernel-secret": this.env.KERNEL_SECRET },
      body: JSON.stringify(body),
    }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error || "the vault failed"), { status: resp.status });
    return out;
  }

  options() {
    const env = this.env;
    const url = new URL(env.ADDRESS);
    const resources = JSON.parse(env.RESOURCES || "[]");
    const sql = this.ctx.storage.sql;
    const self = this;
    return {
      appName: env.NAME || "The One App",
      baseURL: url.origin,
      basePath: "/api/auth",
      secret: env.AUTH_SECRET,
      database: { dialect: new DurableObjectDialect(sql), type: "sqlite", transaction: false },
      trustedOrigins: [url.origin],
      telemetry: { enabled: false },
      advanced: { database: { generateId: hex16 } },
      databaseHooks: {
        user: {
          create: {
            // Only someone invited signs up (their passkey's registration makes them), as who the invite says they
            // are in the install, so nodes know them by the same id.
            async before(user) {
              if (!self.admitting || user.id !== self.admitting) return false;
            },
          },
        },
      },
      hooks: {
        before: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/authorize" && ctx.query && !ctx.query.resource && resources[0]) ctx.query.resource = resources[0];
          if (ctx.path === "/oauth2/token" && ctx.body && !ctx.body.resource && resources[0]) ctx.body.resource = resources[0];
        }),
        after: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/token") await self.tokensIssued(ctx.body || {}, ctx.context.returned);
          if (ctx.path === "/oauth2/revoke" && ctx.body?.token) await self.vault("forget", { secret: ctx.body.token }).catch(() => {});
          // A passkey removed opens nothing again.
          if (ctx.path === "/passkey/delete-passkey" && ctx.context.session) {
            const user = ctx.context.session.user.id;
            const keep = sql.exec("select credentialID from passkey where userId = ?", user).toArray().map((r) => r.credentialID);
            await self.vault("forget-passkeys", { user, keep });
          }
        }),
      },
      plugins: [
        jwt({ jwks: { keyPairConfig: { alg: "EdDSA", crv: "Ed25519" } }, jwt: { issuer: `${url.origin}/api/auth`, expirationTime: "1h" } }),
        passkey({
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
            // A passkey without PRF opens nothing, so none is kept. The page says only whether it has PRF, never its output.
            async afterVerification({ ctx, verification, user, clientData, context }) {
              if (clientData.clientExtensionResults?.prf?.enabled !== true) throw new APIError("BAD_REQUEST", { message: NO_PRF });
              if (!(await getSessionFromCtx(ctx))) await self.admit(await self.openInvite(context), user);
              return { name: passkeyName(verification.registrationInfo?.aaguid, clientData.response?.transports || [], ctx.headers?.get("user-agent") || "") };
            },
          },
        }),
        oauthProvider({
          // Signed in or not, a client's authorization is the sign-in page: its passkey is what gives the client keys.
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
        }),
      ],
    };
  }

  /// After the token endpoint answers: moves the client's keys to its new refresh token and opens a session under
  /// its new access token.
  async tokensIssued(body, returned) {
    const tokens = returned instanceof Response ? await returned.clone().json().catch(() => null) : returned;
    if (!tokens?.access_token || !tokens.refresh_token) return;
    const from = body.grant_type === "authorization_code" ? body.code : body.grant_type === "refresh_token" ? body.refresh_token : null;
    if (!from) return;
    const until = Date.now() + (tokens.expires_in || 3600) * 1000;
    await this.vault("rekey", { from, to: tokens.refresh_token, token: tokens.access_token, until });
  }

  async fetch(request) {
    await this.ready;
    const url = new URL(request.url);
    const path = url.pathname;
    try {
      if (path.startsWith("/api/auth/")) return await this.auth.handler(request);
      if (path.startsWith("/.well-known/oauth-authorization-server") || path.startsWith("/.well-known/openid-configuration")) {
        const which = path.includes("openid") ? "/.well-known/openid-configuration" : "/.well-known/oauth-authorization-server";
        const meta = await this.auth.handler(new Request(`${url.origin}/api/auth${which}`, { headers: request.headers }));
        return new Response(meta.body, { status: meta.status, headers: { "content-type": "application/json", "access-control-allow-origin": "*" } });
      }
      const name = this.auth.options.appName;
      if (path === "/sign-in" && request.method === "GET") return html(signInPage({ name }));
      if (path === "/invite" && request.method === "GET") return html(invitePage({ name }));
      if (path === "/invite/about" && request.method === "POST") return json(await this.inviteAbout(await request.json()));
      if (path === "/invite/keys" && request.method === "POST") return json(await this.inviteKeys(request, await request.json()));
      if (path === "/passkeys" && request.method === "GET") return html(passkeysPage({ name }));
      if (path === "/passkeys/add" && request.method === "POST") return json(await this.addPasskey(request, await request.json()));
      if (path === "/sign-in/about") return json(await this.about(request, url.searchParams.get("client_id")));
      if (path === "/sign-in/accept" && request.method === "POST") return json(await this.accept(request, await request.json()));
      if (path === "/internal/invite" && request.method === "POST") return json(await this.invite(request, await request.json()));
      return new Response("not found", { status: 404 });
    } catch (err) {
      // Our own refusals have a numeric status; Better Auth's say theirs in statusCode.
      const status = typeof err.status === "number" ? err.status : err.statusCode;
      if (status >= 400 && status < 500) return json({ error: err.body?.error_description || err.body?.message || err.message }, status);
      console.error(err.stack || err);
      return json({ error: "the auth service failed" }, 500);
    }
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

  /// What signing in to a client says: its name (the install's, for the web client, which is at its address), and
  /// the person's, if they're signed in here already.
  async about(request, clientId) {
    const session = await this.auth.api.getSession({ headers: request.headers });
    const client = await this.client(clientId);
    const origin = new URL(this.env.ADDRESS).origin;
    const uris = typeof client.redirectUris === "string" ? JSON.parse(client.redirectUris) : client.redirectUris || [];
    const web = uris.length > 0 && uris.every((u) => URL.canParse(u) && new URL(u).origin === origin);
    return { client: web ? this.auth.options.appName : client.name || client.clientId, web, person: session?.user.name || null };
  }

  /// One of the person's passkeys, by its credential id: → its row, or a refusal.
  passkeyOf(user, credential) {
    const [row] = this.ctx.storage.sql.exec("select id, name, credentialID from passkey where userId = ? and credentialID = ?", user, String(credential || "")).toArray();
    return row || refuse(403, "That passkey isn't yours.");
  }

  /// The invite a link's secret opens, while it works.
  async openInvite(token) {
    const [invite] = this.ctx.storage.sql.exec("select * from invites where id = ?", await inviteId(token)).toArray();
    if (!invite) refuse(404, "That invite link doesn't work. Ask for a new one.");
    if (invite.used) refuse(404, "That invite link has been used. Ask for a new one.");
    if (invite.until <= Date.now()) refuse(404, "That invite link has expired. Ask for a new one.");
    return invite;
  }

  /// Makes the invite's person, as their first passkey is registered: the database hook lets only them in.
  async admit(invite, user) {
    if (user.id !== invite.user) refuse(400, "That passkey was made for someone else.");
    const context = await this.auth.$context;
    if (await context.internalAdapter.findUserById(invite.user)) return;
    this.admitting = invite.user;
    try {
      await context.internalAdapter.createUser({ id: invite.user, name: invite.name, email: `${invite.user}@people.invalid`, emailVerified: false });
    } finally {
      this.admitting = null;
    }
  }

  /// What an invite page shows: whose it is.
  async inviteAbout({ token }) {
    const invite = await this.openInvite(token);
    return { person: invite.name, until: invite.until };
  }

  /// The invite's person's first passkey (its PRF output) gets their keys: new ones, or, for someone who lost every
  /// passkey, those the link carries. The link is spent.
  async inviteKeys(request, { token, prf, credential }) {
    const { user } = await this.session(request);
    const invite = await this.openInvite(token);
    if (invite.user !== user.id) refuse(403, "That invite is someone else's.");
    const passkey = this.passkeyOf(user.id, credential);
    await this.vault("passkey", { user: user.id, secret: prf, name: passkey.name || "Passkey", client: passkey.credentialID, from: token });
    this.ctx.storage.sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
    return {};
  }

  /// Another passkey for someone signed in: the vault seals their keys under it too, opened with one they have.
  async addPasskey(request, { from, from_credential, prf, credential }) {
    const { user } = await this.session(request);
    this.passkeyOf(user.id, from_credential);
    const passkey = this.passkeyOf(user.id, credential);
    await this.vault("passkey", { user: user.id, secret: prf, name: passkey.name || "Passkey", client: passkey.credentialID, from });
    return {};
  }

  /// Signing in to a client with a passkey: the client gets a code, and a copy of the person's keys sealed under it.
  async accept(request, { oauth_query, prf, credential }) {
    const { user } = await this.session(request);
    this.passkeyOf(user.id, credential);
    const query = new URLSearchParams(oauth_query || "");
    const client = await this.client(query.get("client_id"));
    const headers = new Headers(request.headers);
    headers.set("content-type", "application/json");
    headers.set("accept", "application/json");
    const resp = await this.auth.handler(new Request(new URL("/api/auth/oauth2/consent", request.url), { method: "POST", headers, body: JSON.stringify({ accept: true, oauth_query }) }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error_description || out.message || "signing in didn't work"), { status: resp.status });
    // Every authorization asks for the passkey, since that's what gives a client keys: none is kept for next time.
    this.ctx.storage.sql.exec("delete from oauthConsent where clientId = ? and userId = ?", client.clientId, user.id);
    const code = new URL(out.url).searchParams.get("code");
    if (!code) throw Object.assign(new Error("signing in gave no code"), { status: 500 });
    // A passkey that opens no keys gets the client nothing: its code, never handed over, is never used.
    try {
      await this.vault("grant", { user: user.id, from: prf, to: code, name: client.name || client.clientId, client: client.clientId });
    } catch (err) {
      if (err.status === 403) refuse(403, "This passkey doesn't open your keys yet: add it on your Passkeys page with one that does.");
      throw err;
    }
    return { url: out.url };
  }

  /// An invite for `name` (only a label), as `user` (their id in the install; a new one if not given): → the secret its
  /// link carries, kept here only as a hash, and until when it works. The kernel's own tools ask, with its secret.
  async invite(request, { name, user }) {
    if (request.headers.get("x-kernel-secret") !== this.env.KERNEL_SECRET) refuse(403, "only the kernel asks that");
    if (typeof name !== "string" || !name.trim()) refuse(400, "give the person's name");
    if (user != null && !/^[0-9a-f]{16}$/.test(user)) refuse(400, "a user id is 16 hex characters");
    const token = btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32)))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
    const until = Date.now() + INVITE;
    this.ctx.storage.sql.exec("delete from invites where until <= ?", Date.now());
    this.ctx.storage.sql.exec("insert into invites (id, name, user, until) values (?, ?, ?, ?)", await inviteId(token), name.trim(), user ?? hex16(), until);
    return { token, until };
  }
}

function html(text) {
  return new Response(text, { headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-ancestors 'none'" } });
}

export default {
  async fetch(request, env) {
    return env.AUTH.get(env.AUTH.idFromName("auth")).fetch(request);
  },
};
