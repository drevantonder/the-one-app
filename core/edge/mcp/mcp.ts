// The network's front door: its MCP endpoint over the web (`/mcp`, or in code mode `/mcp?code`), what tells an
// assistant how to come in (`/llms.txt`), the skills of whoever signed in (`/skills/<path>`), the web client's apps'
// files (`/apps`), and the endpoint for whoever holds a sharing link (`/link/<link>`). Each call is passed through the relay to a server acting for whoever it's from, which is told
// which device that is and decides whose it is itself.
//
// One of the network's own devices signs what it asks with its key, which is checked here. Anyone else has signed in,
// and who they are is the app's to say that signs people in: it's bound here as WHO, by `edge.js`, and asked
// `who(request)` (`apps/sign-in/edge/worker.ts`). With no such app, only devices signing with their key come in.
import { verifiedKey, unsigned, hex, NETWORK } from "../ask.ts";
import { randomBytes } from "@noble/hashes/utils.js";
import { reader, servers as acting } from "../presigner/presigner.ts";
import { forward as relayed, notify } from "../relay/relay.ts";
import rules from "../../server/priv/rules.md";

/** Who a request is from, as the app that signs people in says. */
interface Who {
  /** The device it is to a server. */
  device: string;
  /** Whose that device is, as relation tuples name them: `users/<name>`. */
  who: string;
  /** An assistant's: the `WWW-Authenticate` to answer once a server says its device is nobody's. */
  challenge?: string;
  /** A browser's: its session, as Devices lists it, where its apps' files are, and its session's cookies, renewed. */
  session?: string;
  apps?: string;
  cookies?: string[];
}

declare global {
  interface Env {
    /** The app that signs people in: who a request is from, or the answer for one from nobody signed in. */
    WHO?: { who(request: Request): Promise<Who | { refused: Response }> };
  }
}

const refuse = (status: number, why: string) => Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

/** A sharing link starts with whose it leads to, as their prefix: that's whose servers answer its holder. */
const LEADS_TO = /^(?:(?:users|groups)\/[^/]+\/|network\/)/;
/** The JSON-RPC error a server answers a device that's nobody's (`OneCore.Mcp`). */
const NOBODY = -32003;
/** What's said to whoever isn't one of the network's devices, with nothing here to sign them in. */
const NO_SIGN_IN = "Nothing signs people in on this network, so only its own devices come in, signing what they ask with their key";

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (url.pathname === "/llms.txt") return new Response(llms(url.origin), { headers: { "content-type": "text/markdown; charset=utf-8" } });
    if (url.pathname.startsWith("/skills/")) return skill(request, env, url);
    if (url.pathname === "/apps") return apps(request, env);
    if (url.pathname.startsWith("/apps/")) return file(request, env, url, ctx);
    // MCP's streamable HTTP: each message is a POST, answered with JSON, or a stream of its progress and then its
    // answer. There's no stream to GET.
    if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
    if (url.pathname === "/mcp") return mcp(request, env, ctx);
    if (url.pathname.startsWith("/link/")) return link(request, env, url, ctx);
    return new Response(null, { status: 404 });
  },
} satisfies ExportedHandler<Env>;

/**
 * What an assistant reads to come into the network at `origin`, as llmstxt.org has it. Anyone may read it, so it
 * says nothing of what a network has: which apps, and their skills, are for whoever signed in.
 */
