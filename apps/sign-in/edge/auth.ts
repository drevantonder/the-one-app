// Who signs in to a network, and what they let act as them: Better Auth, kept in this Worker's own database. A
// person signs in with a passkey and nothing else. An assistant becomes theirs when they answer that one question
// for it, and a new device when they also say the code it shows matches the one they're shown, as Better Auth's
// device authorization has it, or when a device or assistant of theirs lets it in by that code. Each is then a line
// in this app's own artifact, `devices/<device>`, saying whose it is: that's where a server reads it, so no call
// asks anything here. What a person goes by is a line there too, `people/<name>`: the name they chose as they
// joined, and their email where how they signed in has one, which a server tells apps of by `people`.
import { env } from "cloudflare:workers";
import { betterAuth } from "better-auth";
import { APIError, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { getMigrations } from "better-auth/db/migration";
import { jwt } from "better-auth/plugins";
import { cimd } from "@better-auth/cimd";
import { mcp } from "@better-auth/mcp";
import { DEVICE_CODE_GRANT_TYPE, getIssuer, oauthDeviceAuthorization, verifyOAuthQueryParams } from "@better-auth/oauth-provider";
import { passkey } from "@better-auth/passkey";
import Bowser from "bowser";
import type { BetterAuthPlugin, DBAdapter, User as AuthUser } from "better-auth";

declare global {
  namespace Cloudflare {
    /** Its bindings: its database and page, and what the edge gives every app's part (`core/edge/edge.js`). */
    interface Env {
      DB: D1Database;
      ASSETS: Fetcher;
      /** Which deployed version of this Worker this is (`version_metadata` in `wrangler.jsonc`). */
      VERSION: WorkerVersionMetadata;
      OWN: {
        write(name: string, data: string): Promise<boolean>;
        update(name: string, change: (was: string | null) => string): Promise<string>;
        remove(name: string): Promise<void>;
      };
      NETWORK: {
        verifiedKey(url: string, headers: Record<string, string>, body: string): Promise<string | null>;
        servers(of: string): Promise<string[]>;
        owner(): Promise<string | null>;
      };
      RELAY: {
        notify(message: string): Promise<void>;
        presence(keys: string[]): Promise<Record<string, { connected: boolean; last_seen?: number } | null>>;
      };
      /** The apps that use this network's passkeys and open its links (`vars` in `wrangler.jsonc`). */
      APPLE_APPS: string[];
      ANDROID_APPS: { package_name: string; sha256_cert_fingerprints: string[] }[];
      APP_LINKS: string[];
    }
  }
}

/** What of Better Auth's database adapter is used here, whatever options it was made with. */
type Adapter = Pick<DBAdapter, "findMany" | "findOne" | "create" | "update" | "delete" | "deleteMany" | "incrementOne">;

/** Someone, as Better Auth keeps them here: `users/<name>`, and the name they go by. */
type User = AuthUser & { called?: string | null };

/** A device or an assistant someone let act as them. */
interface Device {
  id: string;
  userId: string;
  kind: "app" | "assistant";
  name: string;
  createdAt: Date;
}

/** A device's code, as Better Auth's device authorization keeps it, with what the device calls itself. */
interface DeviceCode {
  id: string;
  deviceCode: string;
  userCode: string;
  clientId: string;
  userId?: string | null;
  status: "pending" | "approved" | "denied";
  expiresAt: Date;
  name: string;
}

/** An invitation: the hash of its link, whose it is, and when it ends. */
interface Invite {
  id: string;
  userId: string;
  expiresAt: Date;
}

/** How long ago someone may have answered the passkey question for it to count as them saying yes now, in seconds. */
const FRESH = 5 * 60;
/**
 * How long a browser's session, once checked against the database, is believed without checking again, in seconds:
 * Better Auth's cookie cache, and an app's page's session (`worker.ts`). A session signed out or removed is still
 * let in here that long at most. The servers stop answering it at once, as its device is nobody's from then on
 * (`browserOut`), so all it still reaches is `/apps`, and app files' code.
 */
export const BELIEVED = 60;
/** How long an invitation lasts, in milliseconds. */
const INVITED_FOR = 7 * 24 * 60 * 60 * 1000;
/**
 * How many codes no device shows a person may try to let in by, and over how long, in seconds. A code is one of
 * 32⁸, so this only stops guessing at speed. It's counted in the table Better Auth keeps its own limits in.
 */
const WRONG_CODES = { max: 5, window: 15 * 60 };

const hex = (bytes: ArrayBuffer | Uint8Array) => Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("");
const sha256 = async (text: string) => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));

