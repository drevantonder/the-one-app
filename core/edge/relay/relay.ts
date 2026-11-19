// The relay: passes traffic between a network's servers and clients when they can't reach each other directly. It
// speaks iroh's relay protocol, version 2 (iroh-relay 1.4's `protos`: a WebSocket of binary frames, each a type and
// its fields), to iroh's own endpoints. What it passes is QUIC, encrypted between the two keys, so it can't read it.
//
// An endpoint's connection is held by the Worker it reached, where waiting costs nothing: there it proves its key,
// and iroh's question every 15 seconds, whether the relay is still there, is answered. Once proved, it's joined to
// the one Durable Object that knows every endpoint, which passes on what one sends another. That object sleeps with
// its connections open (hibernation), and is woken only by something to pass on: nothing an idle network says
// reaches it. It admits only the network's own, by asking the presigner about each key once that key has proved
// itself.
//
// It also passes a call that came in at the address over the web to a server, and tells every server what a piece
// here has to say: a server holds one more connection here for those (`/relay/outside`), since nothing else at the
// edge holds one.
//
// The rest of the edge calls it as functions (`forward`, `notify`, `presence`), and an app's part through
// `RelayEntrypoint`, which `edge.js` binds it to as RELAY.
//
// And it knows who's here: a server or a device is connected while it's connected here, through any run of its
// client, and otherwise was last seen when its last connection ended, which the Durable Object keeps. A connection
// that says nothing for longer than iroh's pings allow has ended, as a machine that sleeps or loses its network says
// nothing. A server says what it is as that changes (`OneCore.Release`): its `server_membership`, whether it's one of
// the network's servers, and its `server_daemon_status`, whether its program is starting, running or stopping. It's
// ready once it's active and running, and connected, and only a ready server is sent calls. Servers are told when
// one of them comes or goes (`notifications/one/seen`), and ask (`/relay/seen`).
//
// It's how the network's updater (`updater/updater.ts`) hears and speaks: it tells it what each server says of itself
// and as one comes or goes, and a build published and Update Now (`/relay/update`), and tells every server what the
// updater answers it's to do.
//
// Parts of the platform talk where the protocols they speak overlap (`OneCore.Protocol`): a server says the range it
// speaks, and its build, as it says what it is. A call from outside goes only to a server that talks with the edge,
// and says the edge's protocol and build: a rule apart from being ready.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
/** The protocols the edge speaks, and the cutoff for parts too old to say theirs (`core/protocol.json`). */
import PROTOCOL_CONFIG from "../../protocol.json";
import { blake3 } from "@noble/hashes/blake3.js";
import { verifiedKey, hex, signedBy } from "../ask.ts";
import { admits, reader } from "../presigner/presigner.ts";
import type { Input, Reported } from "../updater/updater.ts";

const PROTOCOL = "iroh-relay-v2";
/** What an endpoint signs to prove its key: a key derived from the relay's challenge under this name. */
const CHALLENGE = "iroh-relay handshake v1 challenge signature";
/** The frames, by their first byte. A datagram frame is passed on as the type two above it. */
const SERVER_CHALLENGE = 0;
const CLIENT_AUTH = 1;
const SERVER_CONFIRMS_AUTH = 2;
const SERVER_DENIES_AUTH = 3;
const CLIENT_TO_RELAY_DATAGRAM = 4;
const CLIENT_TO_RELAY_DATAGRAM_BATCH = 5;
const PING = 9;
const PONG = 10;
/** How long an endpoint may say nothing before it's taken as gone: iroh pings its relay every 15 seconds. */
const SILENT = 45_000;
/** What a server is told when one of the network's servers comes or goes: to ask who's on again. */
const SEEN = JSON.stringify({ jsonrpc: "2.0", method: "notifications/one/seen" });
/**
 * How long a server has to answer a ping before a call goes to another: a server that's registered here may be a
 * machine that has just gone to sleep or is restarting, whose connection hasn't ended yet. It's well inside what an
 * MCP client waits for (Claude's 30 seconds), and long enough for a server across the world. How long the call
 * itself then takes is its tool's.
 */
