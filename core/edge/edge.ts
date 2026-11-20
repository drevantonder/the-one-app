// The network's edge, one Worker at the network's address: each request goes where the first part of its path says,
// to a piece here (`paths.json`), an app's part (`PARTS`, which `edge.js` makes from each app's `edge/package.json`),
// or the web client's files. The pieces call each other as functions; an app's part is a Worker of its own, bound
// here by `edge.js`, and bound in turn to `Network`, `Own` and `RelayEntrypoint`. The relay is a Worker of its own
// (`relay/worker.ts`), which holds every server's and device's connection.
import PATHS from "./paths.json";
import mcp from "./mcp/mcp.ts";
import presigner from "./presigner/presigner.ts";
import relay from "./relay/relay.ts";
import web from "./web/web.ts";

export { Network, Own, Changes } from "./presigner/presigner.ts";
export { RelayEntrypoint } from "./relay/relay.ts";
export { NetworkUpdater } from "./updater/updater.ts";

/** What `edge.js` gives the edge beside what `wrangler.jsonc` says (`worker-configuration.d.ts`). */
declare global {
  interface Env {
    /** The edge's build, as `tooling/version` names the commit it was deployed from. */
    VERSION: string;
    /** Which app's part answers each path's first part, by its binding. */
    PARTS: Record<string, string>;
    /** The web client's files. */
    ASSETS: Fetcher;
    /** The relay's Durable Object, in its own Worker (`relay/worker.ts`). */
    RELAY: DurableObjectNamespace<import("./relay/worker.ts").Relay>;
  }
}

const PIECES = { mcp, presigner, relay };

export default {
  fetch(request, env, ctx) {
    const first = new URL(request.url).pathname.split("/")[1];
    const piece = PATHS[first as keyof typeof PATHS];
    if (piece) return PIECES[piece as keyof typeof PIECES].fetch(request, env, ctx);
    const part = env.PARTS[first];
    if (part) return (env[part as keyof Env] as unknown as Fetcher).fetch(request);
    return web.fetch(request, env);
  },
} satisfies ExportedHandler<Env>;