const llms = (origin: string) => `# ${new URL(origin).host}

> A network of the One App: a person's, a family's or a workplace's email, messages, calendar, files, devices and more,
> each an app. An assistant comes in as the person who lets it in, and does what they can, through MCP or the
> \`one\` command.

## Rules

${rules.trim()}

## Connecting for a person

When someone asks you to connect to this network, do it yourself, and tell them only what they need to do, one step
at a time, in everyday words.

- If you can run commands (Claude Code, Codex, OpenCode, Cursor): run \`one --version\`. If it answers, you're on one
  of their computers already: use \`one\`, and add MCP as below. A computer that isn't in their network yet says so,
  with a link and a code: give them the link, ask them to check it shows that code, and carry on once they've let it
  in. If there's no \`one\`, add MCP over the web as below instead.
- If you can't (ChatGPT, Claude and other chat apps): you can't add it yourself, so show them where their app adds a
  connector (a custom MCP server) and have them add \`${origin}/mcp\`. They sign in when it asks, with their passkey,
  and then you act as them.

## Adding MCP

There are two ways in, and you choose the one that fits you, or tell the person in one line which you're adding and why:

- Every tool, and \`skills\`. For an agent whose client searches tools itself: Claude Code, OpenCode, Codex, Cursor.
  On one of their computers, \`one mcp\` over stdio (\`claude mcp add --scope user one -- one mcp\`); anywhere else,
  \`${origin}/mcp\` over streamable HTTP (\`claude mcp add --transport http one ${origin}/mcp\`).
- Code mode, as Executor's: \`search\` finds what's possible, \`call\` calls one tool or several at once, \`execute\`
  runs a short script where one call's answer feeds the next, and \`skills\` reads the skills. For every other agent.
  \`one mcp --code\`, or \`${origin}/mcp?code\`.

## Finding your way

- Each app has a skill that says when and how to use its tools: read it before them. The \`skills\` tool lists
  and reads them, as do MCP's \`skills/list\` and \`resources/read\`, under \`skill://\`, and \`one <app>\` prints one
  with its tools. Whoever signed in reads them at \`${origin}/skills/<path>\` too, as
  \`${origin}/skills/building-apps/SKILL.md\`.
- \`one search "<words>"\`, and code mode's \`search\`, find the tools for a task across every app, each with how to
  call it and its app's skill. Before saying you can't, search: an app they haven't installed yet may do it, and
  search offers it.
- Every MCP tool is also a command: \`gmail_search\` is \`one gmail search\`. \`one run <file>\` runs a script as
  \`execute\` does.
- When the person asks for an app, tracker or tool, make it here as an app, where it's on all their devices: the
  skill building-apps says how.
`;

/**
 * A file of a skill, `/skills/<path>/<file>` for `skill://<path>/<file>`, for whoever signed in or one of the
 * network's own devices, as a server reads it for them. Skills say what apps someone has, so nobody else reads one.
 */
async function skill(request: Request, env: Env, url: URL) {
  if (request.method !== "GET") return new Response(null, { status: 405, headers: { allow: "GET" } });
  const read = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "resources/read", params: { uri: `skill://${url.pathname.slice("/skills/".length)}` } });
  let answer: Response;
  if (request.headers.has("one-key")) {
    const device = await verifiedKey(request, "");
    if (!device) return new Response("Sign what you ask with your key", { status: 401 });
    answer = await forward(env, read, { device }, [NETWORK]);
  } else {
    const who = await signedIn(request, env);
    if ("refused" in who) return new Response("Sign in to read a skill", { status: 401 });
    answer = await forward(env, read, { device: who.device }, [`${who.who}/`, NETWORK]);
  }
  if (answer.status !== 200) return answer;
  const response = await answer.json<{ result?: { contents: { text: string }[] }; error?: { code?: number; message: string } }>();
  const text = response.result?.contents?.[0]?.text;
  if (text === undefined) return new Response(response.error?.message ?? "There's no such skill", { status: response.error?.code === NOBODY ? 401 : 404 });
  return new Response(text, { headers: { "content-type": "text/markdown; charset=utf-8", "cache-control": "no-store" } });
}

/** Who signed in, as the app that signs people in says from what `request` carries, its body left here. */
async function signedIn(request: Request, env: Env) {
  if (!env.WHO) return { refused: refuse(401, NO_SIGN_IN) };
  return env.WHO.who(new Request(request.url, { method: request.method, headers: request.headers }));
}

async function mcp(request: Request, env: Env, ctx: ExecutionContext) {
  // One of the network's own devices signs what it asks with its key. One that's nobody's is told so by the
  // server, and asks for a code to be let in by.
  if (request.headers.has("one-key")) {
    const body = await request.text();
    const device = await verifiedKey(request, body);
    if (!device) return unsigned(request);
    return forward(env, body, { device }, [NETWORK], request.headers, mode(new URL(request.url)), { signal: request.signal, ctx });
  }
  // A browser or an assistant someone let in. One that's since become nobody's signs in, or is let in, again.
  const who = await signedIn(request, env);
  if ("refused" in who) return who.refused;
  const body = await request.text();
  const answer = await forward(env, body, { device: who.device }, [`${who.who}/`, NETWORK], request.headers, mode(new URL(request.url)), { signal: request.signal, ctx });
  if (!(await nobodys(answer))) return withCookies(!who.session && who.apps ? await drawn(answer, body, new URL(request.url).origin, who.apps) : answer, who.cookies);
  return who.challenge ? new Response(null, { status: 401, headers: { "www-authenticate": who.challenge } }) : refuse(401, "Sign in again");
}

