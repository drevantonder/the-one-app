// The auth service: who you are. Better Auth, in a Durable Object: sign-in by passkey, email and password, or Google,
// and an OAuth server (per MCP's authorization spec) that every client signs in through: the iPhone and Mac apps,
// and MCP clients like Claude Code and ChatGPT. Its access tokens are JWTs signed with Ed25519, its keys at
// /api/auth/jwks, so nodes and the vault check them offline.
//
// Keys open only with a passkey. A client is added at its consent screen: the person's passkey (its PRF output)
// opens their keys at the vault, which seals a copy for the client under its authorization code; each time the
// client swaps a code or refresh token for new tokens, the vault moves the copy to the new refresh token and opens a
// session under the new access token. Better Auth keeps refresh tokens only as hashes, so its database opens nothing.

import { DurableObject } from "cloudflare:workers";
import { oauthProvider } from "@better-auth/oauth-provider";
import { passkey } from "@better-auth/passkey";
import { betterAuth } from "better-auth";
import { createAuthMiddleware } from "better-auth/api";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { consentPage, signInPage } from "./pages.js";
import { DurableObjectDialect } from "./sql.js";

const json = (v, status = 200) => Response.json(v, { status });
const hex16 = () => [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");

export class Auth extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.storage.sql.exec("create table if not exists invites (email text primary key, at integer not null)");
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
    const url = new URL(env.AUTH_URL);
    const resources = JSON.parse(env.RESOURCES || "[]");
    const sql = this.ctx.storage.sql;
    const self = this;
    return {
      appName: env.NAME || "The One App",
      baseURL: url.origin,
      basePath: "/api/auth",
      secret: env.AUTH_SECRET,
      database: { dialect: new DurableObjectDialect(sql), type: "sqlite", transaction: false },
      emailAndPassword: { enabled: true },
      socialProviders: env.GOOGLE ? { google: JSON.parse(env.GOOGLE) } : {},
      trustedOrigins: [url.origin],
      telemetry: { enabled: false },
      advanced: { database: { generateId: hex16 } },
      databaseHooks: {
        user: {
          create: {
            // Only someone invited signs up, the install's first person too (one kernel invite), so nobody who finds
            // the sign-in page first takes the install.
            async before(user) {
              const invited = sql.exec("select email from invites where email = ?", user.email.toLowerCase()).toArray().length;
              if (!invited) return false;
              sql.exec("delete from invites where email = ?", user.email.toLowerCase());
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
        passkey({ rpID: url.hostname, rpName: env.NAME || "The One App", origin: url.origin }),
        oauthProvider({
          loginPage: "/sign-in",
          consentPage: "/consent",
          scopes: ["openid", "profile", "email", "offline_access"],
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
      if (path === "/sign-in" && request.method === "GET") return html(signInPage({ name: this.auth.options.appName, google: !!this.env.GOOGLE, first: this.people() === 0 }));
      if (path === "/consent" && request.method === "GET") return html(consentPage({ name: this.auth.options.appName }));
      if (path === "/consent/about") return json(await this.about(request, url.searchParams.get("client_id")));
      if (path === "/consent/scopes" && request.method === "POST") return json(await this.scopes(request, await request.json()));
      if (path === "/consent/accept" && request.method === "POST") return json(await this.accept(request, await request.json()));
      if (path === "/internal/invite" && request.method === "POST") return json(this.invite(request, await request.json()));
      return new Response("not found", { status: 404 });
    } catch (err) {
      // Our own refusals have a numeric status; Better Auth's say theirs in statusCode.
      const status = typeof err.status === "number" ? err.status : err.statusCode;
      if (status >= 400 && status < 500) return json({ error: err.body?.error_description || err.body?.message || err.message }, status);
      console.error(err.stack || err);
      return json({ error: "the auth service failed" }, 500);
    }
  }

  people() {
    return this.ctx.storage.sql.exec("select count(*) n from user").one().n;
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

  /// What the consent screen shows before the passkey: the client's name, and whether the person has a passkey yet.
  async about(request, clientId) {
    const { user } = await this.session(request);
    const client = await this.client(clientId);
    const passkeys = this.ctx.storage.sql.exec("select count(*) n from passkey where userId = ?", user.id).one().n;
    return { client: client.name || client.clientId, person: user.name || user.email, passkeys };
  }

  /// The scopes the person's passkey opens. Their first passkey makes their keys.
  async scopes(request, { prf, credential }) {
    const { user } = await this.session(request);
    const mine = this.ctx.storage.sql.exec("select credentialID from passkey where userId = ? and credentialID = ?", user.id, String(credential || "")).toArray();
    if (!mine.length) throw Object.assign(new Error("that passkey isn't yours"), { status: 403 });
    try {
      return await this.vault("scopes", { user: user.id, secret: prf });
    } catch (err) {
      if (err.status !== 403) throw err;
      // No copy opens with it: if the person has no keys yet, this passkey makes them; otherwise it's a new passkey
      // that one they had already must add.
      await this.vault("passkey", { user: user.id, secret: prf, name: "Passkey", client: credential });
      return await this.vault("scopes", { user: user.id, secret: prf });
    }
  }

  /// Consent: the client gets a code, and a copy of the person's keys sealed under it, for the scopes they chose.
  async accept(request, { oauth_query, prf, scopes, accept }) {
    const { user } = await this.session(request);
    const query = new URLSearchParams(oauth_query || "");
    const client = await this.client(query.get("client_id"));
    if (accept) await this.vault("scopes", { user: user.id, secret: prf });
    const headers = new Headers(request.headers);
    headers.set("content-type", "application/json");
    headers.set("accept", "application/json");
    const resp = await this.auth.handler(new Request(new URL("/api/auth/oauth2/consent", request.url), { method: "POST", headers, body: JSON.stringify({ accept: !!accept, oauth_query }) }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error_description || out.message || "the consent failed"), { status: resp.status });
    const redirect = out.url;
    // Every authorization asks for the passkey at its consent screen, since that's what gives a client keys: no
    // consent is kept for next time.
    this.ctx.storage.sql.exec("delete from oauthConsent where clientId = ? and userId = ?", client.clientId, user.id);
    if (accept) {
      const code = new URL(redirect).searchParams.get("code");
      if (!code) throw Object.assign(new Error("the consent gave no code"), { status: 500 });
      await this.vault("grant", { user: user.id, from: prf, to: code, name: client.name || client.clientId, client: client.clientId, scopes });
    }
    return { url: redirect };
  }

  /// Lets someone sign up: the kernel's own tools ask, with its secret.
  invite(request, { email }) {
    if (request.headers.get("x-kernel-secret") !== this.env.KERNEL_SECRET) throw Object.assign(new Error("only the kernel asks that"), { status: 403 });
    if (typeof email !== "string" || !email.includes("@")) throw Object.assign(new Error("not an email address"), { status: 400 });
    this.ctx.storage.sql.exec("insert or replace into invites (email, at) values (?, ?)", email.toLowerCase(), Date.now());
    return {};
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
