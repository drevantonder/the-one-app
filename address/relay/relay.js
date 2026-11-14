// The relay: passes traffic between a network's servers and clients when they can't reach each other directly. It
// speaks iroh's relay protocol, version 2 (iroh-relay 1.4's `protos`: a WebSocket of binary frames, each a type and
// its fields), to iroh's own endpoints. What it passes is QUIC, encrypted between the two keys, so it can't read it.
//
// An endpoint's connection is held by the Worker it reached, where waiting costs nothing: there it proves its key,
// and iroh's question every 15 seconds, whether the relay is still there, is answered. Once proved, it's joined to
// the one Durable Object that knows every endpoint, which passes on what one sends another. That object sleeps with
// its connections open (hibernation), and is woken only by something to pass on: nothing an idle network says
// reaches it. It admits only the network's own, by asking the signer about each key once that key has proved
// itself.
//
// It also passes an outsider's call to a server, for the way in: a server holds one more connection here for those
// (`/relay/outside`), since nothing else at the address holds one.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { blake3 } from "@noble/hashes/blake3.js";
import { asker, hex, signedBy } from "../ask.js";

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

const utf8 = new TextEncoder();
const refuse = (status, why) => new Response(why, { status });

export default class extends WorkerEntrypoint {
  async fetch(request) {
    const url = new URL(request.url);
    // iroh times this to learn how far away the relay is. An endpoint with no answer has no relay.
    if (url.pathname === "/ping") return new Response(null, { status: 200 });
    if (request.headers.get("upgrade") !== "websocket") return refuse(url.pathname.startsWith("/relay") ? 426 : 404, "The relay is a WebSocket at /relay");
    if (url.pathname === "/relay") return this.endpoint(request, url);
    if (url.pathname === "/relay/outside") return this.relay().fetch(request);
    return refuse(404, "The relay is at /relay");
  }

