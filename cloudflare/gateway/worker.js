// The gateway: the network's address, the one users use. It holds no keys and runs no app code; it only sends each
// request where its path says: signing in to auth (with the gateway's host kept, since passkeys belong to it),
// `/urls` and `/updates/…` (what devices update themselves from) to the URL signer, `/push/…` to the push Worker, `/telemetry/…` to the telemetry Worker, `/mcp` and what MCP clients read to sign in to it to the
// remote MCP server, everything under `/relay` (and iroh's `/ping` and `/generate_204`) to the relay, and everything
// else to the web client's files. Devices reach the network's nodes over iroh, never through here, and so does git: a
// git request here is git on a computer without the CLI, told how to get it.
import { hex, sha256 } from "../shared/crypto.js";
import { refusal } from "../shared/http.js";
import { IROH_PATHS, UPDATES } from "../shared/protocol.js";

const AUTH = ["/api/auth/", "/sign-in", "/invite", "/passkeys", "/.well-known/oauth-authorization-server", "/.well-known/openid-configuration"];
// The authorization server's metadata is where MCP clients look for it: RFC 8414's path for the issuer `<address>/api/auth`.
const MCP = ["/mcp", "/.well-known/oauth-protected-resource", "/.well-known/oauth-authorization-server/api/auth", "/.well-known/openid-configuration/api/auth"];

const TYPES = {
  html: "text/html; charset=utf-8", js: "text/javascript; charset=utf-8", mjs: "text/javascript; charset=utf-8", css: "text/css; charset=utf-8",
  json: "application/json", webmanifest: "application/manifest+json", svg: "image/svg+xml", png: "image/png", jpg: "image/jpeg",
  jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", ico: "image/x-icon", woff2: "font/woff2", txt: "text/plain; charset=utf-8", wasm: "application/wasm",
};

const under = (path, prefixes) => prefixes.some((p) => (p.endsWith("/") ? path.startsWith(p) : path === p || path.startsWith(`${p}/`)));

function where(path) {
  if (under(path, MCP)) return "mcp";
  if (under(path, AUTH)) return "auth";
  return "web";
}

const NOSNIFF = { "x-content-type-options": "nosniff" };
const digest = async (data) => hex.encode(await sha256(data));

let icons = null;

async function someIcons(env, url) {
  icons ||= env.ASSETS.fetch(new Request(new URL("/icons.json", url.origin))).then(async (r) => {
    const body = await r.arrayBuffer();
    return { all: JSON.parse(new TextDecoder().decode(body)), v: await digest(body) };
  }).catch((err) => ((icons = null), Promise.reject(err)));
  const { all, v } = await icons;
  const names = (url.searchParams.get("names") || "").split(",").slice(0, 1000);
  const asked = url.searchParams.get("v") || "";
  const named = asked.length >= 12 && v.startsWith(asked);
  return Response.json(Object.fromEntries(names.filter((n) => Object.hasOwn(all, n)).map((n) => [n, all[n]])), {
    headers: { "cache-control": named ? "public, max-age=31536000, immutable" : "no-cache", "x-content-type-options": "nosniff" },
  });
}