/**
 * An app's page read by an assistant that shows it, as an MCP App, in a frame of its own: drawn by the renderer, as the
 * web client draws it, in the renderer's frame from here (`clients/web/renderer`), which loads it from where the
 * assistant's apps' files are (`apps`), in a frame of its own origin, as the web client's is. The page the assistant
 * shows only passes messages between them, as the renderer is WebAssembly, which an MCP App's own page may not compile,
 * and says the frame's origin is what it frames (`frameDomains`). Any other answer goes as it is.
 */
async function drawn(answer: Response, body: string, origin: string, apps: string) {
  if (answer.headers.get("content-type") !== "application/json") return answer;
  const uri = (JSON.parse(body) as Call).params?.uri;
  const read = await answer.clone().json<{ result?: { contents?: { uri: string; mimeType: string; text?: string; _meta?: object }[] } }>();
  const [content] = read.result?.contents ?? [];
  if (typeof uri !== "string" || content?.mimeType !== "text/html;profile=mcp-app" || content.uri !== uri) return answer;
  const [, , app, version, ...path] = uri.split("/");
  const frame = `${origin}/renderer/#${origin}${apps}/${app}/${app}@${version}/${path.join("/")}`;
  const page = `<!doctype html>
<meta charset="utf-8">
<style>html, body { margin: 0; height: 100%; overflow: hidden; } iframe { display: block; border: 0; width: 100%; height: 100%; }</style>
<iframe src="${frame}" sandbox="allow-scripts allow-forms" allow="clipboard-write"></iframe>
<script>
  const frame = document.querySelector("iframe");
  addEventListener("message", ({ source, data }) => {
    if (source === frame.contentWindow) {
      // A host sizes a page by what it says, which a page of ours says only where it's asked to (\`sizeChanged\`).
      if (data?.method === "ui/notifications/initialized") parent.postMessage({ jsonrpc: "2.0", method: "ui/notifications/size-changed", params: { height: ${HEIGHT} } }, "*");
      parent.postMessage(data, "*");
    } else if (source === parent) frame.contentWindow.postMessage(data, "*");
  });
</script>
`;
  const ui = { csp: { frameDomains: [origin] } };
  return Response.json({ ...read, result: { ...read.result, contents: [{ ...content, text: page, _meta: { ...content._meta, ui } }] } });
}

/** How tall an MCP App's page is in an assistant's chat, as its renderer's page says nothing of it. */
const HEIGHT = 600;

/** How an agent calls tools at `/mcp`: in code mode with `?code` (`OneCore.Mcp`), and otherwise as MCP's tools. */
const mode = (url: URL) => (url.searchParams.has("code") ? "code" : undefined);

/**
 * Whoever holds a sharing link needs no signing in: the link is all they are. What they ask says it's from the
 * link's holder and nothing else, whatever it said of itself, and a server refuses them everything the link's
 * relation tuple doesn't give. Whose a request is, is a server's to say: one acting for whoever the link leads to,
 * or for the network.
 */
async function link(request: Request, env: Env, url: URL, ctx: ExecutionContext) {
  const link = decodeURIComponent(url.pathname.slice("/link/".length));
  const of = LEADS_TO.exec(link)?.[0];
  if (!of) return refuse(404, "That isn't a sharing link");
  return forward(env, await request.text(), { link }, [of, NETWORK], request.headers, undefined, { signal: request.signal, ctx });
}

/** What MCP says of a request in its `_meta`, under its own prefix: its version, and its client and what that can do. */
const MCP = "io.modelcontextprotocol/";
const VERSION = `${MCP}protocolVersion`;
/** How often a request with a `progressToken` is told it's still running, as a server tells one (`client::THERE`). */
const PROGRESS = 1_000;

type Call = { id?: unknown; method?: unknown; params?: { _meta?: Record<string, unknown>; name?: unknown; uri?: unknown; taskId?: unknown } };

