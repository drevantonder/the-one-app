// The Vault's edge function. An outside service sends whoever signed in to it back here
// (`/vault/callback`), with a code and the `state` their sign-in began with, and this passes both through the relay
// to a server acting for whoever's sign-in it is, which the state starts with. That server swaps the code for
// tokens and keeps them in their Vault: this part holds nothing, and a code alone redeems nothing, since only the
// server holds what proves who asked for it (PKCE). A request for a key leads here too (`/vault/request`), to a
// page that says where it's answered.
import { env } from "cloudflare:workers";

declare global {
  namespace Cloudflare {
    /** Its bindings: its page's files, and what the edge gives every plugin's part (`core/edge/edge.js`). */
    interface Env {
      ASSETS: Fetcher;
      NETWORK: { servers(of: string): Promise<string[]> };
      RELAY: { forward(server: string, message: string): Promise<string | null> };
    }
  }
}

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";
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

/** Finishes a sign-in: has a server acting for whoever's it is take the code. Gives what the page then says. */
async function callback(asked: URLSearchParams) {
  const state = asked.get("state") ?? "";
  const owner = whose(state);
  if (!owner) return { problem: "This sign-in isn't one this network began." };
  const said = { state, ...(asked.has("code") && { code: asked.get("code") }), ...(asked.has("error") && { error: asked.get("error") }) };
  // Whoever comes back is nobody the network knows: what they hold is the state, as a sharing link's holder holds
  // a link, and the Vault's tool checks it's the one it gave.
  const call = { name: "vault_oauth_callback", arguments: said, _meta: { link: `${owner}/${state}` } };
  const message = JSON.stringify({ jsonrpc: "2.0", id: 1, method: "tools/call", params: call });
  const servers = new Set([...(await env.NETWORK.servers(`${owner}/`)), ...(await env.NETWORK.servers(NETWORK))]);
  for (const server of servers) {
    const answer = await env.RELAY.forward(server, message);
    if (answer === null) continue;
    const { result, error }: { result?: { isError?: boolean; content?: { text?: string }[] }; error?: { message?: string } } = JSON.parse(answer);
    const text = result?.content?.[0]?.text ?? error?.message ?? "The server didn't say what happened.";
    return result && !result.isError ? { done: text } : { problem: text };
  }
  return { problem: "None of this network's servers is up. Start the sign-in again once one is." };
}

export default {
  async fetch(request: Request) {
    const url = new URL(request.url);
    // The page asks for what it shows once it's open, so the answer is words in the page, never a page of its own.
    if (url.pathname === "/vault/callback" && request.method === "POST") return Response.json(await callback(url.searchParams));
    if (["/vault/callback", "/vault/request"].includes(url.pathname)) return env.ASSETS.fetch(new URL("/vault/", url));
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler;
