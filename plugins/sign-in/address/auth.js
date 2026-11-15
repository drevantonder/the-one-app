// Who signs in to a network, and what they let act as them: Better Auth, kept in this Worker's own database. A
// person signs in with a passkey and nothing else. An assistant becomes theirs when they answer that one question
// for it, and a new device when they also say the code it shows matches the one they're shown, as Better Auth's
// device authorization has it. Each is then a line in this plugin's own artifact, `devices/<device>`, saying whose
// it is: that's where a server reads it, so no call asks anything here.
import { env } from "cloudflare:workers";
import { betterAuth } from "better-auth";
import { APIError, createAuthMiddleware, getSessionFromCtx } from "better-auth/api";
import { getMigrations } from "better-auth/db/migration";
import { deviceAuthorization, jwt } from "better-auth/plugins";
import { mcp } from "@better-auth/mcp";
import { passkey } from "@better-auth/passkey";
import Bowser from "bowser";

/** How long ago someone may have answered the passkey question for it to count as them saying yes now, in seconds. */
const FRESH = 5 * 60;
/** How long an invitation lasts, in milliseconds. */
const INVITED_FOR = 7 * 24 * 60 * 60 * 1000;

const hex = (bytes) => Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("");
const sha256 = async (text) => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));

/** What a browser is called, from what it says of itself: "Chrome on Linux". */
function browser(agent) {
  const { browser, os } = Bowser.parse(agent || " ");
  return [browser.name, os.name].filter(Boolean).join(" on ") || "A browser";
}

/** A user as relation tuples name them. Their name here is the part after `users/`. */
export const who = (user) => `users/${user.name}`;

/**
 * The device an assistant somewhere else is, to a server: one for each program a person let in, named by both.
 * It's the size of a key and nobody's key, so it never names a device that connects by one.
 */
export const assistant = (client, user) => sha256(`${client}\n${user}`);

/** What signs this Worker's cookies and tokens, made on first use and kept beside what it protects. */
async function secret() {
  await env.DB.exec("create table if not exists secret (secret text not null)");
  const made = hex(crypto.getRandomValues(new Uint8Array(32)));
  await env.DB.prepare("insert into secret select ? where not exists (select 1 from secret)").bind(made).run();
  return (await env.DB.prepare("select secret from secret").first()).secret;
}

/** Tells every server that whose a device is has changed, so each asks again. */
const forget = () => env.RELAY.tell(JSON.stringify({ jsonrpc: "2.0", method: "notifications/one/decided", params: { hooks: ["device"] } }));

/** Makes `device` the user's, under `name`, from now on. */
async function place(adapter, device, kind, name, user) {
  const [was] = await adapter.findMany({ model: "device", where: [{ field: "id", value: device }] });
  if (was) await remove(adapter, was);
  await adapter.create({ model: "device", data: { id: device, userId: user.id, kind, name, createdAt: new Date() }, forceAllowId: true });
  await env.OWN.write(`devices/${device}`, who(user));
}

/** Makes a device nobody's, on every server. */
async function remove(adapter, device) {
  await adapter.delete({ model: "device", where: [{ field: "id", value: device.id }] });
  await env.OWN.remove(`devices/${device.id}`);
  await forget();
}

/** The invitation a link carries, if it's one that still stands. The link is kept nowhere: only its hash is. */
async function invited(adapter, link) {
  const [invite] = await adapter.findMany({ model: "invite", where: [{ field: "id", value: await sha256(link ?? "") }] });
  if (!invite || invite.expiresAt < new Date()) throw new APIError("FORBIDDEN", { message: "That invitation has ended. Ask for another." });
  return invite;
}

/** Where a device asks for a code and for its answer, each naming its key as `client_id`. */
const ASKS_BY_KEY = ["/sign-in/auth/device/code", "/sign-in/auth/device/token"];

