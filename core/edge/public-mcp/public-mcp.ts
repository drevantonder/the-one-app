// The public MCP endpoint for whoever holds a sharing link: they call a tool here over the web, as MCP over HTTP, and the call is
// passed through the relay to a server acting for whoever the link leads to, by that server's key. Everyone else
// outside signs in, which is a plugin's edge function.

const refuse = (status: number, why: string) =>
  Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";
/** A sharing link starts with whose it leads to, as their prefix: that's whose servers answer its holder. */
const LEADS_TO = /^(?:(?:users|groups)\/[^/]+\/|network\/)/;

export default {
  async fetch(request, env) {
    const { pathname } = new URL(request.url);
    // MCP's streamable HTTP: each message is a POST, answered with JSON. There's no stream to GET.
    if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
    // Whoever holds a sharing link needs no signing in: the link is all they are. What they ask says it's from the
    // link's holder and nothing else, whatever it said of itself, and a server refuses them everything the link's
    // relation tuple doesn't give.
    const link = decodeURIComponent(pathname.slice("/link/".length));
    const of = LEADS_TO.exec(link)?.[0];
    if (!of) return refuse(404, "That isn't a sharing link");
    let message;
    try {
      const asked = await request.json<{ method?: unknown; params?: object }>();
      if (typeof asked?.method !== "string") throw new Error("not a request");
      message = JSON.stringify({ ...asked, params: { ...asked.params, _meta: { link } } });
    } catch {
      return refuse(400, "That isn't a JSON-RPC request");
    }
    // Whose a request is, is a server's to say: one acting for whoever the link leads to, or for the network.
    const servers = new Set([...(await env.NETWORK.servers(of)), ...(await env.NETWORK.servers(NETWORK))]);
    const reached = await env.RELAY.forward([...servers], message);
    if (!reached) return refuse(503, "None of this network's servers answered. Try again once one is on.");
    // A notification has no answer.
    if (reached.answer === "") return new Response(null, { status: 202 });
    return new Response(reached.answer, { headers: { "content-type": "application/json" } });
  },
} satisfies ExportedHandler<PublicMcpEnv>;
