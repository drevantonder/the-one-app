// Stand-in sites for tests. The bucket (https://s3.test/install/…, and /install/… asked directly) keeps objects in
// memory, creating one only if it isn't there when asked with If-None-Match: *. Any other host answers with what it
// was sent.
const objects = new Map();

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.host === "s3.test" || url.pathname.startsWith("/install/")) {
      const name = url.pathname;
      if (request.method === "GET") return objects.has(name) ? new Response(objects.get(name)) : new Response("", { status: 404 });
      if (request.method === "DELETE") return new Response(null, { status: objects.delete(name) ? 204 : 404 });
      if (request.headers.get("if-none-match") === "*" && objects.has(name)) return new Response("", { status: 412 });
      objects.set(name, new Uint8Array(await request.arrayBuffer()));
      return new Response("");
    }
    return Response.json({ method: request.method, url: request.url, headers: Object.fromEntries(request.headers), body: await request.text() });
  },
};