/** What a browser is called, from what it says of itself: "Chrome on Linux". */
function browser(agent?: string | null) {
  const { browser, os } = Bowser.parse(agent || " ");
  return [browser.name, os.name].filter(Boolean).join(" on ") || "A browser";
}

/** A user as relation tuples name them. Their name here is the part after `users/`. */
export const who = (user: { name: string }) => `users/${user.name}`;

/**
 * The device an assistant somewhere else is, to a server: one for each program a person let in, named by both.
 * It's the size of a key and nobody's key, so it never names a device that connects by one.
 */
export const assistant = (client: string, user: string) => sha256(`${client}\n${user}`);

/**
 * The device a browser that's signed in here is, to a server: one for each of its sessions, the size of a key and
 * nobody's key, as an assistant's is.
 */
export const browserDevice = (session: string) => sha256(`browser\n${session}`);

/**
 * What signs this Worker's cookies and tokens, made on first use and kept beside what it protects, and the version of
 * this Worker that last made its tables (`createAuth`), in one round trip to the database.
 */
async function stored() {
  const generated = hex(crypto.getRandomValues(new Uint8Array(32)));
  const [, , , secret, migrated] = await env.DB.batch<{ secret?: string; version?: string }>([
    env.DB.prepare("create table if not exists secret (secret text not null)"),
    env.DB.prepare("insert into secret select ? where not exists (select 1 from secret)").bind(generated),
    env.DB.prepare("create table if not exists migrated (version text not null)"),
    env.DB.prepare("select secret from secret"),
    env.DB.prepare("select version from migrated"),
  ]);
  return { secret: secret.results[0].secret!, migrated: migrated.results[0]?.version };
}

/** Tells every server that whose a device is has changed, so each asks again. */
const invalidate = () => env.RELAY.notify(JSON.stringify({ jsonrpc: "2.0", method: "notifications/one/invalidate", params: { hooks: ["device"] } }));

/** Makes `device` the user's, under `name`, from now on. */
async function place(adapter: Adapter, device: string, kind: Device["kind"], name: string, user: { id: string; name: string }) {
  const [was] = await adapter.findMany<Device>({ model: "device", where: [{ field: "id", value: device }] });
  if (was) await remove(adapter, was);
  await adapter.create({ model: "device", data: { id: device, userId: user.id, kind, name, createdAt: new Date() }, forceAllowId: true });
  await env.OWN.write(`devices/${device}`, who(user));
}

/**
 * Writes what `user` goes by where a server reads it: the name they chose, and their email where it's one they
 * proved. Better Auth keeps an email for everyone, made up for whoever signs in only with a passkey, which isn't one.
 */
async function goesBy(user: User) {
  const person = { called: user.called ?? null, email: user.emailVerified ? user.email : null };
  await env.OWN.update(`people/${user.name}`, () => JSON.stringify(person));
}

/** Makes a browser's session one of its person's devices, as a server reads whose a device is. */
async function browserIn(session: { id: string; userId: string }) {
  const user = await env.DB.prepare("select name from user where id = ?").bind(session.userId).first<{ name: string }>();
  await env.OWN.write(`devices/${await browserDevice(session.id)}`, who(user!));
}

/** Makes a browser's session nobody's, on every server. */
async function browserOut(session: string) {
  await env.OWN.remove(`devices/${await browserDevice(session)}`);
  await invalidate();
}

/** Makes a device nobody's, on every server. */
async function remove(adapter: Adapter, device: { id: string }) {
  await adapter.delete({ model: "device", where: [{ field: "id", value: device.id }] });
  await env.OWN.remove(`devices/${device.id}`);
  await invalidate();
}