/** A device's code as Better Auth looks it up: its letters and digits, in capitals. */
const plain = (code) => String(code).replace(/[^a-zA-Z0-9]/g, "").toUpperCase();

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
  // Whatever names a key as `client_id` is signed with it (`address/ask.js`).
  async onRequest(request) {
    if (!ASKS_BY_KEY.includes(new URL(request.url).pathname)) return;
    const body = await request.clone().text();
    let client;
    try {
      client = JSON.parse(body).client_id;
    } catch {}
    const key = await env.NETWORK.asker(request.url, Object.fromEntries(request.headers), body);
    if (key && key === client) return;
    const refused = { error: "invalid_client", error_description: "Sign what you ask with the key it names, by a clock within a minute of the right time" };
    return { response: Response.json(refused, { status: 401 }) };
  },
  hooks: {
    before: [
      {
        // No consent screen: the passkey question is the consent, so it has to have just been answered.
        matcher: (ctx) => ctx.path === "/oauth2/consent",
        handler: createAuthMiddleware(async (ctx) => {
          const session = await getSessionFromCtx(ctx);
          if (!session || Date.now() - new Date(session.session.createdAt).getTime() >= FRESH * 1000)
            throw new APIError("FORBIDDEN", { message: "Sign in again to let this in." });
        }),
      },
    ],
    after: [
      {
        // A device's code keeps what the device calls itself.
        matcher: (ctx) => ctx.path === "/device/code" && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const name = String(ctx.body.name ?? "").trim().slice(0, 100) || "A device";
          await ctx.context.adapter.update({ model: "deviceCode", where: [{ field: "deviceCode", value: ctx.context.returned.device_code }], update: { name } });
        }),
      },
      {
        // Whoever the code is shown to is told what it's for.
        matcher: (ctx) => ctx.path === "/device" && ctx.context.returned?.client_id,
        handler: createAuthMiddleware(async (ctx) => {
          const code = await ctx.context.adapter.findOne({ model: "deviceCode", where: [{ field: "userCode", value: plain(ctx.query.user_code) }] });
          return ctx.json({ ...ctx.context.returned, name: code.name });
        }),
      },
      {
        // A device whose code its person said matches is theirs from then on: the key that asked for that code.
        matcher: (ctx) => ctx.path === "/device/approve" && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const { adapter } = ctx.context;
          const code = await adapter.findOne({ model: "deviceCode", where: [{ field: "userCode", value: plain(ctx.body.userCode) }] });
          const { user } = await getSessionFromCtx(ctx);
          await place(adapter, code.clientId, "app", code.name, user);
        }),
      },
      {
        // The device connects by its key, so the session Better Auth makes for it when it learns its answer would
        // only be listed as a browser that isn't one.
        matcher: (ctx) => ctx.path === "/device/token" && ctx.context.newSession,
        handler: createAuthMiddleware(async (ctx) => {
          await ctx.context.internalAdapter.deleteSession(ctx.context.newSession.session.token);
        }),
      },
      {
        // An assistant that was just let in is one of that person's devices from then on.
        matcher: (ctx) => ctx.path === "/oauth2/consent" && ctx.body?.accept === true && !(ctx.context.returned instanceof Error),
        handler: createAuthMiddleware(async (ctx) => {
          const { adapter } = ctx.context;
          const { user } = await getSessionFromCtx(ctx);
          const client = new URLSearchParams(ctx.body.oauth_query).get("client_id");
          const [{ name }] = await adapter.findMany({ model: "oauthClient", where: [{ field: "clientId", value: client }] });
          await place(adapter, await assistant(client, user.id), "assistant", name || "An assistant", user);
        }),
      },
    ],
  },
});

/** Better Auth for the network at `origin`, with its tables made. */
async function made(origin) {
  const auth = betterAuth({
    baseURL: origin,
    basePath: "/sign-in/auth",
    secret: await secret(),
    database: env.DB,
    // The JWT plugin signs the tokens an assistant is given, and hands out none of its own.
    disabledPaths: ["/token"],
    session: { freshAge: FRESH },
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
            const invite = await invited(ctx.context.adapter, context);
            const user = await ctx.context.internalAdapter.findUserById(invite.userId);
            return { id: user.id, name: user.name };
          },
          afterVerification: async ({ ctx, context }) => {
            if (context) await ctx.context.adapter.delete({ model: "invite", where: [{ field: "id", value: await sha256(context) }] });
          },
        },
      }),
      // What lets an assistant somewhere else in at /mcp, as MCP's authorization says. Signing in and saying yes are
      // one page and one question.
      mcp({
        loginPage: "/sign-in/",
        consentPage: "/sign-in/",
        resource: `${origin}/mcp`,
        scopes: ["openid", "profile", "offline_access"],
        allowPublicClientPrelogin: true,
        // Most assistants still register themselves, which MCP keeps for them.
        allowDynamicClientRegistration: true,
        allowUnauthenticatedClientRegistration: true,
        clientRegistrationDefaultApplicationType: "infer",
        // Who the token's holder acts as, so that asking for their servers asks nothing of the database.
        customAccessTokenClaims: ({ user }) => ({ who: who(user) }),
      }),
      // A new device shows a code, and its link carries it. Its person opens that where they sign in, and says
      // whether the code they're shown matches.
      deviceAuthorization({ verificationUri: "/sign-in/" }),
      devices(),
    ],
  });
  const { runMigrations } = await getMigrations(auth.options);
  await runMigrations();
  return auth;
}