  /** An iroh endpoint connects: it's sent a challenge, and is nobody until it signs it. */
  endpoint(request, url) {
    const speaks = (request.headers.get("sec-websocket-protocol") ?? "").split(",").map((protocol) => protocol.trim());
    if (!speaks.includes(PROTOCOL)) return refuse(400, `The relay speaks ${PROTOCOL}`);
    // iroh shows its pass as a bearer token, and in the URL from a browser, which can't set a header.
    const pass = /^Bearer (.+)$/.exec(request.headers.get("authorization") ?? "")?.[1] ?? url.searchParams.get("token");
    const challenge = crypto.getRandomValues(new Uint8Array(16));
    const [theirs, ours] = Object.values(new WebSocketPair());
    ours.accept();
    ours.binaryType = "arraybuffer";
    // What it sends before it's joined waits for that, in the order it was sent.
    let joined;
    ours.addEventListener("message", async ({ data }) => {
      if (typeof data === "string") return ours.close(1003, "The relay takes binary frames");
      const frame = new Uint8Array(data);
      if (!joined) return void (joined = this.join(ours, challenge, pass, frame));
      const { relay, until } = (await joined) ?? {};
      if (!relay) return;
      if (until < Date.now()) return ours.close(1008, "Its pass ended");
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
   * pass ends if it does.
   */
  async join(ours, challenge, pass, frame) {
    // The frame is postcard's: the type, the key's 32 bytes, then the signature as bytes with their length before.
    const key = hex.encode(frame.subarray(1, 33));
    const proved =
      frame.length === 98 &&
      frame[0] === CLIENT_AUTH &&
      frame[33] === 64 &&
      (await signedBy(key, hex.encode(frame.subarray(34)), blake3(challenge, { context: utf8.encode(CHALLENGE) })));
    const admitted = proved && (await this.env.SIGNER.admits(key, pass));
    if (!admitted) {
      const why = utf8.encode("This network doesn't know you");
      ours.send(Uint8Array.of(SERVER_DENIES_AUTH, why.length, ...why));
      return void ours.close(1008, "Not one of this network's");
    }
    const { webSocket: relay } = await this.relay().fetch(`https://relay/endpoint?key=${key}`, { headers: { upgrade: "websocket" } });
    relay.accept();
    relay.binaryType = "arraybuffer";
    relay.addEventListener("message", ({ data }) => ours.send(data));
    relay.addEventListener("close", () => ours.close(1000, "It connected again"));
    ours.addEventListener("close", () => relay.close(1000, "It left"));
    ours.send(Uint8Array.of(SERVER_CONFIRMS_AUTH));
    return { relay, until: admitted.until };
  }

  /**
   * Passes an outsider's `message` to the server whose key is `server` and gives its answer, or `null` if that
   * server isn't here. The wait is this Worker's, on a connection the relay holds asleep, so nothing is billed
   * for it.
   */
  async pass(server, message) {
    const { webSocket } = await this.relay().fetch(`https://relay/pass?to=${server}`, { headers: { upgrade: "websocket" } });
    if (!webSocket) return null;
    webSocket.accept();
    const answered = new Promise((resolve) => {
      webSocket.addEventListener("message", (event) => resolve(event.data));
      webSocket.addEventListener("close", () => resolve(null));
    });
    webSocket.send(message);
    return answered;
  }

  /** A network has one relay. */
  relay() {
    return this.env.RELAY.get(this.env.RELAY.idFromName("relay"));
  }
}

export class Relay extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    console.log(JSON.stringify({ relay: "woke", connections: ctx.getWebSockets().length }));
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/endpoint") return this.endpoint(url.searchParams.get("key"));
    if (url.pathname === "/relay/outside") return this.server(request);
    return this.call(url.searchParams.get("to"));
  }

  /**
   * An endpoint that proved its key is joined: what it sends another comes over this connection, and what another
   * sends it goes back over it. One that connects again has left its old connection behind.
   */
  endpoint(key) {
    for (const before of this.ctx.getWebSockets(`endpoint ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`endpoint ${key}`]);
    ours.serializeAttachment({ key });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /** A server connects to hear outsiders' calls. It signed the request, so its key is known at once. */
  async server(request) {
    const key = await asker(request, "");
    if (!key || !(await this.env.SIGNER.admits(key, request.headers.get("one-pass")))) return refuse(403, "This network doesn't know you");
    for (const before of this.ctx.getWebSockets(`outside ${key}`)) before.close(1000, "It connected again");
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`outside ${key}`]);
    ours.serializeAttachment({ outside: key });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  /** The way in connects to pass one call to the server `to`, and waits on the connection for its answer. */
  call(to) {
    if (!this.ctx.getWebSockets(`outside ${to}`).length) return refuse(404, "That server isn't here");
    const call = crypto.randomUUID();
    const [theirs, ours] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(ours, [`call ${call}`]);
    ours.serializeAttachment({ call, to });
    return new Response(null, { status: 101, webSocket: theirs });
  }

  async webSocketMessage(ws, message) {
    const said = ws.deserializeAttachment() ?? {};
    if (said.call) {
      const [server] = this.ctx.getWebSockets(`outside ${said.to}`);
      if (!server) return ws.close(1011, "That server left");
      return server.send(JSON.stringify({ call: said.call, message }));
    }
    if (said.outside) {
      const { call, answer } = JSON.parse(message);
      for (const waiting of this.ctx.getWebSockets(`call ${call}`)) {
        waiting.send(answer);
        waiting.close(1000, "Answered");
      }
      return;
    }
    if (!said.key) return;
    // The key it's for becomes the key it's from, and the rest is passed on untouched. With nobody there it's
    // dropped, as a network drops a packet, and QUIC sends again.
    const frame = new Uint8Array(message);
    const [to] = this.ctx.getWebSockets(`endpoint ${hex.encode(frame.subarray(1, 33))}`);
    if (!to) return;
    frame[0] += 2;
    frame.set(hex.decode(said.key), 1);
    to.send(frame);
  }

  webSocketClose(ws) {
    const { outside } = ws.deserializeAttachment() ?? {};
    // A server that left answers nothing, so the calls waiting on it are told.
    if (!outside || this.ctx.getWebSockets(`outside ${outside}`).some((other) => other !== ws)) return;
    for (const waiting of this.ctx.getWebSockets()) {
      if (waiting.deserializeAttachment()?.to === outside) waiting.close(1011, "That server left");
    }
  }
}
