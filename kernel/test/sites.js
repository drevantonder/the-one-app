// Stand-in sites for tests. The bucket (https://s3.test/install/…, and /install/… asked directly) keeps objects in
// memory, each with an ETag that changes as it does, creating one only if it isn't there when asked with
// If-None-Match: *, and replacing one only while it's the version If-Match names. Google's token endpoint, given an
// authorization code, signs in whoever it names: the code is its ID token's claims. Anything else answers with what it
// was sent.
const objects = new Map();
const tags = new Map();
let written = 0;

export default {
  async fetch(request) {
    const url = new URL(request.url);
    const form = url.host === "oauth2.googleapis.com" && new URLSearchParams(await request.clone().text());
    if (form && form.get("grant_type") === "authorization_code") {
      const claims = form.get("code");
      const header = btoa(JSON.stringify({ alg: "RS256", typ: "JWT" })).replaceAll("=", "");
      return Response.json({ access_token: "a Google access token", token_type: "Bearer", expires_in: 3600, id_token: `${header}.${claims}.c2lnbmVk` });
    }
    if (url.host === "s3.test" || url.pathname.startsWith("/install/")) {
      const name = url.pathname;
      const etag = { etag: tags.get(name) };
      if (request.method === "GET") return objects.has(name) ? new Response(objects.get(name), { headers: etag }) : new Response("", { status: 404 });
      if (request.method === "HEAD") return new Response(null, objects.has(name) ? { headers: etag } : { status: 404 });
      if (request.method === "DELETE") return new Response(null, { status: objects.delete(name) ? 204 : 404 });
      if (request.headers.get("if-none-match") === "*" && objects.has(name)) return new Response("", { status: 412 });
      if (request.headers.has("if-match") && request.headers.get("if-match") !== tags.get(name)) return new Response("", { status: 412 });
      objects.set(name, new Uint8Array(await request.arrayBuffer()));
      tags.set(name, `"${++written}"`);
      return new Response("", { headers: { etag: tags.get(name) } });
    }
    return Response.json({ method: request.method, url: request.url, headers: Object.fromEntries(request.headers), body: await request.text() });
  },
};
