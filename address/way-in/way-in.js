// The way in for outsiders: an assistant running somewhere else calls a tool here over the web, as MCP over HTTP,
// and the call is passed through the relay to a server acting for whoever it's from, by that server's key.

const refuse = (status, why) =>
  Response.json({ jsonrpc: "2.0", id: null, error: { code: -32000, message: why } }, { status });

/**
 * Whose a request is: the prefix of the user it's from, or null. Signing in is a plugin that isn't built, so nobody
 * outside is anyone and everything is refused. `DEVELOPMENT_WITHOUT_SIGN_IN` makes every request the one user a
 * network without sign-in has: it's set only where a network is being built, never deployed.
 */
function from(_request, env) {
  return env.DEVELOPMENT_WITHOUT_SIGN_IN === "true" ? "users/anyone/" : null;
}

export default {
  async fetch(request, env) {
    const whose = from(request, env);
    if (!whose) return refuse(401, "Sign in to reach this network");
    // MCP's streamable HTTP: each message is a POST, answered with JSON. There's no stream to GET.
    if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
    const message = await request.text();
    for (const server of await env.SIGNER.servers(whose)) {
      const answer = await env.RELAY.pass(server, message);
      if (answer === null) continue;
      // A notification has no answer.
      if (answer === "") return new Response(null, { status: 202 });
      return new Response(answer, { headers: { "content-type": "application/json" } });
    }
    return refuse(503, "None of this network's servers is up");
  },
};