/**
 * Passes one JSON-RPC message to a server acting for one of `of`, as from `from`, a device or a sharing link's
 * holder, and answers what the server did. What's asked says who it's from, who it's for and which app's page asked
 * (a device's), the span it's under and what MCP has it say, and nothing else of itself. `headers` are the HTTP request's, if it's one, and
 * `client` its client's connection, whose going before its answer the servers are told.
 */
async function forward(
  env: Env,
  message: string,
  from: { device: string } | { link: string },
  of: string[],
  headers?: Headers,
  mode?: string,
  client?: { signal: AbortSignal; ctx: ExecutionContext },
) {
  let call: Call;
  try {
    call = JSON.parse(message);
    if (typeof call?.method !== "string") throw new Error("not a request");
  } catch {
    return refuse(400, "That isn't a JSON-RPC request");
  }
  const meta = call.params?._meta ?? {};
  const modern = headers && (VERSION in meta || (headers.get("mcp-protocol-version") ?? "") >= "2026-07-28");
  const mismatch = modern && mismatched(headers, call);
  if (mismatch) return Response.json({ jsonrpc: "2.0", id: null, error: { code: -32020, message: `Header mismatch: ${mismatch}` } }, { status: 400 });
  const subject = "device" in from ? meta.subject : undefined;
  // Which app's page asked, as the window's page says of each (`clients/app/page/bridge.js`), which a server holds
  // that page to (`OneCore.Mcp`).
  const app = "device" in from ? meta.app : undefined;
  const traceparent = traced(meta.traceparent ?? headers?.get("traceparent") ?? undefined);
  const mcp = Object.fromEntries(Object.entries(meta).filter(([key]) => key.startsWith(MCP) || key === "progressToken"));
  const forwardedMeta = { ...mcp, ...from, traceparent, ...(typeof subject === "string" && { subject }), ...(typeof app === "string" && { app }), ...(mode && { mode }) };
  message = JSON.stringify({ ...call, params: { ...call.params, _meta: forwardedMeta } });
  const lists = reader(env);
  const servers = [...new Set((await Promise.all(of.map((of) => acting(lists, of)))).flat())];
  const forwarded = relayed(env, servers, message);
  // A client that goes away before its answer is said to have, by the request's id: what a call does then is its own
  // (`OneCore.Call.State.gone/1`). MCP's Streamable HTTP has a client cancel with `notifications/cancelled`, which is
  // passed on as any message is.
  if (client && "id" in call) {
    const gone = { jsonrpc: "2.0", method: "notifications/one/gone", params: { requestId: call.id, _meta: forwardedMeta } };
    const went = () => client.ctx.waitUntil(notify(env, JSON.stringify(gone)));
    client.signal.addEventListener("abort", went, { once: true });
    forwarded.finally(() => client.signal.removeEventListener("abort", went));
  }
  // A request with a progress token from a client that takes a stream is told every second that it's still running,
  // as MCP's streamable HTTP has it: once it's run that long, it's answered with a stream of its progress and then
  // its answer, however long it runs. One answered sooner is answered as any other.
  const token = meta.progressToken;
  const streams = "id" in call && (typeof token === "string" || typeof token === "number") && headers?.get("accept")?.includes("text/event-stream");
  if (streams && (await Promise.race([forwarded.then(() => false), scheduler.wait(PROGRESS).then(() => true)]))) return streamed(forwarded, call.id, token);
  const reached = await forwarded;
  if (!reached) return refuse(503, "None of this network's servers answered. Try again once one is on.");
  // A notification has no answer.
  if (reached.answer === "") return new Response(null, { status: 202 });
  // From MCP's 2026-07-28, a version or method a server doesn't have is said in the status too.
  const code = modern && (JSON.parse(reached.answer) as { error?: { code?: number } }).error?.code;
  const status = code === -32022 ? 400 : code === -32601 ? 404 : 200;
  return new Response(reached.answer, { status, headers: { "content-type": "application/json" } });
}

/**
 * The answer to the request `id`, which carries `token`, as a stream of server-sent events: MCP's progress
 * notification every second while it runs, how far it is being the seconds it's run, as a server says it, and then
 * the answer. Whoever asked closing it stops the telling, and the servers are told it went (`forward`).
 */