const REACH = 5_000;
/** MCP's ping, which a server answers at once, whoever asks. */
const MCP_PING = JSON.stringify({ jsonrpc: "2.0", id: 0, method: "ping" });
/**
 * How long a call to the relay's Durable Object is tried again, in milliseconds. A deploy leaves the object on the
 * code before while it's busy, up to `max_delay` in `wrangler.jsonc`, and a Worker on either side of the deploy may
 * call it meanwhile, as Cloudflare says (Durable Objects' "Code updates"): a call the other doesn't take yet, or no
 * longer, fails until it's switched over. That's also what a reset or a broken connection to it fails with, which
 * Cloudflare has callers try again, on a new stub, waiting longer each time, here up to a second, so a try comes soon
 * after it's switched over. An overloaded one isn't tried again. It's within what a server waits for an answer,
 * 15 seconds.
 */
const SWITCH = 12_000;
/** The protocols a part speaks, from `oldest` to `newest` (`core/protocol.json`). */
type Protocol = { oldest: number; newest: number };
/** The protocols the edge speaks, from `oldest` to `newest`. */
const SPEAKS: Protocol = { oldest: PROTOCOL_CONFIG.oldest, newest: PROTOCOL_CONFIG.newest };
/** The protocols a part speaks, as it `said`: none known for one that said none, as `OneCore.Protocol.of/1` says. */
function spoken(said: unknown): Protocol | undefined {
  const { oldest, newest } = (said ?? {}) as Partial<Protocol>;
  return Number.isInteger(oldest) && Number.isInteger(newest) && oldest! <= newest! ? { oldest: oldest!, newest: newest! } : undefined;
}
/** Whether a part that speaks `theirs` talks with the edge: their ranges overlap. */
const talk = (theirs: Protocol | undefined) => !!theirs && theirs.oldest <= SPEAKS.newest && SPEAKS.oldest <= theirs.newest;
/** What a range is, said: `1`, `1 to 2`, or `none`. */
const named = (protocol: Protocol | undefined) => (!protocol ? "none" : protocol.oldest === protocol.newest ? `${protocol.oldest}` : `${protocol.oldest} to ${protocol.newest}`);

/** A W3C `traceparent`: its trace and span. */
const TRACE = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/;

type ServerMembership = "inactive" | "joining" | "active" | "leaving";
type ServerDaemonStatus = "starting" | "running" | "stopping" | "stopped";
type ServerReadyReason = "joining" | "starting" | "draining" | "stopped";
const MEMBERSHIPS: ServerMembership[] = ["joining", "active", "leaving"];
const SAID: ServerDaemonStatus[] = ["starting", "running", "stopping"];

/**
 * How long a server that isn't connected is still starting or stopping as it said, in milliseconds: one says it's
 * starting just before it connects, and one stopping to start again, to take an update, is starting again within a
 * minute. Past that it's stopped.
 */
const STARTING = 60_000;
const STOPPING = 60_000;

/** What a server says of its update, which the relay passes to the network's updater. */
const UPDATE: (keyof Reported)[] = [
  "name",
  "installed",
  "server_update_status",
  "next_build",
  "program_name",
  "program",
  "next_program",
  "went_back",
];

/** What a server last said it is (`/relay/seen`), and when. */
interface ServerStatus {
  server_membership: ServerMembership;
  server_daemon_status: ServerDaemonStatus;
  at: number;
}

/**
 * Whether a server or a device is here: `connected` now, or when it was `last_seen` (ms since 1970). A server's
 * `server_membership` and `server_daemon_status` too, the daemon stopped once it's gone, and from those whether it's
 * `server_ready`, active and running, or the reason it isn't.
 */
interface Presence {
  connected: boolean;
  last_seen?: number;
  server_membership?: ServerMembership;
  server_daemon_status?: ServerDaemonStatus;
  server_ready?: boolean;
  server_ready_reason?: ServerReadyReason;
  /** Whether it answered a ping within `REACH`, where `/relay/seen` was asked to ping it. */
  answered?: boolean;
}

/** An endpoint joined to the relay: the connection its datagrams are passed over, and when its token expires. */
interface Joined {
  relay: WebSocket;
  until?: number;
  key: string;
}

/** What a connection to the Durable Object is: an endpoint's, a server's for outsiders' calls, or one call's. */
interface Attachment {
  key?: string;
  device?: string;
  server?: boolean;
  outside?: string;
  call?: string;
  to?: string;
}

