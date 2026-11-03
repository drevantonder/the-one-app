// The gateway: the install's address, the one people use. It holds no keys and runs no app code; it only sends each
// request where its path says: sign-in to the auth service (with the gateway's host kept, since passkeys belong to
// it), `/urls` to the URL signer, `/mcp` and what MCP clients read to sign in to the MCP Worker, `/relay` (and iroh's
// `/ping` and `/generate_204`) to the relay, and everything else to the web client's files. Devices reach the
// install's nodes over iroh, never through here.

/// Paths the auth service answers: exactly these, or under the ones ending in `/`.
const AUTH = ["/api/auth/", "/sign-in", "/invite", "/passkeys", "/.well-known/oauth-authorization-server", "/.well-known/openid-configuration"];
/// The MCP Worker's: MCP, its token endpoint, the resource's metadata, and the authorization server's as MCP clients
/// find it (RFC 8414's path for the issuer `<address>/api/auth`), which names its token endpoint.
const MCP = ["/mcp", "/.well-known/oauth-protected-resource", "/.well-known/oauth-authorization-server/api/auth", "/.well-known/openid-configuration/api/auth"];

const TYPES = {
  html: "text/html; charset=utf-8", js: "text/javascript; charset=utf-8", mjs: "text/javascript; charset=utf-8", css: "text/css; charset=utf-8",
  json: "application/json", webmanifest: "application/manifest+json", svg: "image/svg+xml", png: "image/png", jpg: "image/jpeg",
  jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", ico: "image/x-icon", woff2: "font/woff2", txt: "text/plain; charset=utf-8", wasm: "application/wasm",
};

const under = (path, prefixes) => prefixes.some((p) => (p.endsWith("/") ? path.startsWith(p) : path === p || path.startsWith(`${p}/`)));

/// Where a path goes: `mcp`, `auth` or `web`.
function where(path) {
  if (under(path, MCP)) return "mcp";
  if (under(path, AUTH)) return "auth";
  return "web";
}

const refuse = (status, error) => Response.json({ error }, { status, headers: { "x-content-type-options": "nosniff" } });

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
const sha256 = async (data) => hex(await crypto.subtle.digest("SHA-256", typeof data === "string" ? new TextEncoder().encode(data) : data));

/// Phosphor's icons (`icons.json`), read once per isolate, with the start of its SHA-256 that names it.
let icons = null;

/// `/icons?names=a,b&v=…`: only the icons named, of the set `v` names, so a screen's few don't cost the whole set.
async function someIcons(env, url) {
  icons ||= env.ASSETS.fetch(new Request(new URL("/icons.json", url.origin))).then(async (r) => {
    const body = await r.arrayBuffer();
    return { all: JSON.parse(new TextDecoder().decode(body)), v: await sha256(body) };
  }).catch((err) => ((icons = null), Promise.reject(err)));
  const { all, v } = await icons;
  const names = (url.searchParams.get("names") || "").split(",").slice(0, 1000);
  const asked = url.searchParams.get("v") || "";
  const named = asked.length >= 12 && v.startsWith(asked);
  return Response.json(Object.fromEntries(names.filter((n) => Object.hasOwn(all, n)).map((n) => [n, all[n]])), {
    headers: { "cache-control": named ? "public, max-age=31536000, immutable" : "no-cache", "x-content-type-options": "nosniff" },
  });
}

/// The web client's files: `/` and any path that isn't a file are its page, which routes itself.
async function toWeb(request, env, url) {
  if (!["GET", "HEAD"].includes(request.method)) return refuse(405, "only GET and HEAD here");
  if (url.pathname === "/icons") return someIcons(env, url);
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
  // The client's WebAssembly is at a path named by what it is, so it's never fetched twice.
  if (resp.ok && path.startsWith("/client/")) {
    headers.set("cache-control", "public, max-age=31536000, immutable");
    return new Response(resp.body, { status: resp.status, headers });
  }
  // A file named by its content (`?v=`, the start of its SHA-256, as index.html names it) is kept for good, by the
  // browser and Cloudflare's edge, once it's checked to be that content; anything else is asked about each time.
  const v = url.searchParams.get("v");
  if (v && resp.ok && request.method === "GET") {
    const body = await resp.arrayBuffer();
    const named = (await sha256(body)).startsWith(v) && v.length >= 12;
    headers.set("cache-control", named ? "public, max-age=31536000, immutable" : "no-cache");
    return new Response(body, { status: resp.status, headers });
  }
  headers.set("cache-control", "no-cache");
  return new Response(resp.body, { status: resp.status, headers });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    // The relay, iroh's: its WebSockets, its probes, and what devices tell it go to its own Worker (kernel/relay/).
    if (["/relay", "/relay/tell", "/relay/computes", "/ping", "/generate_204"].includes(url.pathname)) return env.IROH.fetch(request);
    if (url.pathname === "/urls") return env.SIGNER.fetch(request);
    switch (where(url.pathname)) {
      case "mcp": return env.MCP ? env.MCP.fetch(request) : refuse(503, "this install's MCP isn't set up yet");
      case "auth": return env.AUTH.fetch(request);
      default: return toWeb(request, env, url);
    }
  },
};
