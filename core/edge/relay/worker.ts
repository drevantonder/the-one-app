// The relay, a Worker of its own (`network-relay`, `wrangler.jsonc` beside this): it passes traffic between a network's
// servers and clients when they can't reach each other directly. It speaks iroh's relay protocol, version 2
// (iroh-relay 1.4's `protos`: a WebSocket of binary frames, each a type and its fields), to iroh's own endpoints. What
// it passes is QUIC, encrypted between the two keys, so it can't read it.
//
// It's apart from the rest of the edge (`edge.ts`) because it holds every server's and device's connection, which a
// deploy of the Worker holding them ends: the edge changes with every page and MCP change, this rarely, and is deployed
// only when it has (`edge.js`). The edge passes it its own paths (`/relay`, `/relay/outside`, `/ping`) through a
// service binding, and the connection is then this Worker's, which an edge deploy leaves open.
//
// An endpoint's connection is held by the Worker it reached, where waiting costs nothing: there it proves its key,
// and iroh's question every 15 seconds, whether the relay is still there, is answered. Once proved, it's joined to
// the one Durable Object that knows every endpoint, which passes on what one sends another. That object sleeps with
// its connections open (hibernation), and is woken only by something to pass on: nothing an idle network says
// reaches it. It admits only the network's own, by asking the edge about each key once that key has proved itself
// (`Network`, bound as NETWORK).
//
// A server holds one more connection here (`/relay/outside`), down which the edge passes it a call that came in at
// the address over the web, and tells every server what a piece there has to say (`relay.ts`).
//
// And it knows who's here: a server or a device is connected while it's connected here, through any run of its
// client, and otherwise was last seen when its last connection ended, which the Durable Object keeps. A connection
// that says nothing for longer than iroh's pings allow has ended, as a machine that sleeps or loses its network says
// nothing. A server says what it is as that changes (`OneCore.Release`), through the edge: its `server_membership`,
// whether it's one of the network's servers, and its `server_daemon_status`, whether its program is starting, running
// or stopping. It's ready once it's active and running, and connected. Servers are told when one of them comes or goes
// (`notifications/one/seen`), and the network's updater in the edge (`updater/updater.ts`, bound as UPDATER) is told
// what each server says of itself and as one comes or goes, and what it answers is told to every server.
import { DurableObject } from "cloudflare:workers";
import { blake3 } from "@noble/hashes/blake3.js";
import { verifiedKey, hex, signedBy } from "../ask.ts";
import type { Input, NetworkUpdater, Reported } from "../updater/updater.ts";
import type { Network } from "../presigner/presigner.ts";
import { relay, spoken, type Presence, type Protocol, type ServerReadyReason, type ServerStatus } from "./common.ts";

/** Its bindings to the edge's Worker, typed (`worker-configuration.d.ts` has them untyped). */
declare global {
  interface RelayEnv {
    UPDATER: DurableObjectNamespace<NetworkUpdater>;
    NETWORK: Service<Network>;
  }
}

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
 * How long a server that isn't connected is still starting or stopping as it said, in milliseconds: one says it's
 * starting just before it connects, and one stopping to start again, to take an update, is starting again within a
 * minute. Past that it's stopped.
 */
const STARTING = 60_000;
const STOPPING = 60_000;
/**
 * How long a server isn't heard from before it's forgotten, as Consul reaps a node gone longer than its
 * `reconnect_timeout`: a server whose machine was reinstalled with a new key never says it's leaving. One that comes
 * back is known again as it says what it is, so forgetting loses nothing.
 */
const REAP = 24 * 60 * 60_000;

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

export default {
  async fetch(request: Request, env: RelayEnv) {
    const url = new URL(request.url);
    // iroh times this to learn how far away the relay is. An endpoint with no answer has no relay.
    if (url.pathname === "/ping") return new Response(null, { status: 200 });
    if (request.headers.get("upgrade") !== "websocket") return refuse(url.pathname.startsWith("/relay") ? 426 : 404, "The relay is a WebSocket at /relay");
    if (url.pathname === "/relay") return endpoint(env, request, url);
    if (url.pathname === "/relay/outside") return outside(env, request);
    return refuse(404, "The relay is at /relay");
  },
} satisfies ExportedHandler<RelayEnv>;

