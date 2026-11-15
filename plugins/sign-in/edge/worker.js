// Signing in's edge part. It serves the sign-in page and what the page asks (`/sign-in/`), is the
// OAuth provider that MCP's authorization asks for (`/.well-known`), and is the way in over the web for whatever
// has signed in (`/mcp`): an assistant somewhere else with the token a person let it have, or one of the
// network's own devices with its key. Either way the call is passed through the relay to a server, which is told
// which device it's from and decides whose that is itself.
// `/llms.txt` tells an assistant that's pointed at the network how to come in, as Settings › Connect Your AI asks.
import { env } from "cloudflare:workers";
import { requireMcpAuth } from "@better-auth/mcp";
import { asked, assistant, at } from "./auth.js";

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";
/** The JSON-RPC error a server answers a device that's nobody's (`OneCore.Mcp`). */
const NOBODY = -32003;

const refuse = (status, why) => Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const auth = await at(url.origin);
    if (url.pathname === "/llms.txt") return new Response(llms(url.origin), { headers: { "content-type": "text/markdown; charset=utf-8" } });
    if (url.pathname === "/mcp") return mcp(request, auth, url);
    if (url.pathname === "/sign-in/ask") return ask(request, auth, url);
    if (url.pathname.startsWith("/sign-in/auth/") || url.pathname.startsWith("/.well-known/")) return auth.handler(request);
    return env.ASSETS.fetch(request);
  },
};

/** What an assistant reads to come into the network at `origin`, as llmstxt.org has it. */
const llms = (origin) => `# ${new URL(origin).host}

> A One App network: a family's or a workplace's apps, files and devices. An assistant comes in as the person who
> lets it in, and does what they can.

## Connecting

- Add ${origin}/mcp as a remote MCP server (a connector), over streamable HTTP. In Claude Code:
  \`claude mcp add --transport http one ${origin}/mcp\`
- It signs in with OAuth. The person is asked for their passkey once, and that's their yes.
- Then their tools are yours: \`tools/list\` lists them. Read a plugin's skill before using its tools:
  \`resources/list\` lists them, under \`skill://\`, and \`skill://building-plugins/SKILL.md\` says how to make one.
`;

/** The key that signed what was asked, as the pieces of the edge check it, or null. */
const signerKey = (request, body) => env.NETWORK.verifiedKey(request.url, Object.fromEntries(request.headers), body);

async function mcp(request, auth, url) {
  // MCP's streamable HTTP: each message is a POST, answered with JSON. There's no stream to GET.
  if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });

  // One of the network's own devices signs what it asks with its key. One that's nobody's is told so by the
  // server, and asks for a code to be let in by (`auth.js`).
  if (request.headers.has("one-key")) {
    const body = await request.text();
    const device = await signerKey(request, body);
    if (!device) return refuse(401, "Sign what you ask with your key, by a clock within a minute of the right time");
    return forward(body, device, [NETWORK]);
  }

  // An assistant somewhere else shows the token it was given when a person let it in. One they've since removed
  // is nobody's, and is asked to be let in again.
  const resource = `${url.origin}/mcp`;
  const let_in = requireMcpAuth(
    auth,
    async (request, claims) => {
      const answer = await forward(await request.text(), await assistant(claims.azp, claims.sub), [`${claims.who}/`, NETWORK]);
      if (!(await nobodys(answer))) return answer;
      const challenge = `Bearer error="invalid_token", resource_metadata="${url.origin}/.well-known/oauth-protected-resource/mcp"`;
      return new Response(null, { status: 401, headers: { "www-authenticate": challenge } });
    },
    { resource },
  );
  return let_in(request);
}

/**
 * Passes one JSON-RPC message to a server acting for one of `of`, as from `device`, and answers what the server
 * did. What's asked says which device it's from and who it's for, and nothing else of itself.
 */
async function forward(message, device, of) {
  let call;
  try {
    call = JSON.parse(message);
    if (typeof call?.method !== "string") throw new Error("not a request");
  } catch {
    return refuse(400, "That isn't a JSON-RPC request");
  }
  const asks_for = call.params?._meta?.for;
  const said = { device, ...(typeof asks_for === "string" && { for: asks_for }) };
  message = JSON.stringify({ ...call, params: { ...call.params, _meta: said } });
  const servers = new Set((await Promise.all(of.map((of) => env.NETWORK.servers(of)))).flat());
  for (const server of servers) {
    const answer = await env.RELAY.forward(server, message);
    if (answer === null) continue;
    // A notification has no answer.
    if (answer === "") return new Response(null, { status: 202 });
    return new Response(answer, { headers: { "content-type": "application/json" } });
  }
  return refuse(503, "None of this network's servers is up");
}

/** Whether a server answered that the device is nobody's. The answer is left to be sent on if it isn't. */
async function nobodys(answer) {
  if (answer.status !== 200) return false;
  const said = await answer.clone().json();
  return said.error?.code === NOBODY;
}

/**
 * What a server asks for a person in the app: one acting for them, or for the network, which acts for everyone, as
 * servers have it themselves (`OneCore.Servers.acting/1`). Nothing else is believed about who that is. An
 * invitation, the first owner's included, is only the network's to make, since it can make someone nobody acts for
 * yet, and so is taking someone out and listing everyone, which a person's own server isn't told.
 */
async function ask(request, auth, url) {
  const body = await request.text();
  const server = await signerKey(request, body);
  let what;
  try {
    what = JSON.parse(body);
  } catch {
    return Response.json({ error: "An ask is JSON" }, { status: 400 });
  }
  const of = ["invite", "owner", "everyone", "remove_person"].includes(what.ask) ? [NETWORK] : [NETWORK, `${what.who}/`];
  const acting = (await Promise.all(of.map((of) => env.NETWORK.servers(of)))).flat();
  if (!server || !acting.includes(server)) return Response.json({ error: "Only a server acting for them asks this" }, { status: 403 });
  return Response.json(await asked(auth, url.origin, what));
}