/** The invitation a link carries, if it's one that still stands. The link is kept nowhere: only its hash is. */
async function invited(adapter: Adapter, link: string | undefined) {
  const [invite] = await adapter.findMany<Invite>({ model: "invite", where: [{ field: "id", value: await sha256(link ?? "") }] });
  if (!invite || invite.expiresAt < new Date()) throw new APIError("FORBIDDEN", { message: "That invitation expired. Ask for another." });
  return invite;
}

/**
 * Where a device asks for a code and for its answer, as the metadata says (RFC 8628): the device authorization
 * endpoint, and the token endpoint with the device code grant. Each names its key as `client_id`.
 */
const DEVICE_CODE = "/sign-in/auth/device/code";
const TOKEN = "/sign-in/auth/oauth2/token";
/** Where Better Auth answers a device that isn't an OAuth client, with a session the device has no use for. */
const DEVICE_TOKEN = "/sign-in/auth/device/token";
/** Where a person signs in, which the authorization endpoint shows one who comes with nothing to authorize. */
const AUTHORIZE = "/sign-in/auth/oauth2/authorize";
const SIGN_IN = "/sign-in/";

/** A device's code as Better Auth looks it up: its letters and digits, in capitals. */
const plain = (code: unknown) => String(code).replace(/[^a-zA-Z0-9]/g, "").toUpperCase();

/**
 * Devices, the invitations people join by, and what ties a device's code to its key, beside what Better Auth
 * keeps. A device asks for its code with its key as `client_id`, signed with that key, so a code is only ever for
 * the key that asked for it, and the key is that device's once its person says the code matches.
 */