const utf8 = new TextEncoder();
const refuse = (status: number, why: string) => new Response(why, { status });

/**
 * Whoever signed `request` with its `body`, if the network admits them, as `admitted` (`{until}` for a device's run
 * of a client, whose token expires), with what the body says, JSON's, or `{}` with none; or the refusal they're
 * answered: 403 for one the network doesn't know, and 400 for a body that isn't JSON.
 */
async function admit(env: Env, request: Request, body: string) {
  const key = await verifiedKey(request, body);
  const admitted = key && (await admits(reader(env), key, request.headers.get("one-token")));
  if (!key || !admitted) return refuse(403, "This network doesn't know you");
  try {
    return { key, admitted, said: JSON.parse(body || "{}") as Record<string, unknown> };
  } catch {
    return refuse(400, "What was sent isn't JSON");
  }
}

export default {
  async fetch(request: Request, env: Env) {
    const url = new URL(request.url);
    // iroh times this to learn how far away the relay is. An endpoint with no answer has no relay.
    if (url.pathname === "/ping") return new Response(null, { status: 200 });
    if (url.pathname === "/relay/seen") return handleSeen(env, request, url);
    if (url.pathname === "/relay/update") return handleUpdate(env, request);
    if (request.headers.get("upgrade") !== "websocket") return refuse(url.pathname.startsWith("/relay") ? 426 : 404, "The relay is a WebSocket at /relay");
    if (url.pathname === "/relay") return endpoint(env, request, url);
    if (url.pathname === "/relay/outside") return relay(env, (relay) => relay.fetch(request));
    return refuse(404, "The relay is at /relay");
  },
} satisfies ExportedHandler<Env>;

/** An iroh endpoint connects: it's sent a challenge, and is nobody until it signs it. */
function endpoint(env: Env, request: Request, url: URL) {
  const speaks = (request.headers.get("sec-websocket-protocol") ?? "").split(",").map((protocol) => protocol.trim());
  if (!speaks.includes(PROTOCOL)) return refuse(400, `The relay speaks ${PROTOCOL}`);
  // iroh shows its token as a bearer token, and in the URL from a browser, which can't set a header.
  const token = /^Bearer (.+)$/.exec(request.headers.get("authorization") ?? "")?.[1] ?? url.searchParams.get("token");
  const challenge = crypto.getRandomValues(new Uint8Array(16));
  const [theirs, ours] = Object.values(new WebSocketPair());
  ours.accept();
  ours.binaryType = "arraybuffer";
  // What it sends before it's joined waits for that, in the order it was sent.
  let joined: Promise<Joined | undefined> | undefined;
  let received = Date.now();
  const quiet = setInterval(async () => {
    if (Date.now() - received >= SILENT) {
      clearInterval(quiet);
      ours.close(1001, "It went quiet");
      return void (await joined)?.relay.close(1001, "It went quiet");
    }
    // A server taken off its list, or whose device's token no longer admits it, is cut: what's open isn't the network's.
    const { key } = (await joined) ?? {};
    if (key && !(await admits(reader(env), key, token))) ours.close(1008, "Not one of this network's");
  }, SILENT / 3);
  ours.addEventListener("close", () => clearInterval(quiet));
  ours.addEventListener("message", async ({ data }) => {
    received = Date.now();
    if (typeof data === "string") return ours.close(1003, "The relay takes binary frames");
    const frame = new Uint8Array(data);
    if (!joined) return void (joined = join(env, ours, challenge, token, frame));
    const { relay, until } = (await joined) ?? {};
    if (!relay) return;
    if (until !== undefined && until < Date.now()) return ours.close(1008, "Its token expired");
    if (frame[0] === PING) {
      frame[0] = PONG;
      ours.send(frame);
    } else if (frame[0] === CLIENT_TO_RELAY_DATAGRAM || frame[0] === CLIENT_TO_RELAY_DATAGRAM_BATCH) {
      if (frame.length < 33) return ours.close(1002, "A datagram names who it's for");
      relay.send(frame);
    }
  });
  ours.send(Uint8Array.of(SERVER_CHALLENGE, ...challenge));
  return new Response(null, { status: 101, webSocket: theirs, headers: { "sec-websocket-protocol": PROTOCOL } });
}