/** A server connects to hear outsiders' calls, signing its request with its key. */
async function outside(env: RelayEnv, request: Request) {
  const key = await verifiedKey(request, "");
  if (!key || !(await env.NETWORK.admits(key, request.headers.get("one-token")))) return refuse(403, "This network doesn't know you");
  return relay(env.RELAY, (relay) => relay.fetch(`https://relay/outside?key=${key}`, { headers: { upgrade: "websocket" } }));
}

/** An iroh endpoint connects: it's sent a challenge, and is nobody until it signs it. */
function endpoint(env: RelayEnv, request: Request, url: URL) {
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
    if (key && !(await env.NETWORK.admits(key, token))) ours.close(1008, "Not one of this network's");
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
async function join(env: RelayEnv, ours: WebSocket, challenge: Uint8Array, token: string | null, frame: Uint8Array): Promise<Joined | undefined> {
  // The frame is postcard's: the type, the key's 32 bytes, then the signature as bytes with their length before.
  const key = hex.encode(frame.subarray(1, 33));
  const proved =
    frame.length === 98 &&
    frame[0] === CLIENT_AUTH &&
    frame[33] === 64 &&
    (await signedBy(key, hex.encode(frame.subarray(34)), blake3(challenge, { context: utf8.encode(CHALLENGE) })));
  const admitted = proved && (await env.NETWORK.admits(key, token));
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
  const joined = (await relay(env.RELAY, (relay) => relay.fetch(`https://relay/endpoint?${on}`, { headers: { upgrade: "websocket" } }))).webSocket!;
  joined.accept();
  joined.binaryType = "arraybuffer";
  joined.addEventListener("message", ({ data }) => ours.send(data as ArrayBuffer));
  joined.addEventListener("close", () => ours.close(1000, "It connected again"));
  ours.addEventListener("close", () => joined.close(1000, "It left"));
  ours.send(Uint8Array.of(SERVER_CONFIRMS_AUTH));
  return { relay: joined, until: admitted.until, key };
}

export class Relay extends DurableObject<RelayEnv> {
  constructor(ctx: DurableObjectState, env: RelayEnv) {
    super(ctx, env);
    console.log(JSON.stringify({ relay: "woke", connections: ctx.getWebSockets().length }));
  }

  async fetch(request: Request) {
    const url = new URL(request.url);
    if (url.pathname === "/endpoint") return await this.endpoint(url.searchParams);
    if (url.pathname === "/outside") return this.server(url.searchParams.get("key")!);
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
    const keys = await this.reap([...(await this.ctx.storage.list({ prefix: "version " })).keys()].map((at) => at.slice("version ".length)));
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

  /** Forgets each of the servers `keys` not heard from in `REAP`, and gives the rest. */
  async reap(keys: string[]): Promise<string[]> {
    const [left, said] = await Promise.all([
      this.ctx.storage.get<number>(keys.map((key) => `seen ${key}`)),
      this.ctx.storage.get<ServerStatus>(keys.map((key) => `presence ${key}`)),
    ]);
    const heard = (key: string) => Math.max(left.get(`seen ${key}`) ?? 0, said.get(`presence ${key}`)?.at ?? 0);
    const gone = new Set(keys.filter((key) => !this.on(key) && Date.now() - heard(key) > REAP));
    for (let at = 0; at < gone.size; at += 32) {
      await this.ctx.storage.delete([...gone].slice(at, at + 32).flatMap((key) => [`version ${key}`, `protocol ${key}`, `presence ${key}`, `seen ${key}`]));
    }
    return keys.filter((key) => !gone.has(key));
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

  /** A server connects to hear outsiders' calls, which the Worker admitted by its key. */
  server(key: string) {
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
