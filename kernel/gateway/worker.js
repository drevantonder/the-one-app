// The gateway: the install's address, the one people use. It holds no keys and runs no app code; it only sends each
// request where its path says: sign-in to the auth service (with the gateway's host kept, since passkeys belong to
// it), `/v1/` and `/mcp` to a node, `/relay` to the relay, and everything else to the web client's files.
//
// What a node answers is stamped so it can't run as script at this origin, which holds browsers' tokens: it's
// sandboxed into an opaque origin, can't be sniffed into another type, and sets no cookies here.

/// Paths the auth service answers: exactly these, or under the ones ending in `/`.
const AUTH = ["/api/auth/", "/sign-in", "/invite", "/passkeys", "/.well-known/oauth-authorization-server", "/.well-known/openid-configuration", "/.well-known/oauth-protected-resource"];
const NODE = ["/v1/", "/mcp"];

/// What a node's answer is stamped with.
const SANDBOX = "sandbox allow-scripts allow-forms allow-popups allow-downloads";

/// The request headers a node gets: never the browser's cookies, which are the auth service's.
const TO_NODE = ["authorization", "content-type", "accept", "accept-language", "last-event-id", "mcp-session-id", "mcp-protocol-version"];
/// The answer headers a node may give: `www-authenticate` tells an MCP client where to sign in.
const FROM_NODE = ["content-type", "cache-control", "etag", "last-modified", "content-disposition", "retry-after", "www-authenticate", "mcp-session-id"];

const TYPES = {
  html: "text/html; charset=utf-8", js: "text/javascript; charset=utf-8", mjs: "text/javascript; charset=utf-8", css: "text/css; charset=utf-8",
  json: "application/json", webmanifest: "application/manifest+json", svg: "image/svg+xml", png: "image/png", jpg: "image/jpeg",
  jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", ico: "image/x-icon", woff2: "font/woff2", txt: "text/plain; charset=utf-8", wasm: "application/wasm",
};

const under = (path, prefixes) => prefixes.some((p) => (p.endsWith("/") ? path.startsWith(p) : path === p || path.startsWith(`${p}/`)));

/// Where a path goes: `auth`, `node` or `web`.
function where(path) {
  if (under(path, AUTH)) return "auth";
  if (under(path, NODE)) return "node";
  return "web";
}

const refuse = (status, error) => Response.json({ error }, { status, headers: { "x-content-type-options": "nosniff" } });

async function toNode(request, env, url) {
  // A service worker answers for the whole origin: none comes from a node.
  if (request.headers.has("service-worker")) return refuse(403, "a node's answer can't be a service worker");
  const headers = new Headers();
  for (const name of TO_NODE) if (request.headers.has(name)) headers.set(name, request.headers.get(name));
  // Says it came from elsewhere, so a node on this machine never takes it as one of the machine's own programs.
  headers.set("forwarded", `host="${url.host}";proto=${url.protocol.slice(0, -1)}`);
  if (!env.NODE && !env.NODE_URL) return refuse(503, "this install's kernel has no node to send that to yet");
  const init = { method: request.method, headers, body: ["GET", "HEAD"].includes(request.method) ? null : request.body, redirect: "manual" };
  let resp;
  try {
    // Through a Cloudflare Tunnel (`NODE`, a Workers VPC service), a URL's host is only the Host the node sees, and
    // cloudflared adds nothing saying it came from elsewhere: the gateway's own host is never one a node takes as its
    // machine's.
    resp = env.NODE ? await env.NODE.fetch(`http://${url.host}${url.pathname}${url.search}`, init) : await fetch(`${env.NODE_URL}${url.pathname}${url.search}`, init);
  } catch {
    return refuse(502, "Your devices aren't answering.");
  }
  const out = new Headers();
  for (const name of FROM_NODE) if (resp.headers.has(name)) out.set(name, resp.headers.get(name));
  out.set("content-security-policy", SANDBOX);
  out.set("x-content-type-options", "nosniff");
  return new Response(resp.body, { status: resp.status, statusText: resp.statusText, headers: out });
}

/// The web client's files: `/` and any path that isn't a file are its page, which routes itself.
async function toWeb(request, env, url) {
  if (!["GET", "HEAD"].includes(request.method)) return refuse(405, "only GET and HEAD here");
  const ask = (path) => env.ASSETS.fetch(new Request(new URL(path, url.origin), { method: request.method }));
  let path = url.pathname.endsWith("/") ? `${url.pathname}index.html` : url.pathname;
  let resp = await ask(path);
  if (resp.status === 404 && !/\.[a-z0-9]+$/i.test(url.pathname)) resp = await ask((path = "/index.html"));
  const headers = new Headers(resp.headers);
  headers.set("x-content-type-options", "nosniff");
  // No other site frames the web client; its own page frames frame.html. Each page carries its own CSP.
  headers.set("x-frame-options", "SAMEORIGIN");
  const type = TYPES[path.split(".").pop().toLowerCase()];
  if (resp.ok && type) headers.set("content-type", type);
  return new Response(resp.body, { status: resp.status, headers });
}

