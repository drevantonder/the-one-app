// The way in for outsiders: an assistant running somewhere else, or the holder of a sharing link, calls a tool here
// over the web, as MCP over HTTP, and the call is passed through the relay to a server acting for whoever it's
// from, by that server's key.

const refuse = (status, why) =>
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
    let message = await request.text();
    let of = NETWORK;
    if (pathname.startsWith("/link/")) {
      // Whoever holds a sharing link needs no signing in: the link is all they are. What they ask says it's from
      // the link's holder and nothing else, whatever it said of itself, and a server refuses them everything the
      // link's relation tuple doesn't give.
      const link = decodeURIComponent(pathname.slice("/link/".length));
      of = LEADS_TO.exec(link)?.[0];
      if (!of) return refuse(404, "That isn't a sharing link");
      try {
        const asked = JSON.parse(message);
        if (typeof asked?.method !== "string") throw new Error("not a request");
        message = JSON.stringify({ ...asked, params: { ...asked.params, _meta: { link } } });
      } catch {
        return refuse(400, "That isn't a JSON-RPC request");
      }
    } else if (env.DEVELOPMENT_WITHOUT_SIGN_IN !== "true") {
      // Signing in is a plugin that isn't built, so nobody else outside is anyone and everything is refused.
      // `DEVELOPMENT_WITHOUT_SIGN_IN` lets every request in instead, as the one user a network without sign-in has:
      // it's set only where a network is being built, never deployed.
      return refuse(401, "Sign in to reach this network");
    }
    // Whose a request is, is a server's to say: one acting for whoever a link leads to, or for the network.
    const servers = new Set([...(of === NETWORK ? [] : await env.SIGNER.servers(of)), ...(await env.SIGNER.servers(NETWORK))]);
    for (const server of servers) {
      const answer = await env.RELAY.pass(server, message);
      if (answer === null) continue;
      // A notification has no answer.
      if (answer === "") return new Response(null, { status: 202 });
      return new Response(answer, { headers: { "content-type": "application/json" } });
    }
    return refuse(503, "None of this network's servers is up");
  },
};