function streamed(forwarded: Promise<{ answer: string } | null>, id: unknown, token: string | number) {
  let ran = 1;
  const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>();
  const stream = writable.getWriter();
  const send = (message: string) => stream.write(new TextEncoder().encode(`event: message\ndata: ${message}\n\n`));
  const progress = () => send(JSON.stringify({ jsonrpc: "2.0", method: "notifications/progress", params: { progressToken: token, progress: ran++ } }));
  // A stream that can't be written to has ended, and the write of the answer below says so.
  const telling = setInterval(() => progress().catch(() => clearInterval(telling)), PROGRESS);
  (async () => {
    try {
      await progress();
      const reached = await forwarded;
      const none = { jsonrpc: "2.0", id, error: { code: -32000, message: "None of this network's servers answered. Try again once one is on." } };
      await send(reached?.answer ?? JSON.stringify(none));
    } finally {
      clearInterval(telling);
      await stream.close();
    }
  })();
  return new Response(readable, { headers: { "content-type": "text/event-stream", "cache-control": "no-store", "x-accel-buffering": "no" } });
}

/**
 * Which of the headers MCP's streamable HTTP has a request carry, from 2026-07-28, doesn't say what its body does, if
 * one doesn't. A name that isn't plain ASCII is sent in base64, between `=?base64?` and `?=`. A task's request names
 * its task, as MCP's Tasks have it.
 */
function mismatched(headers: Headers, call: Call) {
  const tasked = ["tasks/get", "tasks/update", "tasks/cancel"].includes(call.method as string);
  const named = tasked || ["tools/call", "resources/read", "prompts/get"].includes(call.method as string);
  const sent = headers.get("mcp-name");
  const encoded = sent && /^=\?base64\?(.*)\?=$/.exec(sent);
  const name = encoded ? new TextDecoder().decode(Uint8Array.from(atob(encoded[1]), (c) => c.charCodeAt(0))) : sent;
  const expected: [string, unknown, string | null][] = [
    ["MCP-Protocol-Version", call.params?._meta?.[VERSION], headers.get("mcp-protocol-version")],
    ["Mcp-Method", call.method, headers.get("mcp-method")],
    ...(named ? [["Mcp-Name", tasked ? call.params?.taskId : (call.params?.name ?? call.params?.uri), name] as [string, unknown, string | null]] : []),
  ];
  const wrong = expected.find(([, body, header]) => header === null || body !== header);
  return wrong && `${wrong[0]} header value '${wrong[2] ?? ""}' does not match body value '${wrong[1] ?? ""}'`;
}

/**
 * The `traceparent` of this request's span, as W3C's Trace Context has it: under `parent`, the client's, which it
 * sends in `_meta` as MCP has it or as a header, if any. It's said in a line Telemetry's Tail Worker reads
 * (`apps/telemetry/edge/worker.ts`), and carried to the server.
 */
function traced(parent: unknown) {
  const random = (bytes: number) => hex.encode(randomBytes(bytes));
  const under = typeof parent === "string" ? /^00-([0-9a-f]{32})-[0-9a-f]{16}-[0-9a-f]{2}$/.exec(parent) : null;
  const traceparent = `00-${under?.[1] ?? random(16)}-${random(8)}-01`;
  console.log(JSON.stringify({ telemetry: { traceparent, parent: under?.[0] } }));
  return traceparent;
}

/** Whether a server answered that the device is nobody's. The answer is left to be sent on if it isn't. */
async function nobodys(answer: Response) {
  // A stream ran past what a server says of a device at once.
  if (answer.status !== 200 || answer.headers.get("content-type") !== "application/json") return false;
  const response = await answer.clone().json<{ error?: { code?: number } }>();
  return response.error?.code === NOBODY;
}

/**
 * Where a signed-in browser's apps' files are, `/apps/<its session, signed>`, and the device it is: its session, as
 * Devices lists it, and its key, as a server knows it, which it fills a tool's `device` input in with. An app's page has an origin of its own in the web client, so its files are asked for with no
 * cookie, and the signed session in their address says whose they are.
 */
async function apps(request: Request, env: Env) {
  const who = await signedIn(request, env);
  if ("refused" in who || !who.apps) return Response.json({ error: "Sign in" }, { status: 401 });
  return withCookies(Response.json({ apps: who.apps, device: who.session, key: who.device }), who.cookies);
}

/** `answer`, with the cookies of a browser's session the app that signs people in renewed, if it did. */
function withCookies(answer: Response, cookies: string[] = []) {
  for (const cookie of cookies) answer.headers.append("set-cookie", cookie);
  return answer;
}