const devices = () => ({
  id: "devices",
  schema: {
    device: {
      fields: {
        userId: { type: "string", references: { model: "user", field: "id" } },
        // `app` connects by its key, and `assistant` comes in over the web with a token.
        kind: { type: "string" },
        name: { type: "string" },
        createdAt: { type: "date" },
      },
    },
    invite: { fields: { userId: { type: "string", references: { model: "user", field: "id" } }, expiresAt: { type: "date" } } },
    // What a device that asked to be let in calls itself, which its person is shown with its code.
    deviceCode: { fields: { name: { type: "string", required: false } } },
  },
  async onRequest(request: Request) {
    const url = new URL(request.url);
    // The web client sends a person who isn't signed in to the authorization endpoint the metadata names, with no
    // OAuth client of its own: what it shows them is the page they sign in on.
    if (url.pathname === AUTHORIZE && request.method === "GET" && !url.searchParams.has("client_id"))
      return { response: Response.redirect(new URL(SIGN_IN, url.origin).href, 302) };
    if (url.pathname !== DEVICE_CODE && url.pathname !== TOKEN) return;
    // A device asks with a form, as RFC 8628 has it.
    const body = await request.clone().text();
    const form = Object.fromEntries(new URLSearchParams(body));
    if (url.pathname === TOKEN && form.grant_type !== DEVICE_CODE_GRANT_TYPE) return;
    // Whatever names a key as `client_id` is signed with it (`core/edge/ask.ts`).
    const key = await env.NETWORK.verifiedKey(request.url, Object.fromEntries(request.headers), body);
    if (!key || key !== form.client_id) {
      const refused = { error: "invalid_client", error_description: "Sign what you ask with the key it names, by a clock within 15 minutes of the right time" };
      return { response: Response.json(refused, { status: 401 }) };
    }
    // A device is no OAuth client, so it's answered as Better Auth answers a device of its own, which takes JSON.
    if (url.pathname === TOKEN)
      return { request: new Request(new URL(DEVICE_TOKEN, url.origin), { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(form) }) };
  },
  hooks: {
    before: [
      {
        // Allow on the consent page goes on to the passkey question, so it has to have just been answered. Cancel
        // needs no one signed in: Better Auth asks for a session to say no, so it's answered here, as Better Auth
        // would, at the redirect its own authorization endpoint checked before it signed the query.
        matcher: (ctx) => ctx.path === "/oauth2/consent",
        handler: createAuthMiddleware(async (ctx) => {
          if (ctx.body?.accept !== true) {
            const signed = String(ctx.body?.oauth_query ?? "");
            if (!(await verifyOAuthQueryParams(signed, ctx.context.secret))) throw new APIError("BAD_REQUEST", { error: "invalid_signature" });
            const query = new URLSearchParams(signed);
            const denied = new URLSearchParams({ error: "access_denied", error_description: "User denied access" });
            if (query.get("state")) denied.set("state", query.get("state")!);
            denied.set("iss", getIssuer(ctx, { loginPage: SIGN_IN, consentPage: SIGN_IN }));
            const url = new URL(query.get("redirect_uri")!);
            for (const [name, value] of denied) url.searchParams.set(name, value);
            return ctx.json({ redirect: true, url: url.href });
          }
          const session = await getSessionFromCtx(ctx);
          if (!session || Date.now() - new Date(session.session.createdAt).getTime() >= FRESH * 1000)
            throw new APIError("FORBIDDEN", { message: "Sign in again to approve this." });
        }),
      },
    ],
    after: [
      {
        // A device's code keeps what the device calls itself.
        matcher: (ctx) => ctx.path === "/device/code" && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const name = String(ctx.body.name ?? "").trim().slice(0, 100) || "A device";
          await ctx.context.adapter.update({ model: "deviceCode", where: [{ field: "deviceCode", value: (ctx.context.returned as { device_code: string }).device_code }], update: { name } });
        }),
      },
      {
        // Whoever the code is shown to is told what it's for.
        matcher: (ctx) => ctx.path === "/device" && !!(ctx.context.returned as { client_id?: string } | undefined)?.client_id,
        handler: createAuthMiddleware(async (ctx) => {
          const code = await ctx.context.adapter.findOne<DeviceCode>({ model: "deviceCode", where: [{ field: "userCode", value: plain(ctx.query?.user_code) }] });
          return ctx.json({ ...(ctx.context.returned as object), name: code!.name });
        }),
      },
      {
        // A device whose code its person said matches is theirs from then on: the key that asked for that code.
        matcher: (ctx) => ctx.path === "/device/approve" && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const { adapter } = ctx.context;
          const code = await adapter.findOne<DeviceCode>({ model: "deviceCode", where: [{ field: "userCode", value: plain(ctx.body.userCode) }] });
          const { user } = (await getSessionFromCtx(ctx))!;
          await place(adapter, code!.clientId, "app", code!.name, user);
        }),
      },
      {
        // The device connects by its key, so the session Better Auth makes for it when it learns its answer would
        // only be listed as a browser that isn't one.
        matcher: (ctx) => ctx.path === "/device/token" && !!ctx.context.newSession,
        handler: createAuthMiddleware(async (ctx) => {
          await ctx.context.internalAdapter.deleteSession(ctx.context.newSession!.session.token);
        }),
      },
      {
        // An assistant that was just let in is one of that person's devices from then on.
        matcher: (ctx) => ctx.path === "/oauth2/consent" && ctx.body?.accept === true && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const { adapter } = ctx.context;
          const { user } = (await getSessionFromCtx(ctx))!;
          const client = new URLSearchParams(ctx.body.oauth_query).get("client_id")!;
          const [{ name }] = await adapter.findMany<{ name?: string }>({ model: "oauthClient", where: [{ field: "clientId", value: client }] });
          await place(adapter, await assistant(client, user.id), "assistant", name || "An assistant", user);
        }),
      },
    ],
  },
}) satisfies BetterAuthPlugin;

/**
 * Fetches an assistant's metadata document, or what that names, as Better Auth's CIMD asks: over HTTPS, and never
 * by a redirect. Better Auth refuses an address that's private before asking. A name that resolves to one is
 * refused by Cloudflare itself, which resolves it once and connects to what it resolved: a Worker's fetch reaches
 * only the public internet.
 */
