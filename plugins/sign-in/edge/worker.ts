// Signing in's edge function. It serves the sign-in page and what the page asks (`/sign-in/`), and is the OAuth
// provider that MCP's authorization asks for (`/.well-known`). It says who a request at the network's address is
// from (`Who`), which the network's MCP endpoint asks (`core/edge/mcp/mcp.ts`): an assistant somewhere else with the
// token a person let it have, or a browser signed in here, the web client.
import { env, WorkerEntrypoint } from "cloudflare:workers";
import { requireMcpAuth } from "@better-auth/mcp";
import { asked, assistant, at, browserDevice, who, type Ask, type Auth } from "./auth.ts";

/** A server on this list acts for the network, and so for everyone in it. */
const NETWORK = "network/";

export default {
  async fetch(request: Request) {
    const url = new URL(request.url);
    const auth = await at(url.origin);
    if (url.pathname === "/sign-in/ask") return ask(request, auth, url);
    if (url.pathname.startsWith("/sign-in/auth/") || url.pathname.startsWith("/.well-known/")) return auth.handler(request);
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler;

/** Who a request is from, as the network's MCP endpoint has it (`core/edge/mcp/mcp.ts`). */
interface Signed {
  device: string;
  who: string;
  challenge?: string;
  session?: string;
  apps?: string;
  cookies?: string[];
}

/** Says who a request at the network's address is from, for the network's MCP endpoint, which `edge.js` binds it to. */
export class Who extends WorkerEntrypoint {
  /**
   * Who `request` is from, by what it carries, its body left out: a browser by its session, either its cookie or,
   * from an app's page, the session signed into the address of its apps' files (`/apps/`), or an assistant by its
   * token. Anyone else is answered `refused`, MCP's challenge to sign in.
   */
  async who(request: Request): Promise<Signed | { refused: Response }> {
    const url = new URL(request.url);
    const auth = await at(url.origin);
    const resource = `${url.origin}/mcp`;
    if (url.pathname.startsWith("/apps/")) return (await appsSession(auth, url)) ?? { refused: new Response("Sign in again", { status: 401 }) };

    // A browser signed in here is the device its session is (`auth.ts`).
    if (!request.headers.has("authorization")) {
      const { headers, response: signedIn } = await auth.api.getSession({ headers: request.headers, returnHeaders: true });
      if (signedIn) {
        const { secret } = await auth.$context;
        const signature = await crypto.subtle.sign("HMAC", await signing(secret), signs(signedIn.session.id));
        const hex = Array.from(new Uint8Array(signature), (byte) => byte.toString(16).padStart(2, "0")).join("");
        return {
          device: await browserDevice(signedIn.session.id),
          who: who(signedIn.user),
          session: signedIn.session.id,
          apps: `/apps/${signedIn.session.id}.${hex}`,
          // Better Auth sends a session that's being used a later end.
          cookies: headers.getSetCookie(),
        };
      }
    }

    // An assistant somewhere else shows the token it was given when a person let it in. One they've since removed
    // is nobody's, and is asked to be let in again.
    let claims: Record<string, unknown> | undefined;
    const refused = await requireMcpAuth(auth, async (_, verified) => ((claims = verified), new Response()), { resource })(request);
    if (!claims) return { refused };
    return {
      device: await assistant(claims.azp as string, claims.sub as string),
      who: claims.who as string,
      challenge: `Bearer error="invalid_token", resource_metadata="${url.origin}/.well-known/oauth-protected-resource/mcp"`,
    };
  }
}

/** The key a session is signed with into the address of its apps' files: this Worker's secret, as HMAC's. */
const signing = (secret: string) => crypto.subtle.importKey("raw", new TextEncoder().encode(secret), { name: "HMAC", hash: "SHA-256" }, false, ["sign", "verify"]);
/** What's signed for a session. */
const signs = (session: string) => new TextEncoder().encode(`apps/${session}`);

/** Who an app's page's request is from, by the session signed into its address, `/apps/<session>.<signature>/…`. */
async function appsSession(auth: Auth, url: URL): Promise<Signed | null> {
  const [session, hex = ""] = url.pathname.slice("/apps/".length).split("/")[0].split(".");
  const { adapter, internalAdapter, secret } = await auth.$context;
  const signature = Uint8Array.from(hex.match(/../g) ?? [], (byte) => parseInt(byte, 16));
  const valid = await crypto.subtle.verify("HMAC", await signing(secret), signature, signs(session));
  const kept = valid && (await adapter.findOne<{ userId: string; expiresAt: Date }>({ model: "session", where: [{ field: "id", value: session }] }));
  if (!kept || kept.expiresAt < new Date()) return null;
  const user = (await internalAdapter.findUserById(kept.userId))!;
  return { device: await browserDevice(session), who: who(user) };
}

/** The key that signed what was asked, as the pieces of the edge check it, or null. */
const signerKey = (request: Request, body: string) => env.NETWORK.verifiedKey(request.url, Object.fromEntries(request.headers), body);

/**
 * What a server asks for a person in the app: one acting for them, or for the network, which acts for everyone, as
 * servers have it themselves (`OneCore.Servers.acting/1`). Nothing else is believed about who that is. An
 * invitation, the first owner's included, is only the network's to make, since it can make someone nobody acts for
 * yet, and so is taking someone out and listing everyone, which a person's own server isn't told.
 */
async function ask(request: Request, auth: Auth, url: URL) {
  const body = await request.text();
  const server = await signerKey(request, body);
  let what: Ask;
  try {
    what = JSON.parse(body);
  } catch {
    return Response.json({ error: "An ask is JSON" }, { status: 400 });
  }
  const of = ["invite", "owner", "everyone", "remove_person"].includes(what.ask ?? "") ? [NETWORK] : [NETWORK, `${what.who}/`];
  const acting = (await Promise.all(of.map((of) => env.NETWORK.servers(of)))).flat();
  if (!server || !acting.includes(server)) return Response.json({ error: "Only a server acting for them asks this" }, { status: 403 });
  return Response.json(await asked(auth, url.origin, what));
}
