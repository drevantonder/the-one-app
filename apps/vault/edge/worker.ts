// The Vault's edge function. An outside service sends whoever signed in to it back here
// (`/vault/callback`), with a code and the `state` their sign-in began with, and this passes both through the relay
// to a server acting for whoever's sign-in it is, which the state starts with. That server swaps the code for
// tokens and keeps them in their Vault: this part holds nothing, and a code alone redeems nothing, since only the
// server holds what proves who asked for it (PKCE). A request for a key's link (`/vault/request?id=`) opens it in the
// Vault, by the web client's link to an app's place (`/?open=<one: link>`), as the app on a device opens it.
import { env } from "cloudflare:workers";

declare global {
  namespace Cloudflare {
    /** Its bindings: its page's files, and what the edge gives every app's part (`core/edge/edge.js`). */
    interface Env {
      ASSETS: Fetcher;
      NETWORK: { servers(of?: string): Promise<string[]> };
      RELAY: { forward(servers: string[], message: string): Promise<{ server: string; answer: string } | null> };
    }
  }
}

/** Whose something is: a user, a group or the network, as relation tuples name them. */
const OWNER = /^(?:(?:users|groups)\/[a-z][a-z0-9-]*|network)$/;

/** Whose sign-in a `state` says it is, or nothing if it isn't one a server here made. */
function whose(state: string) {
  try {
    const owner = atob(state.split(".")[0].replace(/-/g, "+").replace(/_/g, "/"));
    return OWNER.test(owner) ? owner : null;
  } catch {
    return null;
  }
}

/**
 * Finishes a sign-in, or with `again` starts it again: has a server acting for whoever's it is take what the service
 * sent back. Gives what the page then shows: how it ended and where it began, or only a `problem` where it can't say.
 */
async function callback(params: URLSearchParams, again: boolean) {
  const state = params.get("state") ?? "";
  const owner = whose(state);
  if (!owner) return { problem: "This sign-in isn't one this network began." };
  const inputs = {
    state,
    ...(params.has("code") && { code: params.get("code") }),
    ...(params.has("error") && { error: params.get("error") }),
    ...(again && { again }),
  };
  // Whoever comes back is nobody the network knows: what they hold is the state, as a sharing link's holder holds
  // a link, and the Vault's tool checks it's the one it gave.
  const call = { name: "vault_oauth_callback", arguments: inputs, _meta: { link: `${owner}/${state}` } };
  const message = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: call });
  const servers = new Set([...(await env.NETWORK.servers(`${owner}/`)), ...(await env.NETWORK.servers())]);
  const reached = await env.RELAY.forward([...servers], message);
  if (!reached) return { problem: "None of this network's servers answered. Start the sign-in again once one is on." };
  type Result = { isError?: boolean; content?: { text?: string }[]; structuredContent?: Record<string, unknown> };
  const { result, error }: { result?: Result; error?: { message?: string } } = JSON.parse(reached.answer);
  if (result && !result.isError && result.structuredContent) return result.structuredContent;
  return { problem: result?.content?.[0]?.text ?? error?.message ?? "The server didn't say what happened." };
}

export default {
  async fetch(request: Request) {
    const url = new URL(request.url);
    // The page asks for what it shows once it's open, so the answer is words in the page, never a page of its own.
    if (url.pathname === "/vault/callback" && request.method === "POST") {
      const { again } = (await request.json().catch(() => ({}))) as { again?: unknown };
      return Response.json(await callback(url.searchParams, again === true));
    }
    if (url.pathname === "/vault/callback") return env.ASSETS.fetch(new URL("/vault/", url));
    if (url.pathname === "/vault/request") {
      const id = url.searchParams.get("id");
      return Response.redirect(new URL(`/?open=${encodeURIComponent(id ? `vault#request=${id}` : "vault")}`, url).href, 302);
    }
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler;
