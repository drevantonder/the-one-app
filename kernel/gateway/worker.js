// The gateway: the install's address, the one people use. It holds no keys and runs no app code; it only sends each
// request where its path says: sign-in to the auth service (with the gateway's host kept, since passkeys belong to
// it), `/v1/` and `/mcp` to a node, and everything else to the web client's files.
//
// What a node answers is stamped so it can't run as script at this origin, which holds browsers' tokens: it's
// sandboxed into an opaque origin, can't be sniffed into another type, and sets no cookies here.

/// Paths the auth service answers: exactly these, or under the ones ending in `/`.
const AUTH = ["/api/auth/", "/sign-in", "/consent", "/.well-known/oauth-authorization-server", "/.well-known/openid-configuration"];
const NODE = ["/v1/", "/mcp"];

/// What a node's answer is stamped with.
const SANDBOX = "sandbox allow-scripts allow-forms allow-popups allow-downloads";

/// The request headers a node gets: never the browser's cookies, which are the auth service's.
const TO_NODE = ["authorization", "content-type", "accept", "accept-language", "last-event-id"];
/// The answer headers a node may give.
const FROM_NODE = ["content-type", "cache-control", "etag", "last-modified", "content-disposition", "retry-after"];

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
  if (!env.NODE_URL) return refuse(503, "this install's kernel has no node to send that to yet");
  const init = { method: request.method, headers, body: ["GET", "HEAD"].includes(request.method) ? null : request.body, redirect: "manual" };
  let resp;
  try {
    resp = await fetch(`${env.NODE_URL}${url.pathname}${url.search}`, init);
  } catch {
    return refuse(502, "the node can't be reached");
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
  const type = TYPES[path.split(".").pop().toLowerCase()];
  if (resp.ok && type) headers.set("content-type", type);
  return new Response(resp.body, { status: resp.status, headers });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    switch (where(url.pathname)) {
      case "auth": return env.AUTH.fetch(request);
      case "node": return toNode(request, env, url);
      default: return toWeb(request, env, url);
    }
  },
};