const auths = new Map();
/** Better Auth for the network at `origin`, made once for as long as this Worker stays loaded. */
export function at(origin) {
  if (!auths.has(origin)) auths.set(origin, made(origin).catch((error) => (auths.delete(origin), Promise.reject(error))));
  return auths.get(origin);
}

/**
 * What a server acting for the network asks, for a person in the app: `{ask, ...}`. Their name is theirs as
 * relation tuples have it, `users/<name>`.
 */
export async function asked(auth, origin, { ask, who: name, ...about }) {
  const { adapter, internalAdapter } = await auth.$context;
  const named = String(name ?? "").replace(/^users\//, "");
  const [user] = await adapter.findMany({ model: "user", where: [{ field: "name", value: named }] });

  if (ask === "invite") {
    if (!/^[a-z][a-z0-9-]*$/.test(named)) return { error: "A user's name is lowercase letters, digits and hyphens, starting with a letter." };
    // Better Auth keeps a unique email for everyone, so each has one made of their name here. Nothing is sent to it.
    const invitee = user ?? (await internalAdapter.createUser({ name: named, email: `${named}@${new URL(origin).hostname}`, emailVerified: false }));
    const link = hex(crypto.getRandomValues(new Uint8Array(24)));
    await adapter.create({ model: "invite", data: { id: await sha256(link), userId: invitee.id, expiresAt: new Date(Date.now() + INVITED_FOR) }, forceAllowId: true });
    return { link: `${origin}/sign-in/#invite=${link}` };
  }
  if (!user) return { error: `${name} hasn't joined this network.` };
  const mine = [{ field: "userId", value: user.id }];

  if (ask === "devices") {
    const [devices, browsers, passkeys] = await Promise.all([
      adapter.findMany({ model: "device", where: mine }),
      internalAdapter.listSessions(user.id),
      adapter.findMany({ model: "passkey", where: mine }),
    ]);
    return {
      devices: [
        ...devices.map(({ id, kind, name, createdAt }) => ({ id, kind, name, added: createdAt })),
        // A browser that's signed in is a device too, named by what it says it is.
        ...browsers.map(({ id, userAgent, createdAt }) => ({ id, kind: "browser", name: browser(userAgent), added: createdAt })),
      ],
      passkeys: passkeys.map(({ id, name, createdAt }) => ({ id, name: name || "Passkey", added: createdAt })),
    };
  }
  if (ask === "remove_device") {
    const [device] = await adapter.findMany({ model: "device", where: [...mine, { field: "id", value: about.device }] });
    if (device) {
      await remove(adapter, device);
      // What an assistant was given to come back with ends with it.
      if (device.kind === "assistant") {
        const clients = await adapter.findMany({ model: "oauthConsent", where: mine });
        for (const { clientId } of clients) {
          if ((await assistant(clientId, user.id)) !== device.id) continue;
          const theirs = [...mine, { field: "clientId", value: clientId }];
          for (const model of ["oauthRefreshToken", "oauthAccessToken", "oauthConsent"]) await adapter.deleteMany({ model, where: theirs });
        }
      }
      return { removed: device.name };
    }
    const signed_in = (await internalAdapter.listSessions(user.id)).find(({ id }) => id === about.device);
    if (!signed_in) return { error: "That isn't one of your devices." };
    await internalAdapter.deleteSession(signed_in.token);
    return { removed: browser(signed_in.userAgent) };
  }
  if (ask === "remove_passkey") {
    const passkeys = await adapter.findMany({ model: "passkey", where: mine });
    const passkey = passkeys.find(({ id }) => id === about.passkey);
    if (!passkey) return { error: "That isn't one of your passkeys." };
    if (passkeys.length === 1) return { error: "That's your only passkey, and you sign in with it. Add another before it goes." };
    await adapter.delete({ model: "passkey", where: [{ field: "id", value: passkey.id }] });
    return { removed: passkey.name || "Passkey" };
  }
  return { error: `There's nothing to ask named ${ask}.` };
}