// The relay: how nodes reach each other, with no address of their own. Each node keeps one WebSocket here, as the key
// it proves it holds; what one sends `to` another key is passed to that node's socket, with `from` saying who sent it,
// and nothing is kept. It's blind: what nodes send each other is sealed to the receiver's key and signed by the sender
// (src/node/relay.rs), so it reads, changes and passes off nothing; it can only drop it.

/// What a node signs to prove its key: this, then the challenge.
const PROVE = "relay:";
/// The most sockets the relay keeps, the most nodes one message goes to, the most messages a socket sends a minute
/// (a big one goes in pieces of half a megabyte), and how long a socket may wait before proving its key.
const SOCKETS = 256;
const RECIPIENTS = 64;
const PER_MINUTE = 3000;
const PROVE_WITHIN = 30_000;

export class Relay {
  constructor(ctx) {
    this.ctx = ctx;
  }

  async fetch(request) {
    const url = new URL(request.url);
    // From the serverless node, by its binding (the gateway passes on only WebSockets): a notice for nodes.
    if (url.pathname === "/notice") {
      this.pass("serverless", await request.json());
      return new Response(null, { status: 204 });
    }
    const key = url.searchParams.get("key") || "";
    if (request.headers.get("upgrade") !== "websocket" || !/^[0-9a-f]{64}$/.test(key)) return refuse(426, "the relay takes a node's WebSocket, with its key");
    // Sockets that never proved their key go first.
    const now = Date.now();
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (!said.proven && now - said.at > PROVE_WITHIN) ws.close(1008, "it never proved its key");
    }
    if (this.ctx.getWebSockets().length >= SOCKETS) return refuse(503, "the relay has as many nodes as it takes");
    const [client, server] = Object.values(new WebSocketPair());
    const challenge = [...crypto.getRandomValues(new Uint8Array(16))].map((b) => b.toString(16).padStart(2, "0")).join("");
    this.ctx.acceptWebSocket(server, [key]);
    server.serializeAttachment({ key, challenge, proven: false, at: now });
    server.send(JSON.stringify({ challenge }));
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws, message) {
    const said = ws.deserializeAttachment();
    let msg;
    try {
      msg = JSON.parse(message);
    } catch {
      return ws.close(1003, "not JSON");
    }
    if (!said.proven) {
      if (!(await proves(said.key, said.challenge, msg.signature))) return ws.close(1008, "that signature isn't the key's");
      // The newest socket of a key is the one it's reached at.
      for (const other of this.ctx.getWebSockets(said.key)) if (other !== ws) other.close(1000, "the node connected again");
      ws.serializeAttachment({ ...said, proven: true, minute: Date.now(), sent: 0 });
      return ws.send(JSON.stringify({ ready: true }));
    }
    const now = Date.now();
    const fresh = now - said.minute >= 60_000;
    const sent = fresh ? 1 : said.sent + 1;
    if (sent > PER_MINUTE) return ws.close(1008, `more than ${PER_MINUTE} messages in a minute`);
    ws.serializeAttachment({ ...said, minute: fresh ? now : said.minute, sent });
    const missed = this.pass(said.key, msg);
    // An ask of a node that isn't here is answered now, rather than waited on.
    if (msg.kind === "ask" && missed.length) ws.send(JSON.stringify({ from: missed[0], id: msg.id, gone: true }));
  }

  /// Passes `msg` on from `from` to each node it's `to` (a key, or a list of them): → those not connected.
  pass(from, msg) {
    const missed = [];
    const body = JSON.stringify({ ...msg, to: undefined, from });
    const recipients = [...new Set([msg.to].flat().filter((k) => typeof k === "string"))];
    if (recipients.length > RECIPIENTS) return missed;
    for (const to of recipients) {
      const ws = this.ctx.getWebSockets(to).find((w) => w.deserializeAttachment().proven);
      if (ws) ws.send(body);
      else missed.push(to);
    }
    return missed;
  }

  webSocketClose(ws, code) {
    try {
      ws.close(code === 1005 ? 1000 : code, "closed");
    } catch {}
  }
}

/// Whether `signature` (hex) is `key`'s (hex, Ed25519) over the challenge.
async function proves(key, challenge, signature) {
  if (typeof signature !== "string" || !/^[0-9a-f]{128}$/.test(signature)) return false;
  const bytes = (hex) => Uint8Array.from(hex.match(/../g), (b) => parseInt(b, 16));
  try {
    const public_ = await crypto.subtle.importKey("raw", bytes(key), { name: "Ed25519" }, false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", public_, bytes(signature), new TextEncoder().encode(PROVE + challenge));
  } catch {
    return false;
  }
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    // One relay for the install: only nodes' WebSockets reach it here.
    if (url.pathname === "/relay") {
      if (request.headers.get("upgrade") !== "websocket") return refuse(426, "the relay takes a node's WebSocket");
      return env.RELAY.get(env.RELAY.idFromName("relay")).fetch(request);
    }
    switch (where(url.pathname)) {
      case "auth": return env.AUTH.fetch(request);
      case "node": return toNode(request, env, url);
      default: return toWeb(request, env, url);
    }
  },
};