async function toWeb(request, env, url) {
  if (!["GET", "HEAD"].includes(request.method)) return refusal(405, "only GET and HEAD here", NOSNIFF);
  if (url.pathname === "/icons") return someIcons(env, url);
  const ask = (path) => env.ASSETS.fetch(new Request(new URL(path, url.origin), { method: request.method }));
  let path = url.pathname.endsWith("/") ? `${url.pathname}index.html` : url.pathname;
  let resp = await ask(path);
  if (resp.status === 404 && !/\.[a-z0-9]+$/i.test(url.pathname)) resp = await ask((path = "/index.html"));
  const headers = new Headers(resp.headers);
  headers.set("x-content-type-options", "nosniff");
  headers.set("vary", "accept-encoding");
  // No other site frames the web client; its own page frames frame.html. Each page carries its own CSP.
  headers.set("x-frame-options", "SAMEORIGIN");
  const type = TYPES[path.split(".").pop().toLowerCase()];
  if (resp.ok && type) headers.set("content-type", type);
  // `<file>.br`, made with the Workers' files, is a fifth smaller than the edge's own compression.
  const packed = resp.ok && request.method === "GET" && /\bbr\b/.test(request.headers.get("accept-encoding") || "") && !path.endsWith(".br") ? await ask(`${path}.br`) : null;
  const send = (body) => {
    if (!packed?.ok) return new Response(body, { status: resp.status, headers });
    resp.body?.cancel().catch(() => {});
    headers.set("content-encoding", "br");
    if (packed.headers.has("content-length")) headers.set("content-length", packed.headers.get("content-length"));
    else headers.delete("content-length");
    headers.delete("etag");
    return new Response(packed.body, { status: resp.status, headers, encodeBody: "manual" });
  };
  if (resp.ok && path.startsWith("/onet/")) {
    headers.set("cache-control", "public, max-age=31536000, immutable");
    return send(resp.body);
  }
  const v = url.searchParams.get("v");
  if (v && resp.ok && request.method === "GET") {
    const body = await resp.arrayBuffer();
    const named = (await digest(body)).startsWith(v) && v.length >= 12;
    headers.set("cache-control", named ? "public, max-age=31536000, immutable" : "no-cache");
    return send(body);
  }
  headers.set("cache-control", "no-cache");
  return send(resp.body);
}

// Stock git's requests to any app's address: a `.git` in the path, or git's own files and services.
const GIT = /\.git(\/|$)|\/(info\/refs|git-upload-pack|git-receive-pack)$/;

// Git shows this body as the remote's error.
function gitWithoutCli(env, url) {
  const name = env.NAME || "The One App", cli = env.CLI || "one";
  const said = [
    `${url.origin}${url.pathname.replace(/(\.git)\/.*$|\/(info\/refs|git-[a-z-]+)$/, "$1")} is a git address in ${env.YOURS || "your One App network"}, which git reaches through ${name}'s CLI, ${cli}.`,
    `Install it on this computer, join ${env.YOURS || "your One App network"} there with`,
    `  ${cli} join <pairing code>`,
    `(${cli} pair on a device in it gives one), and try again.`,
  ];
  return new Response(said.join("\n") + "\n", { status: 403, headers: { "content-type": "text/plain; charset=utf-8", "x-content-type-options": "nosniff" } });
}

// The native apps' windows, whose client (net-worker.js) reaches this address as a page at their own origin does.
const APPS = ["tauri://localhost", "https://tauri.localhost", "http://tauri.localhost"];

function letApps(request, respond) {
  const origin = request.headers.get("origin");
  if (!APPS.includes(origin)) return respond();
  const headers = { "access-control-allow-origin": origin, "access-control-allow-methods": "GET, POST, HEAD, OPTIONS", "access-control-allow-headers": request.headers.get("access-control-request-headers") || "content-type", vary: "origin" };
  if (request.method === "OPTIONS") return Promise.resolve(new Response(null, { status: 204, headers }));
  return respond().then((resp) => {
    if (resp.webSocket || resp.status === 101) return resp;
    const out = new Response(resp.body, resp);
    for (const [name, value] of Object.entries(headers)) out.headers.set(name, value);
    return out;
  });
}

export default {
  fetch(request, env) {
    return letApps(request, () => route(request, env));
  },
};

async function route(request, env) {
  {
    const url = new URL(request.url);
    if (under(url.pathname, Object.values(IROH_PATHS))) return env.IROH.fetch(request);
    if (url.pathname === "/urls" || url.pathname.startsWith(`/${UPDATES}/`)) return env.SIGNER.fetch(request);
    if (url.pathname.startsWith("/push/")) return env.PUSH.fetch(request);
    if (url.pathname.startsWith("/telemetry/")) return env.TELEMETRY.fetch(request);
    if (GIT.test(url.pathname)) return gitWithoutCli(env, url);
    switch (where(url.pathname)) {
      case "mcp": return env.MCP ? env.MCP.fetch(request) : refusal(503, "this network's MCP isn't set up yet", NOSNIFF);
      case "auth": return env.AUTH.fetch(request);
      default: return toWeb(request, env, url);
    }
  }
}