/**
 * The answer to a challenge: the endpoint's key, and its signature of the challenge. One of the network's own is
 * joined to the relay: `{relay, until}`, the connection there that its datagrams are passed over, and when its
 * token expires if it does.
 */
async function join(env: Env, ours: WebSocket, challenge: Uint8Array, token: string | null, frame: Uint8Array): Promise<Joined | undefined> {
  // The frame is postcard's: the type, the key's 32 bytes, then the signature as bytes with their length before.
  const key = hex.encode(frame.subarray(1, 33));
  const proved =
    frame.length === 98 &&
    frame[0] === CLIENT_AUTH &&
    frame[33] === 64 &&
    (await signedBy(key, hex.encode(frame.subarray(34)), blake3(challenge, { context: utf8.encode(CHALLENGE) })));
  const admitted = proved && (await admits(reader(env), key, token));
  if (!admitted) {
    const why = utf8.encode("This network doesn't know you");
    ours.send(Uint8Array.of(SERVER_DENIES_AUTH, why.length, ...why));
    return void ours.close(1008, "Not one of this network's");
  }
  // A run of a client is on for the device it runs for, which its token names (the presigner checked it), and a server
  // for itself: whatever else its token says, it's not a device.
  const server = admitted.until === undefined;
  const device = server ? key : ((token ?? "").split(".")[1] ?? key);
  const on = new URLSearchParams({ key, device, ...(server && { server: "yes" }) });
  const joined = (await relay(env, (relay) => relay.fetch(`https://relay/endpoint?${on}`, { headers: { upgrade: "websocket" } }))).webSocket!;
  joined.accept();
  joined.binaryType = "arraybuffer";
  joined.addEventListener("message", ({ data }) => ours.send(data as ArrayBuffer));
  joined.addEventListener("close", () => ours.close(1000, "It connected again"));
  ours.addEventListener("close", () => joined.close(1000, "It left"));
  ours.send(Uint8Array.of(SERVER_CONFIRMS_AUTH));
  return { relay: joined, until: admitted.until, key };
}

/**
 * Passes an outsider's `message` to one of `servers`, by their keys, and gives `{server, answer}`: the first of
 * them to show it's there by answering a ping within `REACH`, or `null` if none does. The wait for the answer is
 * on a connection the relay holds asleep, so nothing is billed for it, and it's as long as the call takes. It's under
 * the span of the request that asked, whose `traceparent` the message carries, and says in that span which server
 * answered, or why none did.
 */
export async function forward(env: Env, servers: string[], message: string): Promise<{ server: string; answer: string } | null> {
  const call = JSON.parse(message);
  message = JSON.stringify({ ...call, params: { ...call.params, _meta: { ...call.params?._meta, protocol: SPEAKS, version: env.VERSION } } });
  const said = (attributes: Record<string, string | number>, error?: string) => console.log(JSON.stringify({ telemetry: { attributes, error } }));

  // Only a server that talks with the edge is asked, and only one that's ready, or isn't known not to be.
  const protocols = await relay(env, (relay) => relay.protocols(servers));
  const talking = servers.filter((server) => talk(protocols[server]));
  if (!talking.length) {
    said({ "one.servers": servers.length }, `None of its ${servers.length} servers speaks a protocol the edge's ${named(SPEAKS)} does: ${servers.map((server) => `${server.slice(0, 8)} ${named(protocols[server])}`).join(", ")}`);
    return null;
  }
  const here = await presence(env, talking);
  servers = talking.filter((server) => here[server]?.server_ready !== false);
  if (!servers.length) {
    said({ "one.servers": talking.length }, `None of its ${talking.length} servers is ready: ${talking.map((server) => `${server.slice(0, 8)} ${here[server]?.server_ready_reason}`).join(", ")}`);
    return null;
  }
  const tries = servers.map((server) => reach(relay(env, (relay) => relay.fetch(`https://relay/call?to=${server}`, { headers: { upgrade: "websocket" } })), server));
  const reached = await Promise.any(tries).catch(() => null);
  for (const tried of tries) tried.then((other) => other !== reached && other.close(), () => {});
  const answer = reached && (await reached.ask(message));
  reached?.close();
  // With none reached, every try has failed, and each says why.
  const why = (tried: Promise<unknown>, at: number) => tried.then(() => "", (error: Error) => `${servers[at].slice(0, 8)} ${error.message}`);
  const error = !reached
    ? `None of its ${servers.length} servers answered: ${(await Promise.all(tries.map(why))).join("; ") || "it has none"}`
    : answer === null
      ? `${reached.server.slice(0, 8)} left before it answered`
      : undefined;
  said({ "one.servers": servers.length, ...(reached && { "one.server": reached.server.slice(0, 8) }) }, error);
  return reached && answer !== null ? { server: reached.server, answer } : null;
}