/**
 * What an app's page loads in the web client: `/apps/<session, signed>/<app>/<app>/<path>` is `path` among the
 * latest files of `app` that a server lists for that session's person, as the app's client has them at its own
 * address for each app (`clients/app/src/lib.rs`). An app opened for a group or the network says so after its name,
 * last part first (`notes.family.groups`).
 *
 * `<app>@<version>` in place of the second `<app>` is that version's file, which never changes, so it's kept in
 * Cloudflare's cache for whoever asked and answered from there, with no server asked. A page asked for by its app's
 * name alone loads the rest by its version, as its `<base>` says: its frame's origin is its own and opaque, which
 * browsers give no HTTP cache, so this cache is the only one its files have.
 */
async function file(request: Request, env: Env, url: URL, ctx: ExecutionContext) {
  const [session, host, named, ...path] = url.pathname.slice("/apps/".length).split("/");
  const [app, pinned] = named.split("@");
  const who = await signedIn(request, env);
  if ("refused" in who) return new Response("Sign in again", { status: 401 });
  const [, ...whose] = host.split(".");
  const meta = whose.length ? { _meta: { subject: whose.reverse().join("/") } } : {};
  const ask = async <Result,>(method: string, params: object): Promise<Result | null> => {
    const answer = await forward(env, JSON.stringify({ jsonrpc: "2.0", id: 1, method, params: { ...params, ...meta } }), { device: who.device }, [`${who.who}/`, NETWORK]);
    if (answer.status !== 200) return null;
    return (await answer.json<{ result: Result }>()).result;
  };
  const listed = pinned ? null : await ask<{ resources: { uri: string }[] }>("resources/list", {});
  const version = pinned ?? listed?.resources.map(({ uri }) => uri.split("/")).find(([scheme, , of]) => scheme === "ui:" && of === app)?.[3];
  if (!version) return new Response(`There's nothing at ${url.pathname}`, { status: 404 });
  // Kept for whoever it was read for, as a server may answer another person otherwise.
  const kept = new Request(`${url.origin}/apps/${who.who}/${host}/${app}@${version}/${path.join("/")}`);
  let found = await caches.default.match(kept);
  if (!found) {
    const read = await ask<{ contents: { blob?: string; text?: string; mimeType: string }[] }>("resources/read", { uri: `ui://${app}/${version}/${path.join("/")}` });
    const content = read?.contents?.[0];
    if (!content) return new Response(`There's nothing at ${url.pathname}`, { status: 404 });
    const body = content.blob ? Uint8Array.from(atob(content.blob), (char) => char.charCodeAt(0)) : content.text;
    found = new Response(body, { headers: { "content-type": content.mimeType.replace(";profile=mcp-app", ""), "cache-control": "public, max-age=31536000, immutable" } });
    ctx.waitUntil(caches.default.put(kept, found.clone()));
  }
  const headers = {
    "content-type": found.headers.get("content-type") ?? "",
    // What an app's page may load, which is what an MCP App may by default and what the app lets one
    // (`APP_MAY_LOAD` in `clients/app/src/lib.rs`): its own app's files and the UI library, which are all
    // here, and nothing from anywhere else; and in a frame, another app's page, which is here too (`Frame`).
    "content-security-policy": `default-src 'none'; script-src ${url.origin} 'unsafe-inline'; style-src ${url.origin} 'unsafe-inline'; img-src ${url.origin} data:; media-src ${url.origin} data:; font-src ${url.origin} data:; connect-src 'none'; frame-src ${url.origin}; object-src 'none'; base-uri ${url.origin}`,
    // The page's origin is its own, so what it loads from here is from elsewhere to it.
    "access-control-allow-origin": "*",
    // Its address has its session in it, so it's this browser's alone. A page asked for by its app's name is
    // whichever version is latest.
    "cache-control": pinned ? "private, max-age=31536000, immutable" : "no-store",
  };
  if (pinned || !headers["content-type"].startsWith("text/html")) return new Response(found.body, { headers });
  // Before anything in it that has an address, its import map too, but after its doctype, which a browser reads first.
  const base = `<base href="${url.origin}/apps/${session}/${host}/${app}@${version}/${path.slice(0, -1).map((part) => `${part}/`).join("")}">`;
  const page = (await found.text()).replace(/^(\s*<!doctype html>)?/i, (doctype) => doctype + base);
  return new Response(page, { headers });
}
