// Signing in's edge function. It serves the sign-in page and what the page asks (`/sign-in/`), is the
// OAuth provider that MCP's authorization asks for (`/.well-known`), and is the way in over the web for whatever
// has signed in (`/mcp`): an assistant somewhere else with the token a person let it have, one of the network's own
// devices with its key, or a browser signed in here, the web client. Either way the call is passed through the relay
// to a server, which is told which device it's from and decides whose that is itself. The web client's apps' files
// come through here too (`/apps/`).
// `/llms.txt` tells an assistant that's pointed at the network how to come in, as Settings › Connect Your AI asks.
import { env } from "cloudflare:workers";
import { requireMcpAuth } from "@better-auth/mcp";
import { asked, assistant, at, browserDevice, who, type Ask, type Auth } from "./auth.ts";

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";
/** The JSON-RPC error a server answers a device that's nobody's (`OneCore.Mcp`). */
const NOBODY = -32003;

const refuse = (status: number, why: string) => Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

export default {
  async fetch(request: Request) {
    const url = new URL(request.url);
    const auth = await at(url.origin);
    if (url.pathname === "/llms.txt") return new Response(llms(url.origin), { headers: { "content-type": "text/markdown; charset=utf-8" } });
    if (url.pathname === "/mcp") return mcp(request, auth, url);
    if (url.pathname === "/apps") return apps(request, auth);
    if (url.pathname.startsWith("/apps/")) return file(auth, url);
    if (url.pathname === "/sign-in/ask") return ask(request, auth, url);
    if (url.pathname.startsWith("/sign-in/auth/") || url.pathname.startsWith("/.well-known/")) return auth.handler(request);
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler;

/** What an assistant reads to come into the network at `origin`, as llmstxt.org has it. */
const llms = (origin: string) => `# ${new URL(origin).host}

> A One App network: a family's or a workplace's apps, files and devices. An assistant comes in as the person who
> lets it in, and does what they can.

## Connecting

- Add ${origin}/mcp as a remote MCP server (a connector), over streamable HTTP. In Claude Code:
  \`claude mcp add --transport http one ${origin}/mcp\`
- It signs in with OAuth. The person is asked for their passkey once, and that's their yes.
- Then their tools are yours: \`tools/list\` lists them. Read a plugin's skill before using its tools:
  \`resources/list\` lists them, under \`skill://\`, and \`skill://building-plugins/SKILL.md\` says how to make one.
`;

/** The key that signed what was asked, as the pieces of the edge check it, or null. */
const signerKey = (request: Request, body: string) => env.NETWORK.verifiedKey(request.url, Object.fromEntries(request.headers), body);

async function mcp(request: Request, auth: Auth, url: URL) {
  // MCP's streamable HTTP: each message is a POST, answered with JSON. There's no stream to GET.
  if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
  const traceparent = request.headers.get("traceparent") ?? undefined;

  // One of the network's own devices signs what it asks with its key. One that's nobody's is told so by the
  // server, and asks for a code to be let in by (`auth.ts`).
  if (request.headers.has("one-key")) {
    const body = await request.text();
    const device = await signerKey(request, body);
    if (!device) return refuse(401, "Sign what you ask with your key, by a clock within a minute of the right time");
    return forward(body, device, [NETWORK], traceparent);
  }

  // A browser signed in here is the device its session is (`auth.ts`). One that's nobody's signs in again.
  if (!request.headers.has("authorization")) {
    const signedIn = await auth.api.getSession({ headers: request.headers });
    if (signedIn) {
      const answer = await forward(await request.text(), await browserDevice(signedIn.session.id), [`${who(signedIn.user)}/`, NETWORK], traceparent);
      return (await nobodys(answer)) ? refuse(401, "Sign in again") : answer;
    }
  }

  // An assistant somewhere else shows the token it was given when a person let it in. One they've since removed
  // is nobody's, and is asked to be let in again.
  const resource = `${url.origin}/mcp`;
  const approve = requireMcpAuth(
    auth,
    async (request, claims) => {
      const answer = await forward(await request.text(), await assistant(claims.azp as string, claims.sub as string), [`${claims.who}/`, NETWORK], traceparent);
      if (!(await nobodys(answer))) return answer;
      const challenge = `Bearer error="invalid_token", resource_metadata="${url.origin}/.well-known/oauth-protected-resource/mcp"`;
      return new Response(null, { status: 401, headers: { "www-authenticate": challenge } });
    },
    { resource },
  );
  return approve(request);
}

/**
 * Passes one JSON-RPC message to a server acting for one of `of`, as from `device`, and answers what the server
 * did. What's asked says which device it's from, who it's for and the span it's under, and nothing else of itself.
 */
async function forward(message: string, device: string, of: string[], header?: string) {
  let call: { method?: unknown; params?: { _meta?: { for?: unknown; traceparent?: unknown } } };
  try {
    call = JSON.parse(message);
    if (typeof call?.method !== "string") throw new Error("not a request");
  } catch {
    return refuse(400, "That isn't a JSON-RPC request");
  }
  const asks_for = call.params?._meta?.for;
  const traceparent = traced(call.params?._meta?.traceparent ?? header);
  const said = { device, traceparent, ...(typeof asks_for === "string" && { for: asks_for }) };
  message = JSON.stringify({ ...call, params: { ...call.params, _meta: said } });
  const servers = [...new Set((await Promise.all(of.map((of) => env.NETWORK.servers(of)))).flat())];
  const reached = await env.RELAY.forward(servers, message);
  if (!reached) return refuse(503, "None of this network's servers answered. Try again once one is on.");
  // A notification has no answer.
  if (reached.answer === "") return new Response(null, { status: 202 });
  return new Response(reached.answer, { headers: { "content-type": "application/json" } });
}

/**
 * The `traceparent` of this request's span, as W3C's Trace Context has it: under `parent`, the client's, which it
 * sends in `_meta` as MCP has it or as a header, if any. It's said in a line Telemetry's Tail Worker reads
 * (`plugins/telemetry/edge/worker.ts`), and carried to the server.
 */
function traced(parent: unknown) {
  const random = (bytes: number) => Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (byte) => byte.toString(16).padStart(2, "0")).join("");
  const under = typeof parent === "string" ? /^00-([0-9a-f]{32})-[0-9a-f]{16}-[0-9a-f]{2}$/.exec(parent) : null;
  const traceparent = `00-${under?.[1] ?? random(16)}-${random(8)}-01`;
  console.log(JSON.stringify({ telemetry: { traceparent, parent: under?.[0] } }));
  return traceparent;
}

/** Whether a server answered that the device is nobody's. The answer is left to be sent on if it isn't. */
async function nobodys(answer: Response) {
  if (answer.status !== 200) return false;
  const said = await answer.clone().json<{ error?: { code?: number } }>();
  return said.error?.code === NOBODY;
}

/**
 * What a server asks for a person in the app: one acting for them, or for the network, which acts for everyone, as
 * servers have it themselves (`OneCore.Servers.acting/1`). Nothing else is believed about who that is. An
 * invitation, the first owner's included, is only the network's to make, since it can make someone nobody acts for
 * yet, and so is taking someone out and listing everyone, which a person's own server isn't told.
 */
async function ask(request: Request, auth: Auth, url: URL) {
  const body = await request.text();
  const server = await signerKey(request, body);
  let what: Ask;
  try {
    what = JSON.parse(body);
  } catch {
    return Response.json({ error: "An ask is JSON" }, { status: 400 });
  }
  const of = ["invite", "owner", "everyone", "remove_person"].includes(what.ask ?? "") ? [NETWORK] : [NETWORK, `${what.who}/`];
  const acting = (await Promise.all(of.map((of) => env.NETWORK.servers(of)))).flat();
  if (!server || !acting.includes(server)) return Response.json({ error: "Only a server acting for them asks this" }, { status: 403 });
  return Response.json(await asked(auth, url.origin, what));
}

/**
 * Where a signed-in browser's apps' files are, `/apps/<its session, signed>`, and the device it is: its session, as
 * Devices lists it. An app's page has an origin of its own in the web client, so its files are asked for with no
 * cookie, and the signed session in their address says whose they are.
 */
async function apps(request: Request, auth: Auth) {
  const { headers, response: signedIn } = await auth.api.getSession({ headers: request.headers, returnHeaders: true });
  if (!signedIn) return Response.json({ error: "Sign in" }, { status: 401 });
  const { secret } = await auth.$context;
  const signature = await crypto.subtle.sign("HMAC", await signing(secret), signs(signedIn.session.id));
  const hex = Array.from(new Uint8Array(signature), (byte) => byte.toString(16).padStart(2, "0")).join("");
  const answer = Response.json({ apps: `/apps/${signedIn.session.id}.${hex}`, device: signedIn.session.id });
  // Better Auth sends a session that's being used a later end.
  for (const cookie of headers.getSetCookie()) answer.headers.append("set-cookie", cookie);
  return answer;
}

/** The key a session is signed with into the address of its apps' files: this Worker's secret, as HMAC's. */
const signing = (secret: string) => crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
/** What's signed for a session. */
const signs = (session: string) => new TextEncoder().encode(`apps/${session}`);

/**
 * What an app's page loads in the web client: `/apps/<session, signed>/<app>/<plugin>/<path>` is `path` among the
 * latest files of `plugin` that a server lists for that session's person, as the app's client has them at its own
 * address for each app (`clients/app/src/lib.rs`). An app opened for a group or the network says so after its name,
 * last part first (`notes.family.groups`).
 */
async function file(auth: Auth, url: URL) {
  const [signed, app, plugin, ...path] = url.pathname.slice("/apps/".length).split("/");
  const [session, hex = ""] = signed.split(".");
  const { adapter, internalAdapter, secret } = await auth.$context;
  const signature = Uint8Array.from(hex.match(/../g) ?? [], (byte) => parseInt(byte, 16));
  const valid = await crypto.subtle.verify("HMAC", await signing(secret), signature, signs(session));
  const kept = valid && (await adapter.findOne<{ userId: string; expiresAt: Date }>({ model: "session", where: [{ field: "id", value: session }] }));
  if (!kept || kept.expiresAt < new Date()) return new Response("Sign in again", { status: 401 });
  const user = (await internalAdapter.findUserById(kept.userId))!;
  const [, ...whose] = app.split(".");
  const meta = whose.length ? { _meta: { for: whose.reverse().join("/") } } : {};
  const ask = async <Result,>(method: string, params: object): Promise<Result | null> => {
    const answer = await forward(JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: { ...params, ...meta } }), await browserDevice(session), [`${who(user)}/`, NETWORK]);
    if (answer.status !== 200) return null;
    return (await answer.json<{ result: Result }>()).result;
  };
  const listed = await ask<{ resources: { uri: string }[] }>("resources/list", {});
  const version = listed?.resources.map(({ uri }) => uri.split("/")).find(([scheme, , of]) => scheme === "ui:" && of === plugin)?.[3];
  const read = version ? await ask<{ contents: { blob?: string; text?: string; mimeType: string }[] }>("resources/read", { uri: `ui://${plugin}/${version}/${path.join("/")}` }) : null;
  const found = read?.contents?.[0];
  if (!found) return new Response(`There's nothing at ${url.pathname}`, { status: 404 });
  const body = found.blob ? Uint8Array.from(atob(found.blob), (char) => char.charCodeAt(0)) : found.text;
  return new Response(body, {
    headers: {
      "content-type": found.mimeType.replace(";profile=mcp-app", ""),
      // What an app's page may load, which is what an MCP App may by default and what the app lets one
      // (`APP_MAY_LOAD` in `clients/app/src/lib.rs`): its own plugin's files and the UI library, which are all
      // here, and nothing from anywhere else.
      "content-security-policy": `default-src 'none'; script-src ${url.origin} 'unsafe-inline'; style-src ${url.origin} 'unsafe-inline'; img-src ${url.origin} data:; media-src ${url.origin} data:; font-src ${url.origin} data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri ${url.origin}`,
      // The page's origin is its own, so what it loads from here is from elsewhere to it.
      "access-control-allow-origin": "*",
      "cache-control": "no-store",
    },
  });
}
