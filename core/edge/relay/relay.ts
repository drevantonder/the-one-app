// The edge's side of the relay (`worker.ts`, a Worker of its own, which holds every server's and device's connection):
// it passes a call that came in at the address over the web to a server, tells every server what a piece here has to
// say, and says who's here. The rest of the edge calls it as functions (`forward`, `notify`, `presence`), and an app's
// part through `RelayEntrypoint`, which `edge.js` binds it to as RELAY. It reaches the relay's Durable Object by its
// binding RELAY, and the relay's Worker by RELAY_WORKER, to which it passes the relay's own paths.
//
// A server says what it is here as that changes (`/relay/seen`), and a build published and Update Now
// (`/relay/update`), which the relay tells the network's updater (`updater/updater.ts`); only a ready server is sent
// calls.
//
// Parts of the platform talk where the protocols they speak overlap (`OneCore.Protocol`): a server says the range it
// speaks, and its build, as it says what it is. A call from outside goes only to a server that talks with the edge,
// and says the edge's protocol and build: a rule apart from being ready.
import { WorkerEntrypoint } from "cloudflare:workers";
/** The protocols the edge speaks, and the cutoff for parts too old to say theirs (`core/protocol.json`). */
import PROTOCOL_CONFIG from "../../protocol.json";
import { verifiedKey, hex } from "../ask.ts";
import { admits, reader } from "../presigner/presigner.ts";
import type { Input, Reported } from "../updater/updater.ts";
import { relay as relayed, spoken, type Presence, type Protocol, type ServerDaemonStatus, type ServerMembership } from "./common.ts";
import type { Relay } from "./worker.ts";

/**
 * How long a server has to answer a ping before a call goes to another: a server that's registered here may be a
 * machine that has just gone to sleep or is restarting, whose connection hasn't ended yet. It's well inside what an
 * MCP client waits for (Claude's 30 seconds), and long enough for a server across the world. How long the call
 * itself then takes is its tool's.
 */
const REACH = 5_000;
/** MCP's ping, which a server answers at once, whoever asks. */
const MCP_PING = JSON.stringify({ jsonrpc: "2.0", id: 0, method: "ping" });
/** The protocols the edge speaks, from `oldest` to `newest`. */
const SPEAKS: Protocol = { oldest: PROTOCOL_CONFIG.oldest, newest: PROTOCOL_CONFIG.newest };
/** Whether a part that speaks `theirs` talks with the edge: their ranges overlap. */
const talk = (theirs: Protocol | undefined) => !!theirs && theirs.oldest <= SPEAKS.newest && SPEAKS.oldest <= theirs.newest;
/** What a range is, said: `1`, `1 to 2`, or `none`. */
const named = (protocol: Protocol | undefined) => (!protocol ? "none" : protocol.oldest === protocol.newest ? `${protocol.oldest}` : `${protocol.oldest} to ${protocol.newest}`);

/** A W3C `traceparent`: its trace and span. */
const TRACE = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/;

const MEMBERSHIPS: ServerMembership[] = ["joining", "active", "leaving"];
const SAID: ServerDaemonStatus[] = ["starting", "running", "stopping"];

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
    if (url.pathname === "/relay/seen") return handleSeen(env, request, url);
    if (url.pathname === "/relay/update") return handleUpdate(env, request);
    return env.RELAY_WORKER.fetch(request);
  },
} satisfies ExportedHandler<Env>;

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
 * A server asks for Update Now, `{"now": true}`, with a POST, and the network's updater's status is given; a GET,
 * from any of the network's own, gives it. What the core released the edge tells the updater itself
 * (`../updater/core.ts`).
 */
async function handleUpdate(env: Env, request: Request) {
  const admitted = await admit(env, request, request.method === "POST" ? await request.text() : "");
  if (admitted instanceof Response) return admitted;
  let input: Input = { status: true };
  if (request.method === "POST") {
    if (admitted.admitted.until !== undefined) return refuse(403, "Only a server asks for Update Now");
    if (admitted.said.now !== true) return refuse(400, 'A server asks {"now": true}');
    input = { now: true };
  }
  return Response.json(await relay(env, (relay) => relay.updated(input)), { headers: { "one-version": env.VERSION } });
}

/** Gives what `call` gives of the network's one relay, as the relay's Worker reaches it (`worker.ts`). */
const relay = <T,>(env: Env, call: (relay: DurableObjectStub<Relay>) => Promise<T>) => relayed(env.RELAY, call);

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
