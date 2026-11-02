// The auth service: who you are. Better Auth, in a Durable Object: sign-in by passkey only, since only a passkey's PRF
// output opens a person's keys, or, on an install whose people sign in with their work Google account (`SIGN_IN`), by
// Google only, every time, limited to the install's domain (`JOIN_DOMAIN`); and an OAuth server (per MCP's authorization spec) that every client signs in through: the iPhone and Mac apps,
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
//
// Signing in with Google, anyone at the domain joins by signing in, and the vault opens their keys itself once Google
// says it's them. An invite there only says who someone with an email is in the install, for those who were in it
// before it signed people in this way.
//
// A try install's kernel (`SIGN_IN` is "try") signs in its first person (`TRY_USER`, `TRY_NAME`) with no passkey and
// no Google: its sign-in page goes straight on, and the vault opens their keys itself, as for Google. Someone it
// invites joins by opening their link, again with no passkey, and is then who that browser signs in as. It's for
// testing clients without a person: only `one try` turns it on (kernel::Settings::try_person), on a kernel that
// listens only on its own machine, and `kernel deploy` refuses one, so no real install's kernel ever does this.

import { DurableObject } from "cloudflare:workers";
import { oauthProvider } from "@better-auth/oauth-provider";
import { getAuthenticatorName, passkey } from "@better-auth/passkey";
import { betterAuth } from "better-auth";
import { APIError, createAuthEndpoint, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { setSessionCookie } from "better-auth/cookies";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { NO_PRF, codePage, invitePage, passkeysPage, signInPage, tryPage } from "./pages.js";
import { DurableObjectDialect } from "./sql.js";

const json = (v, status = 200) => Response.json(v, { status });
const hex16 = () => [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
/// Refuses, as Better Auth does, since some refusals are made inside its routes.
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

const refuse = (status, message) => {
  throw new APIError({ 400: "BAD_REQUEST", 403: "FORBIDDEN", 404: "NOT_FOUND" }[status], { message });
};
/// How long an invite link works.
const INVITE = 3 * 86_400_000;
/// How long a sign-in code works.
const CODE = 10 * 60_000;
/// How lately someone signed in with Google for a client to be added as them.
const FRESH = 5 * 60_000;
/// How long a Google sign-in started for a client's authorization goes on to it once it's back.
const STARTED = 15 * 60_000;
/// A sign-in code's letters: no vowels, so it spells no word, and none that look alike.
const LETTERS = "BCDFGHJKLMNPQRSTVWXZ";

/// The hash an invite is kept by, so the auth service's database holds no link that works.
async function inviteId(token) {
  if (!token) refuse(403, "Joining takes an invite link: ask an admin for one.");
  if (typeof token !== "string" || token.length < 32) refuse(404, "That invite link isn't whole: open the link you were sent as it is.");
  return hash(token);
}

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

export class Auth extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    ctx.storage.sql.exec("create table if not exists invites (id text primary key, name text not null, user text not null, until integer not null, used integer)");
    if (!ctx.storage.sql.exec("select name from pragma_table_info('invites') where name = 'email'").toArray().length) ctx.storage.sql.exec("alter table invites add column email text");
    ctx.storage.sql.exec("create table if not exists codes (code text primary key, state text not null unique, query text not null, until integer not null, back text)");
    ctx.storage.sql.exec("create table if not exists google_starts (id text primary key, query text not null, made integer not null)");
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
            // Only someone invited signs up (their passkey's registration makes them), as who the invite says they
            // are in the install, so nodes know them by the same id. Signing in with Google, anyone at the domain
            // does, as who an invite for their email says, if there is one.
            async before(user) {
              if (tried) return user.id === env.TRY_USER || (self.admitting && user.id === self.admitting) ? undefined : false;
              if (!google) return self.admitting && user.id === self.admitting ? undefined : false;
              const email = String(user.email || "").toLowerCase();
              if (!user.emailVerified || !email.endsWith(`@${env.JOIN_DOMAIN}`)) return false;
              const [invite] = sql.exec("select id, name, user from invites where email = ? and used is null order by until desc", email).toArray();
              if (!invite) return;
              sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
              return { data: { ...user, id: invite.user, name: invite.name } };
            },
          },
        },
      },
      hooks: {
        before: createAuthMiddleware(async (ctx) => {
          if (ctx.path === "/oauth2/authorize" && ctx.query) {
            if (!ctx.query.resource && resources[0]) ctx.query.resource = resources[0];
            // A client's keys are kept under its refresh token, so every client gets one, asked for or not: without
            // it, it'd be signed in and open nothing.
            const scopes = String(ctx.query.scope || "").split(" ").filter(Boolean);
            if (!scopes.includes("offline_access")) ctx.query.scope = [...scopes, "offline_access"].join(" ");
          }
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
        ...(tried ? [tryInstall(self, env.TRY_USER, env.TRY_NAME)] : []),
        ...(google || tried ? [] : [passkey({
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
        })]),
        oauthProvider({
          // Signed in or not, a client's authorization is the sign-in page: its passkey (or Google) is what gives the
          // client keys.
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
      if (path === "/api/auth/oauth2/register" && request.method === "POST") await this.notNamedAsUs(await request.clone().json().catch(() => ({})));
      if (path.startsWith("/api/auth/")) return await this.auth.handler(request);
      if (path.startsWith("/.well-known/oauth-authorization-server") || path.startsWith("/.well-known/openid-configuration")) {
        const which = path.includes("openid") ? "/.well-known/openid-configuration" : "/.well-known/oauth-authorization-server";
        const meta = await this.auth.handler(new Request(`${url.origin}/api/auth${which}`, { headers: request.headers }));
        return new Response(meta.body, { status: meta.status, headers: { "content-type": "application/json", "access-control-allow-origin": "*" } });
      }
      const name = this.auth.options.appName;
      // What its pages look like: the white label's name and accent.
      const look = { name, accent: this.env.ACCENT };
      if (path === "/.well-known/oauth-protected-resource" || path === "/.well-known/oauth-protected-resource/mcp") return this.protectedResource(path, name);
      const google = this.env.SIGN_IN === "google" ? this.env.JOIN_DOMAIN : null;
      const tried = this.env.SIGN_IN === "try";
      if (path === "/sign-in" && request.method === "GET") return html(tried ? tryPage({ ...look, person: this.env.TRY_NAME }) : signInPage({ ...look, google }));
      if (path === "/internal/invite" && request.method === "POST") return json(await this.invite(request, await request.json()));
      if (path === "/sign-in/about") return json(await this.about(request, url.searchParams));
      if (path === "/sign-in/google" && request.method === "POST" && google) return await this.startGoogle(request, await request.json());
      if (path === "/sign-in/code" && request.method === "POST") return json(await this.codeFor(await request.json()));
      if (path === "/sign-in/code" && request.method === "GET") return this.codeOpened(url.searchParams.get("c"), look, google);
      if (path === "/sign-in/code/done" && request.method === "GET") return html(codePage({ ...look, google, done: await this.codeDone(url.searchParams) }));
      if (path === "/sign-in/code/take" && request.method === "POST") return this.codeTaken(await request.json());
      if (path === "/sign-in/accept" && request.method === "POST") {
        const body = await request.json();
        return json(await (tried ? this.acceptTry(request, body) : google ? this.acceptGoogle(request, body) : this.accept(request, body)));
      }
      // Invite links are for an install whose people sign in with a passkey, or a try install's; passkeys only the former.
      if (google) return new Response("not found", { status: 404 });
      if (path === "/invite" && request.method === "GET") return html(invitePage({ ...look, tried }));
      if (path === "/invite/about" && request.method === "POST") return json(await this.inviteAbout(await request.json()));
      if (tried) return new Response("not found", { status: 404 });
      if (path === "/invite/keys" && request.method === "POST") return json(await this.inviteKeys(request, await request.json()));
      if (path === "/passkeys" && request.method === "GET") return html(passkeysPage(look));
      if (path === "/passkeys/add" && request.method === "POST") return json(await this.addPasskey(request, await request.json()));
      return new Response("not found", { status: 404 });
    } catch (err) {
      // Our own refusals have a numeric status; Better Auth's say theirs in statusCode.
      const status = typeof err.status === "number" ? err.status : err.statusCode;
      if (status >= 400 && status < 500) return json({ error: err.body?.error_description || err.body?.message || err.message }, status);
      console.error(err.stack || err);
      return json({ error: "the auth service failed" }, 500);
    }
  }

  /// What an MCP client asks first (RFC 9728): the address's MCP endpoint (or the address) is signed in to here.
  protectedResource(path, name) {
    const origin = new URL(this.env.ADDRESS).origin;
    const meta = {
      resource: path.endsWith("/mcp") ? `${origin}/mcp` : origin,
      resource_name: name,
      authorization_servers: [`${origin}/api/auth`],
      scopes_supported: ["openid", "profile", "offline_access"],
      bearer_methods_supported: ["header"],
    };
    return Response.json(meta, { headers: { "access-control-allow-origin": "*" } });
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
    const origin = new URL(this.env.ADDRESS).origin;
    if (request.headers.get("origin") !== origin) refuse(403, "only the sign-in page starts that");
    const query = new URLSearchParams(typeof oauth_query === "string" ? oauth_query : "");
    await this.client(query.get("client_id"));
    const secret = btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32)))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
    const sql = this.ctx.storage.sql;
    sql.exec("delete from google_starts where made <= ?", Date.now() - STARTED);
    sql.exec("insert into google_starts (id, query, made) values (?, ?, ?)", await hash(secret), await hash(query.toString()), Date.now());
    const secure = origin.startsWith("https:");
    const cookie = `${secure ? "__Host-" : ""}one-google-start=${secret}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${STARTED / 1000}${secure ? "; Secure" : ""}`;
    return new Response("{}", { headers: { "content-type": "application/json", "set-cookie": cookie } });
  }

  /// The Google sign-in this browser started for the authorization `query`, if it's back from it just now: → its id.
  async started(request, session, query) {
    if (!session || !fresh(session)) return null;
    const secure = this.env.ADDRESS.startsWith("https:");
    const name = `${secure ? "__Host-" : ""}one-google-start=`;
    const secret = (request.headers.get("cookie") || "").split(/;\s*/).find((c) => c.startsWith(name))?.slice(name.length);
    if (!secret) return null;
    const id = await hash(secret);
    const [row] = this.ctx.storage.sql.exec("select query, made from google_starts where id = ? and made > ?", id, Date.now() - STARTED).toArray();
    const signedIn = new Date(session.session.createdAt).getTime();
    return row && row.query === (await hash(query.toString())) && row.made <= signedIn ? id : null;
  }

  // Signing in with a code, for a client with no browser of its own (the CLI on a server): it makes an authorization
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
    return this.consent(request, user, oauth_query, async (client, code) => {
      // A passkey that opens no keys gets the client nothing: its code, never handed over, is never used.
      try {
        await this.vault("grant", { user: user.id, from: prf, to: code, name: client.name || client.clientId, client: client.clientId });
      } catch (err) {
        if (err.status === 403) refuse(403, "This passkey doesn't unlock your data yet: add it on your Passkeys page with one that does.");
        throw err;
      }
    });
  }

  /// Signing in to a client with Google, just now, started for this authorization (a client's code is swapped for
  /// tokens only while that sign-in lasts, so it isn't ended here; a later client asks Google again): the client gets
  /// a code, and the vault opens the person's keys for it.
  async acceptGoogle(request, { oauth_query }) {
    const signedIn = await this.session(request);
    const started = await this.started(request, signedIn, new URLSearchParams(typeof oauth_query === "string" ? oauth_query : ""));
    if (!started) refuse(403, "Sign in with Google again.");
    this.ctx.storage.sql.exec("delete from google_starts where id = ?", started);
    const { user } = signedIn;
    return this.consent(request, user, oauth_query, (client, code) =>
      this.vault("own", { user: user.id, to: code, name: client.name || client.clientId, client: client.clientId }));
  }

  /// Signing in to a client on a try install: whoever is signed in here, with nothing to show, and the vault opens their keys.
  async acceptTry(request, { oauth_query }) {
    const { user } = await this.session(request);
    return this.consent(request, user, oauth_query, (client, code) =>
      this.vault("own", { user: user.id, to: code, name: client.name || client.clientId, client: client.clientId }));
  }

  /// Lets a client's authorization (`oauth_query`) through as `user`, with `keys(client, code)` giving its code a copy of
  /// their keys before it's handed over: → where the client goes on to.
  async consent(request, user, oauth_query, keys) {
    const query = new URLSearchParams(oauth_query || "");
    const client = await this.client(query.get("client_id"));
    const headers = new Headers(request.headers);
    headers.set("content-type", "application/json");
    headers.set("accept", "application/json");
    const resp = await this.auth.handler(new Request(new URL("/api/auth/oauth2/consent", request.url), { method: "POST", headers, body: JSON.stringify({ accept: true, oauth_query }) }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) throw Object.assign(new Error(out.error_description || out.message || "signing in didn't work"), { status: resp.status });
    // Every authorization asks for the passkey (or Google), since that's what gives a client keys: none is kept for
    // next time.
    this.ctx.storage.sql.exec("delete from oauthConsent where clientId = ? and userId = ?", client.clientId, user.id);
    const code = new URL(out.url).searchParams.get("code");
    if (!code) throw Object.assign(new Error("signing in gave no code"), { status: 500 });
    await keys(client, code);
    return { url: out.url };
  }

  /// An invite for `name` (only a label), as `user` (their id in the install; a new one if not given): → the secret its
  /// link carries, kept here only as a hash, and until when it works. The kernel's own tools ask, with its secret.
  /// Signing in with Google, it's for `email` instead: who they are when they first sign in, with no link.
  async invite(request, { name, user, email }) {
    if (request.headers.get("x-kernel-secret") !== this.env.KERNEL_SECRET) refuse(403, "only the kernel asks that");
    if (typeof name !== "string" || !name.trim()) refuse(400, "give the person's name");
    if (user != null && !/^[0-9a-f]{16}$/.test(user)) refuse(400, "a user id is 16 hex characters");
    const google = this.env.SIGN_IN === "google";
    if (google && !String(email || "").toLowerCase().endsWith(`@${this.env.JOIN_DOMAIN}`)) refuse(400, `people here sign in with Google: give their email at ${this.env.JOIN_DOMAIN}`);
    const token = btoa(String.fromCharCode(...crypto.getRandomValues(new Uint8Array(32)))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
    const until = Date.now() + INVITE;
    this.ctx.storage.sql.exec("delete from invites where until <= ?", Date.now());
    this.ctx.storage.sql.exec("insert into invites (id, name, user, until, email) values (?, ?, ?, ?, ?)", await inviteId(token), name.trim(), user ?? hex16(), until, google ? email.toLowerCase() : null);
    return google ? { until } : { token, until };
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
        const invite = await auth.openInvite(ctx.body?.token);
        await auth.admit(invite, { id: invite.user });
        auth.ctx.storage.sql.exec("update invites set used = ? where id = ?", Date.now(), invite.id);
        return signIn(ctx, await ctx.context.internalAdapter.findUserById(invite.user));
      }),
    },
  };
}

/// Whether a sign-in here was made just now: Google's, which a client is added with.
function fresh(session) {
  return Date.now() - new Date(session.session.createdAt).getTime() < FRESH;
}

function html(text) {
  return new Response(text, { headers: { "content-type": "text/html; charset=utf-8", "content-security-policy": "default-src 'self'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; frame-ancestors 'none'" } });
}

export default {
  async fetch(request, env) {
    return env.AUTH.get(env.AUTH.idFromName("auth")).fetch(request);
  },
};
