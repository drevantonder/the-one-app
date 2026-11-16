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
// And it knows who's on: a server or a device is on while it's connected here, through any run of its client, and
// otherwise was last seen when its last connection ended, which the Durable Object keeps. A connection that says
// nothing for longer than iroh's pings allow has ended, as a machine that sleeps or loses its network says nothing.
// Servers are told when one of them comes or goes (`notifications/one/seen`), and ask (`/relay/seen`).
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { blake3 } from "@noble/hashes/blake3.js";
import { verifiedKey, hex, signedBy } from "../ask.ts";

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
/** A W3C `traceparent`: its trace and span. */
const TRACE = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/;

/** Who's on: `"on"` for one connected now, when the last one that was left (ms since 1970), or `null` for one never seen. */
type Seen = "on" | number | null;

/** An endpoint joined to the relay: the connection its datagrams are passed over, and when its token expires. */
interface Joined {
  relay: WebSocket;
  until?: number;
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

export default class extends WorkerEntrypoint<RelayEnv> {
  async fetch(request: Request) {
    const url = new URL(request.url);
    // iroh times this to learn how far away the relay is. An endpoint with no answer has no relay.
    if (url.pathname === "/ping") return new Response(null, { status: 200 });
    if (url.pathname === "/relay/seen") return this.handleSeen(request, url);
    if (request.headers.get("upgrade") !== "websocket") return refuse(url.pathname.startsWith("/relay") ? 426 : 404, "The relay is a WebSocket at /relay");
    if (url.pathname === "/relay") return this.endpoint(request, url);
    if (url.pathname === "/relay/outside") return this.relay().fetch(request);
    return refuse(404, "The relay is at /relay");
  }

