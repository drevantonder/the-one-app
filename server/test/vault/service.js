// An outside service, for the Vault's proof (`server/test/vault.sh`): one that signs people in with OAuth and
// answers what they ask with the tokens it gave. It runs on this machine and stands for Google or anyone else.
//
//     MARKER=<text> bun service.js <port>
//
// Every secret it makes carries MARKER, so the proof can search for it wherever it mustn't be. Its sign-in page
// asks nothing: whoever opens it is signed in and sent back at once. An access token lasts LASTS seconds, which
// is short, whatever the service said when it gave it, so the proof sees one renewed once the service refuses it.
import { createHash } from "node:crypto";

const { MARKER, LASTS = "2" } = process.env;
const client = { id: "proof-client", secret: `${MARKER}-client-secret` };
/** The codes given and not yet redeemed, each with the challenge its redeemer has to meet. */
const codes = new Map();
/** The tokens that work, by token, with when each ends, and the refresh tokens. */
const access = new Map();
const refresh = new Set();
/** What happened, for the proof to read: never a secret. */
const seen = { signed_in: 0, exchanged: 0, renewed: 0, asked: 0, refused: 0, without_verifier: 0 };
let made = 0;

const json = (body, status = 200) => Response.json(body, { status });
const tokens = () => {
  const token = `${MARKER}-access-${++made}`;
  const again = `${MARKER}-refresh-${made}`;
  access.set(token, Date.now() + Number(LASTS) * 1000);
  refresh.add(again);
  return { access_token: token, refresh_token: again, token_type: "Bearer", expires_in: 3600 };
};

Bun.serve({
  port: Number(process.argv[2]),
  hostname: "127.0.0.1",
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/authorize") {
      const asked = url.searchParams;
      if (asked.get("client_id") !== client.id || asked.get("code_challenge_method") !== "S256") return json({ error: "invalid_request" }, 400);
      // A code is no secret: it passes through the browser and the network's address, and redeems nothing alone.
      const code = `code-${crypto.randomUUID()}`;
      codes.set(code, { challenge: asked.get("code_challenge"), back: asked.get("redirect_uri") });
      seen.signed_in++;
      const back = new URL(asked.get("redirect_uri"));
      back.searchParams.set("code", code);
      back.searchParams.set("state", asked.get("state"));
      return Response.redirect(back, 302);
    }
    if (url.pathname === "/token" && request.method === "POST") {
      const form = new URLSearchParams(await request.text());
      if (form.get("client_id") !== client.id || form.get("client_secret") !== client.secret) return json({ error: "invalid_client" }, 401);
      if (form.get("grant_type") === "authorization_code") {
        const code = codes.get(form.get("code"));
        codes.delete(form.get("code"));
        const met = createHash("sha256").update(form.get("code_verifier") ?? "").digest("base64url");
        if (!form.get("code_verifier")) seen.without_verifier++;
        if (!code || code.challenge !== met || code.back !== form.get("redirect_uri")) return json({ error: "invalid_grant" }, 400);
        seen.exchanged++;
        return json(tokens());
      }
      if (form.get("grant_type") === "refresh_token") {
        if (!refresh.delete(form.get("refresh_token"))) return json({ error: "invalid_grant" }, 400);
        seen.renewed++;
        return json(tokens());
      }
      return json({ error: "unsupported_grant_type" }, 400);
    }
    if (url.pathname === "/me") {
      const token = request.headers.get("authorization")?.replace(/^Bearer /, "");
      if (!(access.get(token) > Date.now())) return (seen.refused++, json({ error: "invalid_token" }, 401));
      seen.asked++;
      // It says the token back, as a careless service might: the Vault blots it out of what a tool answers.
      return json({ account: "person@fake.example", you_sent: token });
    }
    // What it was sent in `x-key`, said back.
    if (url.pathname === "/echo") return json({ you_sent: request.headers.get("x-key") });
    if (url.pathname === "/seen") return json(seen);
    return json({ error: "not_found" }, 404);
  },
});