/** Tells every server that's here `message`, which none answers: a JSON-RPC notification. */
export async function notify(env: Env, message: string): Promise<void> {
  await relay(env, (relay) => relay.fetch("https://relay/notify", { method: "POST", body: message }));
}

/** Whether each of `keys`, servers' and devices', is here (`Presence`), or `null` for one never seen. */
export async function presence(env: Env, keys: string[]): Promise<Record<string, Presence | null>> {
  // Disposed of once it's copied, so the relay's call ends as it answers. Its own keys are copied, not its disposer.
  using presence = await relay(env, (relay) => relay.presence(keys));
  return Object.fromEntries(Object.entries(presence));
}

/**
 * The relay, for an app's part (`RELAY`): `forward`, `notify` and `presence`. A call it forwards is a span of its own,
 * under the part's, which the server's is under in turn (W3C's Trace Context), from here to the answer: the call
 * lives on past it while its connections close, which isn't the call's.
 */
export class RelayEntrypoint extends WorkerEntrypoint<Env> {
  async forward(servers: string[], message: string) {
    const start = Date.now();
    const call = JSON.parse(message);
    const parent: string | undefined = call.params?._meta?.traceparent;
    const traceparent = `00-${TRACE.exec(parent ?? "")?.[1] ?? hex.encode(crypto.getRandomValues(new Uint8Array(16)))}-${hex.encode(crypto.getRandomValues(new Uint8Array(8)))}-01`;
    try {
      return await forward(this.env, servers, JSON.stringify({ ...call, params: { ...call.params, _meta: { ...call.params?._meta, traceparent } } }));
    } finally {
      console.log(JSON.stringify({ telemetry: { traceparent, parent, start, end: Date.now() } }));
    }
  }

  notify(message: string) {
    return notify(this.env, message);
  }

  presence(keys: string[]) {
    return presence(this.env, keys);
  }
}

/**
 * A server or a device of the network's asks who's here, of the keys it names: `?keys=<key>,<key>`, and is told
 * `{"presence": {<key>: <Presence>}, "protocol": <the edge's>}` and the edge's build as `one-version`. A server says
 * what it is with a POST of `{"server_membership": <it>, "server_daemon_status": <it>, "version": <its build>,
 * "protocol": <its>}`, as that changes.
 */
async function handleSeen(env: Env, request: Request, url: URL) {
  const admitted = await admit(env, request, request.method === "POST" ? await request.text() : "");
  if (admitted instanceof Response) return admitted;
  const { key } = admitted;
  if (request.method === "POST") {
    if (admitted.admitted.until !== undefined) return refuse(403, "Only a server says what it is");
    const said = admitted.said as { server_membership?: ServerMembership; server_daemon_status?: ServerDaemonStatus; version?: unknown; protocol?: unknown } & Reported;
    const version = typeof said.version === "string" ? said.version : undefined;
    const protocol = spoken(said.protocol);
    const status = MEMBERSHIPS.includes(said.server_membership!) && SAID.includes(said.server_daemon_status!)
      ? { server_membership: said.server_membership!, server_daemon_status: said.server_daemon_status!, at: Date.now() }
      : undefined;
    const update: Reported = {
      version,
      ...Object.fromEntries(Object.entries(said).filter(([name]) => UPDATE.includes(name as keyof Reported))),
    };
    const error = !talk(protocol) ? `${key.slice(0, 8)} speaks protocol ${named(protocol)} and the edge ${named(SPEAKS)}, so the edge sends it nothing until one is updated` : undefined;
    if (!version) return refuse(400, `A server says its build, {"version": <it>}`);
    const now = await relay(env, (relay) => relay.report(key, version, protocol, status, update));
    const attributes = Object.fromEntries(Object.entries({ server: key.slice(0, 8), ...now }).map(([name, value]) => [`one.${name}`, value]));
    console.log(JSON.stringify({ telemetry: { start: Date.now(), end: Date.now(), attributes, error } }));
    if (!status) return refuse(400, `A server says what it is with {"server_membership": ${MEMBERSHIPS.join("|")}, "server_daemon_status": ${SAID.join("|")}}`);
    return new Response(null, { status: 204 });
  }
  const keys = (url.searchParams.get("keys") ?? "").split(",").filter((key) => /^[0-9a-f]{64}$/.test(key));
  const here = await presence(env, keys.slice(0, 1000));
  // With `ping`, each server is pinged as one is before it's given an outsider's call, and whether it `answered`
  // within `REACH` is said too: a connection that has gone quiet stays here for up to `SILENT`, as a machine that
  // has just gone to sleep does, so being connected doesn't say a server would answer.
  if (url.searchParams.has("ping")) {
    for (const server of keys.slice(0, 10)) {
      const call = relay(env, (relay) => relay.fetch(`https://relay/call?to=${server}`, { headers: { upgrade: "websocket" } }));
      const answered = await reach(call, server).then(({ close }) => (close(), true), () => false);
      here[server] = { ...(here[server] ?? { connected: false }), answered };
    }
  }
  return Response.json({ presence: here, protocol: SPEAKS }, { headers: { "one-version": env.VERSION } });
}