function fetchMetadata(input: RequestInfo | URL, init?: RequestInit) {
  const request = new Request(input, { ...init, redirect: "manual" });
  if (new URL(request.url).protocol !== "https:" || !["GET", "HEAD"].includes(request.method)) throw new TypeError("Only an HTTPS GET or HEAD");
  return fetch(request);
}

/** Better Auth for the network at `origin`, with its tables made. */
async function createAuth(origin: string) {
  const { secret, migrated } = await stored();
  const auth = betterAuth({
    baseURL: origin,
    basePath: "/sign-in/auth",
    secret,
    database: env.DB,
    // The JWT plugin signs the tokens an assistant is given, and hands out none of its own. A device asks for its
    // answer at the token endpoint, which passes it on to `/device/token` once it's checked its signature.
    disabledPaths: ["/token", "/device/token"],
    session: { freshAge: FRESH, cookieCache: { enabled: true, maxAge: BELIEVED } },
    // The name a person chose as they joined, which they can change: the one they're `users/<name>` by is chosen
    // for them, as they're invited, and never changes.
    user: { additionalFields: { called: { type: "string", required: false, input: true } } },
    databaseHooks: {
      user: { update: { after: goesBy } },
      // A browser that's signed in is one of its person's devices for as long as it is: the web client asks as its
      // session (`worker.ts`).
      session: {
        create: { after: (session) => browserIn(session) },
        delete: { after: (session) => browserOut(session.id) },
      },
    },
    // Better Auth limits how often an address asks only where NODE_ENV says production, which a Worker's doesn't,
    // and counts in memory, which each copy of a Worker has its own of.
    rateLimit: { enabled: true, storage: "database" },
    // Cloudflare says where a request came from in a header of its own.
    advanced: { ipAddress: { ipAddressHeaders: ["cf-connecting-ip"] } },
    plugins: [
      jwt({ disableSettingJwtHeader: true }),
      passkey({
        rpName: new URL(origin).host,
        registration: {
          // Whoever holds an invitation makes their first passkey with it, signed in as nobody yet.
          requireSession: false,
          resolveUser: async ({ ctx, context }) => {
            const invite = await invited(ctx.context.adapter, context ?? undefined);
            const user = (await ctx.context.internalAdapter.findUserById(invite.userId))!;
            return { id: user.id, name: user.name };
          },
          afterVerification: async ({ ctx, context }) => {
            if (context) await ctx.context.adapter.delete({ model: "invite", where: [{ field: "id", value: await sha256(context) }] });
          },
        },
      }),
      // What lets an assistant somewhere else in at /mcp, as MCP's authorization says. Its consent page is the sign-in
      // page, whose Allow goes on to the passkey.
      mcp({
        loginPage: SIGN_IN,
        consentPage: SIGN_IN,
        resource: `${origin}/mcp`,
        scopes: ["openid", "profile", "offline_access"],
        allowPublicClientPrelogin: true,
        // Most assistants still register themselves, which MCP keeps for them.
        allowDynamicClientRegistration: true,
        allowUnauthenticatedClientRegistration: true,
        clientRegistrationDefaultApplicationType: "infer",
        // Who the token's holder acts as, so that asking for their servers asks nothing of the database.
        customAccessTokenClaims: ({ user }) => ({ who: who(user!) }),
      }),
      // An assistant may instead name itself by the address of its metadata document, which MCP recommends and
      // Claude asks for first. Better Auth checks the address, fetches it within 5 seconds and 5 KB, checks it names
      // that address and the redirect it asks for, and keeps it as its cache headers say.
      cimd({ fetchClientMetadataResource: fetchMetadata, metadataProfile: "mcp-2026-07-28" }),
      // A new device shows a code, and its link carries it. Its person opens that where they sign in, and says
      // whether the code they're shown matches. The metadata names where it asks (RFC 8628).
      // A device's `client_id` is its key, which it signed what it asked with (`onRequest` in `devices`).
      oauthDeviceAuthorization({ verificationUri: SIGN_IN, validateClient: async () => true }),
      devices(),
    ],
  });
  // Working out what to make asks the database about every table, a round trip each, so it's done once for each
  // version of this Worker deployed, as a deploy's migrations are, not as each copy of it starts.
  if (migrated !== env.VERSION.id) {
    const { runMigrations } = await getMigrations(auth.options);
    await runMigrations();
    await env.DB.batch([env.DB.prepare("delete from migrated"), env.DB.prepare("insert into migrated values (?)").bind(env.VERSION.id)]);
  }
  return auth;
}

