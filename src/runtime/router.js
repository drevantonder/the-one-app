// The one Worker the node talks to: /<app>/... goes to that app's Worker.
export default {
  fetch(req, env) {
    const app = new URL(req.url).pathname.split("/")[1];
    if (app === "ready") return new Response("ready");
    const worker = env[app];
    if (!worker) return new Response(JSON.stringify({ error: `no app named ${app}` }), { status: 404 });
    return worker.fetch(req);
  },
};