/**
 * A server says builds were published to the network, `{"published": <the newest>, "latest": {<name>: <build>}}`,
 * the server build and each `one`, or asks for Update Now,
 * `{"now": true}`, with a POST, and the network's updater's status is given; a GET, from any of the network's own,
 * gives it.
 */
async function handleUpdate(env: Env, request: Request) {
  const admitted = await admit(env, request, request.method === "POST" ? await request.text() : "");
  if (admitted instanceof Response) return admitted;
  let input: Input = { status: true };
  if (request.method === "POST") {
    if (admitted.admitted.until !== undefined) return refuse(403, "Only a server says a build was published, or asks for Update Now");
    const { said } = admitted;
    if (typeof said.published === "string" && said.latest && typeof said.latest === "object") input = { published: said.published, latest: said.latest as Record<string, string> };
    else if (said.now === true) input = { now: true };
    else return refuse(400, 'A server says {"published": <build>, "latest": {<name>: <build>}} or {"now": true}');
  }
  return Response.json(await relay(env, (relay) => relay.updated(input)), { headers: { "one-version": env.VERSION } });
}

/** Gives what `call` gives of the network's one relay, tried again for up to `SWITCH` where it fails. */
async function relay<T>(env: Env, call: (relay: DurableObjectStub<Relay>) => Promise<T>): Promise<T> {
  const until = Date.now() + SWITCH;
  for (let wait = 250; ; wait = Math.min(wait * 2, 1_000)) {
    try {
      return await call(env.RELAY.getByName("relay"));
    } catch (error) {
      if ((error as { overloaded?: boolean }).overloaded || Date.now() + wait > until) throw error;
      console.log(JSON.stringify({ relay: "tried again", error: String(error) }));
      await scheduler.wait(wait);
    }
  }
}

/**
 * A connection to `server`, the relay's answer to `call`, once it has answered a ping within `REACH`: `ask` sends a
 * message and gives its answer, or `null` if the server left first. Rejects for a server that isn't here or didn't
 * answer.
 */
async function reach(call: Promise<Response>, server: string) {
  const { webSocket } = await call;
  if (!webSocket) throw new Error("isn't connected to the relay");
  webSocket.accept();
  let answered: ((answer: string | null) => void) | undefined;
  webSocket.addEventListener("message", (event) => answered?.(event.data as string));
  webSocket.addEventListener("close", () => answered?.(null));
  const ask = (message: string) => {
    webSocket.send(message);
    return new Promise<string | null>((resolve) => (answered = resolve));
  };
  const close = () => webSocket.close(1000, "Done");
  const pong = await Promise.race([ask(MCP_PING), scheduler.wait(REACH).then(() => null)]);
  if (pong === null) {
    close();
    throw new Error(`didn't answer a ping within ${REACH / 1000} s`);
  }
  return { server, ask, close };
}