  /** An iroh endpoint connects: it's sent a challenge, and is nobody until it signs it. */
  endpoint(request: Request, url: URL) {
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
      if (Date.now() - received < SILENT) return;
      clearInterval(quiet);
      ours.close(1001, "It went quiet");
      (await joined)?.relay.close(1001, "It went quiet");
    }, SILENT / 3);
    ours.addEventListener("close", () => clearInterval(quiet));
    ours.addEventListener("message", async ({ data }) => {
      received = Date.now();
      if (typeof data === "string") return ours.close(1003, "The relay takes binary frames");
      const frame = new Uint8Array(data);
      if (!joined) return void (joined = this.join(ours, challenge, token, frame));
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
  async join(ours: WebSocket, challenge: Uint8Array, token: string | null, frame: Uint8Array): Promise<Joined | undefined> {
    // The frame is postcard's: the type, the key's 32 bytes, then the signature as bytes with their length before.
    const key = hex.encode(frame.subarray(1, 33));
    const proved =
      frame.length === 98 &&
      frame[0] === CLIENT_AUTH &&
      frame[33] === 64 &&
      (await signedBy(key, hex.encode(frame.subarray(34)), blake3(challenge, { context: utf8.encode(CHALLENGE) })));
    const admitted = proved && (await this.env.NETWORK.admits(key, token));
    if (!admitted) {
      const why = utf8.encode("This network doesn't know you");
      ours.send(Uint8Array.of(SERVER_DENIES_AUTH, why.length, ...why));
      return void ours.close(1008, "Not one of this network's");
    }
    // A run of a client is on for the device it runs for, which its token names, and a server for itself.
    const [, device = key] = (token ?? "").split(".");
    const on = new URLSearchParams({ key, device, ...(admitted.until === undefined && { server: "yes" }) });
    const relay = (await this.relay().fetch(`https://relay/endpoint?${on}`, { headers: { upgrade: "websocket" } })).webSocket!;
    relay.accept();
    relay.binaryType = "arraybuffer";
    relay.addEventListener("message", ({ data }) => ours.send(data as ArrayBuffer));
    relay.addEventListener("close", () => ours.close(1000, "It connected again"));
    ours.addEventListener("close", () => relay.close(1000, "It left"));
    ours.send(Uint8Array.of(SERVER_CONFIRMS_AUTH));
    return { relay, until: admitted.until };
  }

  /**
   * Passes an outsider's `message` to one of `servers`, by their keys, and gives `{server, answer}`: the first of
   * them to show it's there by answering a ping within `REACH`, or `null` if none does. The wait for the answer is
   * this Worker's, on a connection the relay holds asleep, so nothing is billed for it, and it's as long as the
   * call takes.
   */
  async forward(servers: string[], message: string): Promise<{ server: string; answer: string } | null> {
    // A span of its own under the caller's, which the server's is under in turn (W3C's Trace Context).
    const call = JSON.parse(message);
    const parent: string | undefined = call.params?._meta?.traceparent;
    const traceparent = `00-${TRACE.exec(parent ?? "")?.[1] ?? hex.encode(crypto.getRandomValues(new Uint8Array(16)))}-${hex.encode(crypto.getRandomValues(new Uint8Array(8)))}-01`;
    message = JSON.stringify({ ...call, params: { ...call.params, _meta: { ...call.params?._meta, traceparent } } });

    const tries = servers.map((server) => reach(this.relay(), server));
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
    console.log(JSON.stringify({ telemetry: { traceparent, parent, attributes: { "one.servers": servers.length, ...(reached && { "one.server": reached.server.slice(0, 8) }) }, error } }));
    return reached && answer !== null ? { server: reached.server, answer } : null;
  }

  /** Tells every server that's here `message`, which none answers: a JSON-RPC notification. */
  async notify(message: string): Promise<void> {
    await this.relay().fetch("https://relay/notify", { method: "POST", body: message });
  }

  /**
   * Who's on, of `keys`, servers' and devices': `"on"` for one connected here now, when the last one that was left
   * (milliseconds since 1970), or `null` for one never seen.
   */
  seen(keys: string[]): Promise<Record<string, Seen>> {
    return this.relay().seen(keys);
  }

  /** A server or a device of the network's asks who's on, of the keys it names: `?keys=<key>,<key>`. */
  async handleSeen(request: Request, url: URL) {
    const key = await verifiedKey(request, "");
    if (!key || !(await this.env.NETWORK.admits(key, request.headers.get("one-token")))) return refuse(403, "This network doesn't know you");
    const keys = (url.searchParams.get("keys") ?? "").split(",").filter((key) => /^[0-9a-f]{64}$/.test(key));
    return Response.json(await this.seen(keys.slice(0, 1000)));
  }

  /** A network has one relay. */
  relay() {
    return this.env.RELAY.get(this.env.RELAY.idFromName("relay"));
  }
}

/**
 * A connection to `server` through `relay`, once it has answered a ping within `REACH`: `ask` sends a message and
 * gives its answer, or `null` if the server left first. Rejects for a server that isn't here or didn't answer.
 */
async function reach(relay: DurableObjectStub<Relay>, server: string) {
  const { webSocket } = await relay.fetch(`https://relay/call?to=${server}`, { headers: { upgrade: "websocket" } });
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

export class Relay extends DurableObject<RelayEnv> {
  constructor(ctx: DurableObjectState, env: RelayEnv) {
    super(ctx, env);
    console.log(JSON.stringify({ relay: "woke", connections: ctx.getWebSockets().length }));
  }

  async fetch(request: Request) {
    const url = new URL(request.url);
    if (url.pathname === "/endpoint") return this.endpoint(url.searchParams);
    if (url.pathname === "/relay/outside") return this.server(request);
    if (url.pathname === "/notify") return this.notify(await request.text());
    return this.call(url.searchParams.get("to") ?? "");
  }

  /**
   * An endpoint that proved its key is joined: what it sends another comes over this connection, and what another
   * sends it goes back over it. One that connects again has left its old connection behind. It's on for its
   * `device`, and a `server` that comes is told to the others.
   */
  endpoint(params: URLSearchParams) {
    const [key, device, server] = [params.get("key"), params.get("device"), params.has("server")];
    for (const before of this.ctx.getWebSockets(`endpoint ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`endpoint ${key}`, `on ${device}`]);
    ours.serializeAttachment({ key, device, server });
    if (server) this.notify(SEEN);
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /** Who's on, of `keys`, as the relay's `seen` says. */
  async seen(keys: string[]): Promise<Record<string, Seen>> {
    const left = await this.ctx.storage.get<number>(keys.map((key) => `seen ${key}`));
    return Object.fromEntries(keys.map((key) => [key, this.on(key) ? "on" : (left.get(`seen ${key}`) ?? null)]));
  }

  /** Whether `device` has a connection here, other than `leaving`. */
  on(device: string, leaving?: WebSocket) {
    return this.ctx.getWebSockets(`on ${device}`).some((ws) => ws !== leaving);
  }

  /** A server connects to hear outsiders' calls. It signed the request, so its key is known at once. */
  async server(request: Request) {
    const key = await verifiedKey(request, "");
    if (!key || !(await this.env.NETWORK.admits(key, request.headers.get("one-token")))) return refuse(403, "This network doesn't know you");
    for (const before of this.ctx.getWebSockets(`outside ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`outside ${key}`]);
    ours.serializeAttachment({ outside: key });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /** Says `message` to every server that's here. Each takes it as a call, whose answer nothing waits for. */
  notify(message: string) {
    for (const server of this.ctx.getWebSockets()) {
      if (server.deserializeAttachment()?.outside) server.send(JSON.stringify({ message }));
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
      if (server) this.notify(SEEN);
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
