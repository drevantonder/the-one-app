/**
 * The web client's files, as the edge builds them (`core/edge/edge.js`). One kept compressed already, as Brotli at its
 * most (`<file>.br` beside it: the renderer's WebAssembly, `clients/web/renderer/build.js`), goes as it's kept to a
 * browser that takes Brotli, where Cloudflare would compress it again as it sends, at a lower level: 1.1 MB, not 1.4.
 */
interface Env {
  ASSETS: Fetcher;
}

/** What a kept file is, by its extension. */
const TYPES: Record<string, string> = { wasm: "application/wasm", js: "text/javascript" };

export default {
  async fetch(request: Request, env: Env): Promise<Response> {
    const url = new URL(request.url);
    const type = TYPES[url.pathname.split(".").pop() ?? ""];
    if (type && /\bbr\b/.test(request.headers.get("accept-encoding") ?? "")) {
      const kept = await env.ASSETS.fetch(new Request(new URL(`${url.pathname}.br`, url), request));
      if (kept.ok) {
        const headers = new Headers(kept.headers);
        headers.set("content-type", type);
        headers.set("content-encoding", "br");
        headers.append("vary", "accept-encoding");
        return new Response(kept.body, { headers, encodeBody: "manual" });
      }
    }
    return env.ASSETS.fetch(request);
  },
} satisfies ExportedHandler<Env>;
