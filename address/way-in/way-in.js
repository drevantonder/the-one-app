// The way in for outsiders: an assistant running somewhere else calls a tool here over the web, as MCP over HTTP,
// and the call is passed through the relay to a server acting for whoever it's from, by that server's key.

const refuse = (status, why) =>
  Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";

export default {
  async fetch(request, env) {
    // Signing in is a plugin that isn't built, so nobody outside is anyone and everything is refused.
    // `DEVELOPMENT_WITHOUT_SIGN_IN` lets every request in instead, as the one user a network without sign-in has:
    // it's set only where a network is being built, never deployed.
    if (env.DEVELOPMENT_WITHOUT_SIGN_IN !== "true") return refuse(401, "Sign in to reach this network");
    // MCP's streamable HTTP: each message is a POST, answered with JSON. There's no stream to GET.
    if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
    const message = await request.text();
    // Whose a request is, is a server's to say once users sign in. Until then one acting for the network is asked.
    for (const server of await env.SIGNER.servers(NETWORK)) {
      const answer = await env.RELAY.pass(server, message);
      if (answer === null) continue;
      // A notification has no answer.
      if (answer === "") return new Response(null, { status: 202 });
      return new Response(answer, { headers: { "content-type": "application/json" } });
    }
    return refuse(503, "None of this network's servers is up");
  },
};