/** Better Auth, as this Worker sets it up. */
export type Auth = Awaited<ReturnType<typeof createAuth>>;

const auths = new Map<string, Promise<Auth>>();
/** Better Auth for the network at `origin`, made once for as long as this Worker stays loaded. */
export function at(origin: string): Promise<Auth> {
  if (!auths.has(origin)) auths.set(origin, createAuth(origin).catch((error) => (auths.delete(origin), Promise.reject(error))));
  return auths.get(origin)!;
}

/**
 * The codes no device showed that `user` tried to let a device in by, within `WRONG_CODES.window` of the last:
 * how many, when the last was, and `again()`, which counts one more.
 */
async function wrong(adapter: Adapter, user: User) {
  const key = `approve/${user.id}`;
  const [row] = await adapter.findMany<{ count: number; lastRequest: number | string }>({ model: "rateLimit", where: [{ field: "key", value: key }] });
  const lastRequest = Number(row?.lastRequest ?? 0);
  const count = row && Date.now() - lastRequest < WRONG_CODES.window * 1000 ? row.count : 0;
  const again = () => {
    const data = { key, count: count + 1, lastRequest: Date.now() };
    return row ? adapter.update({ model: "rateLimit", where: [{ field: "key", value: key }], update: data }) : adapter.create({ model: "rateLimit", data });
  };
  return { count, lastRequest, again };
}

/**
 * A link that `name` joins the network by, as that user, made for them if they're new. It's their only one: an
 * earlier link to join as them no longer works.
 */
async function invite(adapter: Adapter, internalAdapter: InternalAdapter, origin: string, name: string, user: User | undefined) {
  if (!/^[a-z][a-z0-9-]*$/.test(name)) return { error: "A user's name is lowercase letters, digits and hyphens, starting with a letter." };
  const invitee = user ?? (await made(internalAdapter, origin, name));
  return { link: `${origin}/sign-in/#invite=${await invitation(adapter, invitee, INVITED_FOR)}` };
}

type InternalAdapter = Awaited<Auth["$context"]>["internalAdapter"];

/** The user `name`, made here, who hasn't joined. */
function made(internalAdapter: InternalAdapter, origin: string, name: string): Promise<User> {
  // Better Auth keeps a unique email for everyone, so each has one made of their name here. Nothing is sent to it.
  return internalAdapter.createUser({ name, email: `${name}@${new URL(origin).hostname}`, emailVerified: false }, { method: "admin" });
}

/**
 * A new invitation for `user` to make a passkey by, lasting `lasts` milliseconds: what's carried after `#invite=` or
 * `#passkey=` in a link. It ends the one made for them before.
 */
async function invitation(adapter: Adapter, user: { id: string }, lasts: number) {
  await adapter.deleteMany({ model: "invite", where: [{ field: "userId", value: user.id }] });
  const link = hex(crypto.getRandomValues(new Uint8Array(24)));
  await adapter.create({ model: "invite", data: { id: await sha256(link), userId: user.id, expiresAt: new Date(Date.now() + lasts) }, forceAllowId: true });
  return link;
}

/** Whether `user` has joined: they made a passkey. */
const joined = async (adapter: Adapter, user: User) => (await adapter.findMany({ model: "passkey", where: [{ field: "userId", value: user.id }] })).length > 0;

/**
 * What a server acting for the network asks, for a person in the app: `{ask, ...}`. Their name is theirs as
 * relation tuples have it, `users/<name>`.
 */