export class Relay extends DurableObject<Env> {
  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env);
    console.log(JSON.stringify({ relay: "woke", connections: ctx.getWebSockets().length }));
  }

  async fetch(request: Request) {
    const url = new URL(request.url);
    if (url.pathname === "/endpoint") return await this.endpoint(url.searchParams);
    if (url.pathname === "/relay/outside") return this.server(request);
    if (url.pathname === "/notify") return this.notify(await request.text());
    return this.call(url.searchParams.get("to") ?? "");
  }

  /**
   * An endpoint that proved its key is joined: what it sends another comes over this connection, and what another
   * sends it goes back over it. One that connects again has left its old connection behind. It's connected for its
   * `device`, and a `server` that comes is told to the others.
   */
  async endpoint(params: URLSearchParams) {
    const [key, device, server] = [params.get("key"), params.get("device"), params.has("server")];
    for (const before of this.ctx.getWebSockets(`endpoint ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`endpoint ${key}`, `on ${device}`]);
    ours.serializeAttachment({ key, device, server });
    if (server) {
      this.notify(SEEN);
      await this.updated({ seen: key! });
    }
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /**
   * A server says what it is, `status`, its build, `version`, and the protocols it speaks. The other servers are told.
   * Gives its presence now.
   */
  async report(key: string, version: string, protocol: Protocol | undefined, status?: ServerStatus, update?: Reported) {
    await this.ctx.storage.put({ [`version ${key}`]: version, [`protocol ${key}`]: protocol ?? null, ...(status && { [`presence ${key}`]: status }) });
    if (status) this.notify(SEEN);
    if (update) await this.updated({ reported: { key, ...update } });
    return (await this.presence([key]))[key];
  }

  /**
   * Tells the network's updater `input`, with every server's presence and build, tells every server what it answers
   * they're to do, and gives its status. The relay goes on where the updater can't be reached, as one not deployed yet,
   * and says so: what the updater missed it's told again as each server next says what it is.
   */
  async updated(input: Input): Promise<object | null> {
    const keys = [...(await this.ctx.storage.list({ prefix: "version " })).keys()].map((at) => at.slice("version ".length));
    const [presence, versions] = await Promise.all([this.presence(keys), this.versions(keys)]);
    const servers = Object.fromEntries(keys.map((key) => [key, presence[key] && { ...presence[key], version: versions[key] }]));
    try {
      // Disposed of as it's read, so the updater's call ends as it answers, as Cloudflare's RPC lifecycle has it.
      using answered = await this.env.UPDATER.getByName("network").update(input, servers);
      if (answered.broadcast) this.notify(answered.broadcast);
      return answered.status;
    } catch (error) {
      console.log(JSON.stringify({ telemetry: { start: Date.now(), end: Date.now(), attributes: { "one.updater": Object.keys(input)[0] }, error: `The network's updater wasn't told: ${error}` } }));
      return null;
    }
  }

  /** The protocols each of `keys` that's a server speaks (`spoken`), by its key, leaving out one that never said what it is. */
  async protocols(keys: string[]): Promise<Record<string, Protocol | undefined>> {
    const [versions, said] = await Promise.all([this.versions(keys), this.ctx.storage.get<Protocol | null>(keys.map((key) => `protocol ${key}`))]);
    return Object.fromEntries(Object.keys(versions).map((key) => [key, spoken(said.get(`protocol ${key}`))]));
  }

  /** The build each of `keys` that's a server said it is, by its key, leaving out one that never said. */
  async versions(keys: string[]): Promise<Record<string, string>> {
    return Object.fromEntries([...(await this.ctx.storage.get<string>(keys.map((key) => `version ${key}`)))].map(([at, version]) => [at.slice(8), version]));
  }

  /** Whether each of `keys` is here, as the relay's `presence` says. */
  async presence(keys: string[]): Promise<Record<string, Presence | null>> {
    const [left, said] = await Promise.all([
      this.ctx.storage.get<number>(keys.map((key) => `seen ${key}`)),
      this.ctx.storage.get<ServerStatus>(keys.map((key) => `presence ${key}`)),
    ]);
    const presence = (key: string): Presence | null => {
      const [connected, last_seen, status] = [this.on(key), left.get(`seen ${key}`), said.get(`presence ${key}`)];
      if (!connected && last_seen === undefined && !status) return null;
      const here: Presence = { connected, ...(!connected && last_seen !== undefined && { last_seen }) };
      if (!status) return here;
      const { server_membership, server_daemon_status: daemon, at } = status;
      const lasts = daemon === "starting" ? STARTING : daemon === "stopping" ? STOPPING : 0;
      const server_daemon_status = connected || Date.now() - at < lasts ? daemon : "stopped";
      const server_ready = server_membership === "active" && server_daemon_status === "running";
      const server_ready_reason: ServerReadyReason | undefined = server_ready ? undefined
        : server_daemon_status === "stopped" ? "stopped"
        : server_membership === "joining" ? "joining"
        : server_membership === "leaving" || server_daemon_status === "stopping" ? "draining"
        : "starting";
      return { ...here, server_membership, server_daemon_status, server_ready, ...(server_ready_reason && { server_ready_reason }) };
    };
    return Object.fromEntries(keys.map((key) => [key, presence(key)]));
  }

  /** Whether `device` has a connection here, other than `leaving`. */
  on(device: string, leaving?: WebSocket) {
    return this.ctx.getWebSockets(`on ${device}`).some((ws) => ws !== leaving);
  }

  /** A server connects to hear outsiders' calls. It signed the request, so its key is known at once. */
  async server(request: Request) {
    const admitted = await admit(this.env, request, "");
    if (admitted instanceof Response) return admitted;
    const { key } = admitted;
    for (const before of this.ctx.getWebSockets(`outside ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`outside ${key}`]);
    ours.serializeAttachment({ outside: key });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /**
   * Says `message` to every server that's here. Each takes it as a call, whose answer nothing waits for. A connection
   * that has closed is listed until the relay hears so, as one a server that's starting again left behind is.
   */
  notify(message: string) {
    for (const server of this.ctx.getWebSockets()) {
      if (server.deserializeAttachment()?.outside && server.readyState === WebSocket.OPEN) server.send(JSON.stringify({ message }));
    }
    return new Response(null, { status: 204 });
  }

  /** A piece here connects to pass calls to the server `to`, one at a time, and waits on the connection for each answer. */
  call(to: string) {
    if (!this.ctx.getWebSockets(`outside ${to}`).length) return refuse(404, "That server isn't here");
    const call = crypto.randomUUID();
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`call ${call}`]);
    ours.serializeAttachment({ call, to });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer) {
    const attachment: Attachment = ws.deserializeAttachment() ?? {};
    if (attachment.call) {
      const [server] = this.ctx.getWebSockets(`outside ${attachment.to}`);
      if (!server) return ws.close(1011, "That server left");
      return server.send(JSON.stringify({ call: attachment.call, message }));
    }
    if (attachment.outside) {
      const { call, answer } = JSON.parse(message as string);
      for (const waiting of this.ctx.getWebSockets(`call ${call}`)) waiting.send(answer);
      return;
    }
    if (!attachment.key) return;
    // The key it's for becomes the key it's from, and the rest is passed on untouched. With nobody there it's
    // dropped, as a network drops a packet, and QUIC sends again.
    const frame = new Uint8Array(message as ArrayBuffer);
    const [to] = this.ctx.getWebSockets(`endpoint ${hex.encode(frame.subarray(1, 33))}`);
    if (!to) return;
    frame[0] += 2;
    frame.set(hex.decode(attachment.key), 1);
    to.send(frame);
  }

  async webSocketClose(ws: WebSocket) {
    const { outside, device, server }: Attachment = ws.deserializeAttachment() ?? {};
    // The last connection a device or server had here ends, so it was last seen now.
    if (device && !this.on(device, ws)) {
      await this.ctx.storage.put(`seen ${device}`, Date.now());
      if (server) {
        this.notify(SEEN);
        await this.updated({ seen: device });
      }
    }
    // A server that left answers nothing, so the calls waiting on it are told.
    if (!outside || this.ctx.getWebSockets(`outside ${outside}`).some((other) => other !== ws)) return;
    for (const waiting of this.ctx.getWebSockets()) {
      if (waiting.deserializeAttachment()?.to === outside) waiting.close(1011, "That server left");
    }
  }

  webSocketError(ws: WebSocket) {
    return this.webSocketClose(ws);
  }
}
