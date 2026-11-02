// A stand-in node for tests: it takes the schedules service's runs, asks the vault what each run's token
// opens, and keeps what it saw, for GET /runs.
const runs = [];

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    if (url.pathname === "/runs") return Response.json(runs);
    if (url.pathname !== "/kernel/run") return new Response("not found", { status: 404 });
    const auth = request.headers.get("authorization");
    const ask = async (method, path, body) => {
      const resp = await env.VAULT.fetch(new Request(`https://vault${path}`, { method, headers: { authorization: auth, "content-type": "application/json" }, body: body && JSON.stringify(body) }));
      return { status: resp.status, body: await resp.json() };
    };
    const run = await request.json();
    const session = await ask("GET", "/session");
    const keys = await ask("POST", "/keys", { keys: [{ scope: run.scope, app: run.app }] });
    const other = await ask("POST", "/keys", { keys: [{ scope: run.scope, app: "calendar" }] });
    runs.push({ host: url.host, run, session, keys, other });
    return Response.json({ ok: true });
  },
};
