// The gateway: sends each request at the network's address where its path says, and does nothing else. Which
// piece answers which path is made where the edge is deployed (`edge.js`): the pieces every edge has, and
// each app's edge function.

/** The gateway's bindings, which `edge.js` makes: which piece answers each path's first part, and each piece. */
interface GatewayEnv {
  PATHS: Record<string, string>
  [piece: string]: Fetcher | Record<string, string>
}

export default {
  fetch(request: Request, env: GatewayEnv) {
    const first = new URL(request.url).pathname.split("/")[1];
    return (env[env.PATHS[first] ?? "WEB"] as Fetcher).fetch(request);
  },
} satisfies ExportedHandler<GatewayEnv>;