/** What a server asks: `ask`, about whom, and what else that ask takes. */
export interface Ask {
  ask?: string;
  who?: string;
  called?: unknown;
  device?: string;
  code?: unknown;
  passkey?: string;
  /** Whether the network is being built, where a server acts as whoever a client says (`mix network --development`). */
  building?: boolean;
}

export async function asked(auth: Auth, origin: string, { ask, who: name, building, ...about }: Ask) {
  const { adapter, internalAdapter } = await auth.$context;
  const named = String(name ?? "").replace(/^users\//, "");
  let [user] = await adapter.findMany<User>({ model: "user", where: [{ field: "name", value: named }] });
  // In a network being built nobody joins, and a server asks as whoever a client says it is: they're made here as
  // they're first asked about, so they approve a device's code as anyone who joined does.
  if (!user && building && ask !== "invite" && /^[a-z][a-z0-9-]*$/.test(named)) user = await made(internalAdapter, origin, named);

  if (ask === "invite") return invite(adapter, internalAdapter, origin, named, user);
  // The network's first owner is invited only until they've joined, so a link to join as them isn't made after.
  if (ask === "owner") return user && (await joined(adapter, user)) ? { joined: true } : invite(adapter, internalAdapter, origin, named, user);
  // Everyone who has joined, and everyone invited who hasn't yet, with the name each goes by.
  if (ask === "everyone") {
    type Row = { id: string; userId: string; expiresAt?: Date; name?: string; called?: string | null; email?: string; emailVerified?: boolean };
    const [users, passkeys, invites] = await Promise.all(["user", "passkey", "invite"].map((model) => adapter.findMany<Row>({ model })));
    const has = (rows: Row[]) => new Set(rows.filter(({ expiresAt }) => !expiresAt || expiresAt > new Date()).map(({ userId }) => userId));
    const [keyed, invited] = [has(passkeys), has(invites)];
    return {
      people: users
        .filter(({ id }) => keyed.has(id) || invited.has(id))
        .map((user) => ({ who: who(user as { name: string }), called: user.called ?? null, email: user.emailVerified ? user.email : null, joined: keyed.has(user.id) })),
    };
  }
  if (!user) return { error: `${name} hasn't joined this network.` };
  const mine = [{ field: "userId", value: user.id }];

  if (ask === "called") {
    const called = String(about.called ?? "").trim().slice(0, 100);
    if (!called) return { error: "A name can't be empty." };
    await internalAdapter.updateUser(user.id, { called });
    return { called };
  }

  if (ask === "devices") {
    const [devices, browsers, passkeys] = await Promise.all([
      adapter.findMany<Device>({ model: "device", where: mine }),
      internalAdapter.listSessions(user.id),
      adapter.findMany<{ id: string; name?: string; createdAt: Date }>({ model: "passkey", where: mine }),
    ]);
    // Whether an app is connected to the relay, which says when it was last seen otherwise.
    const presence = await env.RELAY.presence(devices.filter(({ kind }) => kind === "app").map(({ id }) => id));
    const when = (id: string) => {
      const here = presence[id];
      return here ? { connected: here.connected, ...(here.last_seen && { last_seen: new Date(here.last_seen) }) } : {};
    };
    return {
      devices: [
        ...devices.map(({ id, kind, name, createdAt }) => ({ id, kind, name, added: createdAt, ...when(id) })),
        // A browser that's signed in is a device too, named by what it says it is.
        ...browsers.map(({ id, userAgent, createdAt }) => ({ id, kind: "browser", name: browser(userAgent), added: createdAt })),
      ],
      passkeys: passkeys.map(({ id, name, createdAt }) => ({ id, name: name || "Passkey", added: createdAt })),
    };
  }
  if (ask === "remove_device") {
    const [device] = await adapter.findMany<Device>({ model: "device", where: [...mine, { field: "id", value: about.device! }] });
    if (device) {
      await remove(adapter, device);
      // What an assistant was given to come back with ends with it.
      if (device.kind === "assistant") {
        const clients = await adapter.findMany<{ clientId: string }>({ model: "oauthConsent", where: mine });
        for (const { clientId } of clients) {
          if ((await assistant(clientId, user.id)) !== device.id) continue;
          const theirs = [...mine, { field: "clientId", value: clientId }];
          for (const model of ["oauthRefreshToken", "oauthAccessToken", "oauthConsent"]) await adapter.deleteMany({ model, where: theirs });
        }
      }
      return { removed: device.name, device: device.id };
    }
    const signed_in = (await internalAdapter.listSessions(user.id)).find(({ id }) => id === about.device);
    if (!signed_in) return { error: "That isn't one of your devices." };
    await internalAdapter.deleteSession(signed_in.token);
    return { removed: browser(signed_in.userAgent), device: await browserDevice(signed_in.id) };
  }
  if (ask === "approve") {
    // A person lets a device in by its code from one of their own devices or assistants, which a passkey let in.
    // Better Auth's device authorization answers a code only for a browser that's signed in, so this is what its
    // `/device` and `/device/approve` check, in one change that only a code nobody else has opened makes: it's
    // answered once. The device then learns its answer from Better Auth as it would have.
    const missed = await wrong(adapter, user);
    if (missed.count >= WRONG_CODES.max) {
      const minutes = Math.ceil((missed.lastRequest + WRONG_CODES.window * 1000 - Date.now()) / 60_000);
      return { error: `That's too many codes no device showed. Try again in ${minutes} minute${minutes === 1 ? "" : "s"}.` };
    }
    const code = await adapter.findOne<DeviceCode>({ model: "deviceCode", where: [{ field: "userCode", value: plain(about.code) }] });
    // Better Auth forgets a code once its device has learned it was answered, stopped or ended.
    if (!code) {
      await missed.again();
      return { error: "That isn't a code any device is showing now." };
    }
    if (code.expiresAt < new Date()) return { error: "That code expired. The device shows another when it's asked again." };
    if (code.status === "denied") return { error: "That code was denied, so its device isn't approved." };
    if (code.status !== "pending") return { error: "Already approved." };
    if (code.userId && code.userId !== user.id) return { error: "Someone else opened that code first, and only they can answer it." };
    const where = [
      { field: "id", value: code.id },
      { field: "status", value: "pending" },
      { field: "userId", operator: "eq" as const, value: code.userId ?? null },
    ];
    const updated = await adapter.incrementOne({ model: "deviceCode", where, increment: {}, set: { status: "approved", userId: user.id } });
    if (!updated) return { error: "Already approved." };
    await place(adapter, code.clientId, "app", code.name, user);
    return { approved: code.name };
  }
  // Someone taken out of the network: nothing acts as them from then on, they sign in nowhere, and their name is free.
  if (ask === "remove_person") {
    for (const device of await adapter.findMany<Device>({ model: "device", where: mine })) await remove(adapter, device);
    for (const { id } of await adapter.findMany<{ id: string }>({ model: "session", where: mine })) await browserOut(id);
    for (const model of ["oauthRefreshToken", "oauthAccessToken", "oauthConsent", "session", "passkey", "invite", "account"])
      await adapter.deleteMany({ model, where: mine });
    await adapter.delete({ model: "user", where: [{ field: "id", value: user.id }] });
    await env.OWN.remove(`people/${user.name}`);
    return { removed: user.called ?? user.name };
  }
  // A link that this person makes another passkey by, in a browser, within a quarter of an hour.
  if (ask === "passkey") return { link: `${origin}/sign-in/#passkey=${await invitation(adapter, user, 15 * 60 * 1000)}` };
  if (ask === "remove_passkey") {
    const passkeys = await adapter.findMany<{ id: string; name?: string }>({ model: "passkey", where: mine });
    const passkey = passkeys.find(({ id }) => id === about.passkey);
    if (!passkey) return { error: "That isn't one of your passkeys." };
    if (passkeys.length === 1) return { error: "That's your only passkey, and you sign in with it. Add another before it goes." };
    await adapter.delete({ model: "passkey", where: [{ field: "id", value: passkey.id }] });
    return { removed: passkey.name || "Passkey" };
  }
  return { error: `There's nothing to ask named ${ask}.` };
}
