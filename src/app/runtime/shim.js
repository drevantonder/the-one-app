// Every app runs inside this shim, in workerd. The app module (./app.js) is the app's deployed code; this turns it
// into a Worker that answers the node: tools, widgets, hooks and screens. Its databases, one per scope, are the
// node's: a query asks the node, and a call's writes are handed to it when the call returns, as one write. A node on a
// machine asks it over HTTP (`fetch`); the Cloudflare node by RPC (`Call`), handing it what answers for the node. What
// it asks and hands back is src/call.rs's (`ASKS`, `READ_ONLY`, `CODE_HEADER`, written before this when it's built).
import { AsyncLocalStorage } from "node:async_hooks";
import { WorkerEntrypoint } from "cloudflare:workers";

// An app that throws as it starts answers every request with why, and every other app runs on.
let app = {};
let broken;
try {
  app = (await import("./app.js")).default ?? {};
} catch (err) {
  broken = `app.js doesn't start: ${String(err?.stack ?? err).split("\n").slice(0, 2).map((l) => l.trim()).join(" ")}`;
}

// Every request goes out through the node (the Worker's global outbound), carrying the token of the call it's made
// in, so the node knows whose it is: it fills in that call's secrets, and refuses them to other hosts. On the
// Cloudflare node, which has no outbound to give, the call hands each request to the node instead (`out`).
const calls = new AsyncLocalStorage();
const fetchOut = globalThis.fetch;
globalThis.fetch = (input, init) => {
  const req = new Request(input, init);
  const call = calls.getStore();
  if (call?.out) return call.out(req);
  if (call) req.headers.set("x-one-call", call.token);
  return fetchOut(req);
};

const json = (v, status = 200) => new Response(JSON.stringify(v ?? null), { status, headers: { "content-type": "application/json" } });

const tools = () => app.tools || {};

function describe() {
  return {
    name: app.name, title: app.title, icon: app.icon, color: app.color, domain: app.domain, ...(app.tagline ? { tagline: String(app.tagline) } : {}), description: app.description || "", instructions: app.instructions || "",
    ...(app.advanced ? { advanced: true } : {}), ...(app.needs ? { needs: String(app.needs) } : {}),
    ...(app.vault && Object.keys(app.vault).length ? { vault: app.vault } : {}),
    // Features still being built, each off until an admin switches it on in Settings › Labs (src/labs.rs). A tool
    // with `lab` is there only while its lab is on.
    ...(app.labs && Object.keys(app.labs).length ? { labs: Object.fromEntries(Object.entries(app.labs).map(([name, l]) => [name, { title: String(l.title || name), description: String(l.description || "") }])) } : {}),
    tools: Object.entries(tools()).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {}, ...(o.reads ? { reads: true } : {}), ...(o.confirm ? { confirm: true } : {}), ...(o.accounts ? { accounts: true } : {}), ...(o.words ? { words: o.words.map(String) } : {}), ...(o.lab ? { lab: String(o.lab) } : {}) })),
    widgets: Object.entries(app.widgets || {}).map(([name, w]) => {
      const size = w.size || "medium";
      const options = Object.entries(w.options || {}).map(([name, o]) => ({ name, title: o.title || name, default: o.default ?? null }));
      return { name, title: w.title || name, description: w.description || "", size, sizes: w.sizes?.length ? w.sizes : [size], ...(options.length ? { options } : {}) };
    }),
    badge: typeof app.badge === "function",
    hooks: Object.keys(app.hooks || {}),
    screens: Object.entries(app.screens || {}).map(([name, s]) => ({ name, title: s.title || name })),
    uses: { apps: app.uses?.apps || [], services: app.uses?.services || [] },
    // Its service's wrapper: the hosts it may reach ("*" for any) and whether it runs as the node's person; and the
    // platforms it's built for, the nodes the App Store offers to run it.
    ...(app.service ? { service: { network: [].concat(app.service.network || []).map(String), person: app.service.person === true, ...(app.service.platforms ? { platforms: [].concat(app.service.platforms).map(String) } : {}) } } : {}),
    schedules: Object.entries(app.schedules || {}).map(([name, s]) => ({ name, ...s })),
    // Each kind of scope's schema (user, group, global), as the changes that make it, oldest first.
    schema: Object.fromEntries(Object.entries(app.schema || {}).map(([kind, sql]) => [kind, [].concat(sql)])),
    // Its own types of object, as the core's (src/permissions): each one's verbs, and its roles, each a list of them.
    // Who holds a role on one is a row of its database's `relations` (subject, relation, object, until, as the core's),
    // which `may` reads. A type the core has stays the core's.
    ...(app.types && Object.keys(app.types).length ? { types: Object.fromEntries(Object.entries(app.types).map(([type, t]) => [type, {
      permissions: [].concat(t.permissions || []).map(String),
      roles: Object.fromEntries(Object.entries(t.roles || {}).map(([role, verbs]) => [role, [].concat(verbs).map(String)])),
    }])) } : {}),
  };
}

/// The node's answer to an ask, as a Response: its error, with the code the node gave it, thrown.
async function answered(r) {
  if (!r.ok) throw Object.assign(new Error(await r.text()), r.headers.has(CODE_HEADER) ? { code: r.headers.get(CODE_HEADER) } : {});
  return r;
}

/// How the shim asks the node `name` (an ask of `ASKS`) for a call, with `send(name, body)` the way its node answers:
/// the same on a machine and on the Cloudflare node.
const asking = (send, body) => async (name, data) => {
  const ask = ASKS[name];
  if (!ask) throw new Error(`the node has no ask ${name}`);
  if (ask.writes && body.read_only) throw new Error(READ_ONLY);
  return answered(await send(name, data ?? null));
};

/// After a call returned: waits for the work it left running (ctx.waitUntil), hands the node what that work wrote,
/// and says it's done.
async function finish(ask, later, appends) {
  let error = null;
  for (let i = 0; i < later.length; i++) {
    try {
      await later[i];
    } catch (err) {
      error ||= String(err?.message ?? err);
      console.warn(`${app.name}: work after a call failed: ${error}`);
    }
  }
  try {
    if (appends.length) await ask("appends", JSON.stringify({ appends: appends.splice(0) }));
  } catch (err) {
    error ||= String(err?.message ?? err);
  }
  await ask("done", JSON.stringify({ error })).catch(() => {});
}

/// A widget's options' defaults, as its render sees them when a person chose none.
function optionDefaults(widget) {
  return Object.fromEntries(Object.entries(widget?.options || {}).map(([name, o]) => [name, o.default ?? null]));
}

/// A widget's options for a shell to offer: each one's choices (`{ value, title }`), worked out from the data where
/// the app says how; one with none and a true or false default is a switch.
async function widgetOptions(widget, ctx) {
  const out = [];
  for (const [name, o] of Object.entries(widget?.options || {})) {
    const raw = typeof o.choices === "function" ? await o.choices(ctx) : o.choices;
    const choices = Array.isArray(raw) ? raw.map((c) => (typeof c === "object" && c ? { value: c.value ?? null, title: String(c.title ?? c.value) } : { value: c, title: String(c) })) : undefined;
    out.push({ name, title: o.title || name, default: o.default ?? null, ...(choices ? { choices } : {}) });
  }
  return out;
}

/// What a call, widget or hook gets: the caller, and a handle on each scope the node lets it reach.
function context(ask, body, appends, later) {
  const writing = () => {
    if (body.read_only) throw new Error(READ_ONLY);
  };
  const post = async (name, data) => (await ask(name, JSON.stringify(data))).json();
  const handle = (s) => ({
    id: s.id, name: s.name, role: s.role,
    /// Reads this scope's database: the rows, each an object by column. It sees what the call writes only once the
    /// call has returned.
    query: async (q, params = []) => (await post("query", { scope: s.id, sql: q, params })).ok,
    /// Whether the caller may do `permission` (a verb) to `object` (`<type>:<id>`), as the core's rows and this scope's
    /// `relations`, by the app's own `types`, say: `may("push", "repo:" + id)`, `may("use", "group:" + id)`. Given a
    /// list of [permission, object], whether to each, in order, asked at once: `may([["push", a], ["manage", a]])`.
    may: async (permission, object) => {
      const asks = Array.isArray(permission) ? permission : [[permission, object]];
      const ok = (await post("may", { scope: s.id, asks })).ok;
      return Array.isArray(permission) ? ok : ok[0] === true;
    },
    /// Writes this scope's database once the call returns, with every other statement it ran, as one transaction;
    /// `blobs` are blobs the scope keeps from now on.
    exec: (q, params = [], blobs = []) => {
      writing();
      appends.push({ kind: "exec", scope: s.id, sql: String(q), params, blobs });
    },
    /// Says the write holds only while a query's first value is true, as it runs: when it isn't (another call wrote
    /// first), the call fails with `message` and writes nothing.
    check: (q, params = [], message = "the data changed meanwhile: try again") => {
      writing();
      appends.push({ kind: "exec", scope: s.id, sql: String(q), params, check: String(message) });
    },
    /// Says this scope no longer needs a blob it kept: once no scope does, devices delete it.
    forget: (hash) => {
      writing();
      appends.push({ kind: "forget", blob: String(hash), scope: s.id });
    },
  });
  const scopes = body.scopes.map(handle);
  // `ctx.apps.<name>.<tool>(input, { device })`, for exactly the apps the app says it uses, and `ctx.services.<name>`
  // for its own service, which only it calls (the node checks both); `device` (a name) runs it there, like a service
  // on the machine that has what it needs.
  const uses = { apps: app.uses?.apps || [], services: app.uses?.services || [] };
  const used = (kind) => new Proxy({}, {
    get(_, name) {
      if (typeof name !== "string" || name === "then") return undefined;
      if (kind === "services" && name !== app.name) throw new Error(`${app.name} can't call the ${name} service: a service is only its own app's, so call the ${name} app's tools with ctx.apps`);
      if (!uses[kind].includes(name)) throw new Error(`${app.name} doesn't use ${name}: add it to uses.${kind}`);
      return new Proxy({}, {
        get(_, tool) {
          if (typeof tool !== "string" || tool === "then") return undefined;
          return async (input = {}, { device, ...other } = {}) => {
            // An option it doesn't know (one renamed, say) fails, rather than running the call somewhere else.
            const [unknown] = Object.keys(other);
            if (unknown) throw new Error(`${unknown} isn't an option of ctx.${kind} calls: give device`);
            return (await post("call", { app: name, service: kind === "services", [kind === "services" ? "function" : "tool"]: tool, input, device })).ok;
          };
        },
      });
    },
  });
  const find = (id) => scopes.find((s) => s.id === id) || null;
  return {
    me: body.me,
    /// Each of the app's labs, by name: true while an admin has it switched on in Settings › Labs.
    labs: Object.fromEntries(Object.keys(app.labs || {}).map((name) => [name, body.labs?.[name] === true])),
    /// The app whose ctx.apps made this call, if one did.
    from: body.from ?? null,
    /// The port this node's local API answers on at 127.0.0.1, the custom build's (config.toml) unless it was started
    /// on another: where a sign-in sends the person's browser back. Null on a node with none (the Cloudflare node).
    port: body.port ?? null,
    /// The network's address (`https://…`, no last `/`), or null in a network with no address yet: a sign-in at
    /// another site sends the person back to `<address>/back?state=<app>/<screen>&…`, which opens that screen of this
    /// app with `params.back`, that whole address.
    address: body.address ?? null,
    /// Everyone's and every group's handle in the network, what apps call them by in addresses and lists:
    /// [{ handle, id: "user:<id>" | "group:<id>", name }], a removed person's with removed: true, a group's with its
    /// members: { <user id>: "admin" | "member" }. `me.handle` is the caller's.
    handles: body.handles,
    /// Says what this call spent on AI, { model, usd, input, output }: it shows in the insights of the app that called
    /// it (ctx.from), or else of this one.
    cost: (spent) => {
      if (body.read_only) return;
      appends.push({ kind: "cost", model: String(spent?.model ?? "?"), usd: Number(spent?.usd) || 0, input: Math.round(Number(spent?.input) || 0), output: Math.round(Number(spent?.output) || 0) });
    },
    /// Keeps work going after the call returns (an AI read, a copy to Drive), up to 10 minutes: its requests and
    /// calls still go out as this call's, and what it writes is written when it's done.
    waitUntil: (promise) => {
      writing();
      later.push(Promise.resolve(promise));
    },
    now: () => body.now,
    user: scopes.find((s) => s.id.startsWith("user:")) || null,
    global: find("global"),
    groups: scopes.filter((s) => s.id.startsWith("group:")),
    group: (id) => find(id.startsWith("group:") ? id : `group:${id}`),
    apps: used("apps"),
    services: used("services"),
    blob: {
      put: async (bytes) => (await (await ask("blob.put", bytes)).json()).blob,
      get: async (hash) => new Uint8Array(await (await ask("blob.get", JSON.stringify({ hash }))).arrayBuffer()),
    },
    /// The Vault, as this app may use it: the items it's switched on for (a secret field as { secret: true }, never its
    /// value), and making, changing and letting go of items. A field goes into a request as {{<item id>:<field>}}.
    vault: {
      items: async () => (await ask("vault")).json(),
      /// Makes an item switched on for this app ({ scope?, title, type?, domain?, fields: { name: { value, secret? } },
      /// hosts }), in the caller's scope by default; with `same`, a plain field's name, changes the item of that type
      /// whose field has that value instead (every secret field given anew). Gives its id.
      async create({ scope = "user", same, ...item }) {
        writing();
        const at = scope === "user" ? scopes.find((s) => s.id.startsWith("user:"))?.id : scope;
        if (!at) throw new Error("this device doesn't keep your data: use one of your own devices");
        let id = null;
        if (same) id = (await post("vault.match", { scope: at, type: item.type ?? "", field: same, value: item.fields?.[same]?.value })).id;
        id ||= [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
        appends.push({ kind: "vault", id, scope: at, item, same: same ?? null });
        return id;
      },
      /// Changes an item it's switched on for: { title?, type?, domain?, fields?, hosts? }, its hosts only with every
      /// secret field given anew.
      update: (id, changes) => {
        writing();
        appends.push({ kind: "vault", id: String(id), item: changes ?? {} });
      },
      /// Switches this app off for an item (signing out an account); the item goes once no app uses it.
      release: (id) => {
        writing();
        appends.push({ kind: "vault", id: String(id), release: true });
      },
    },
    /// Pushes a notification to each of the caller's devices that gets them, { title, body?, screen?, params? };
    /// tapping it opens this app's screen. → { sent, devices, why? }: how many it went to, and why none, if none.
    notify: async (push) => (await post("notify", push)).ok,
    /// Every app in the network, as `GET /v1/apps` gives them to the caller.
    catalog: async () => (await ask("catalog")).json(),
    /// An app's code as deployed (the release's, for a built-in app no one has added): `{ files: [{ path, size }] }`, or
    /// with `path`, `{ path, text }` for that file.
    code: async (app, path) => post("code", { app, path }),
    /// The schedules the caller may switch, of the apps they may use, that are on and failed their last run, each as
    /// `catalog` gives an app's `schedules`, with its `app`.
    failing: async () => (await ask("failing")).json(),
    /// The network's devices, as `GET /v1/state` gives them to the caller: online or not, and each one's load.
    devices: async () => (await ask("devices")).json(),
    /// Run priority (src/rank.rs): `{ order, admin }`, every device and the Cloudflare node (key "cloudflare") in the
    /// order things go to them, each with its `kind` and its `priority` (a number from 1, or "last"), and whether the
    /// caller is an admin. Given a device (its name or key, or "cloudflare") and a priority, gives it that one first,
    /// for an admin.
    order: async (device, priority) => {
      if (device !== undefined) await post("priority", { device, priority });
      return (await ask("order")).json();
    },
    /// Adds (or with `added: false`, removes) an app in a scope, with the caller's own rights there.
    add: (name, scope, added = true, node) => {
      writing();
      appends.push({ kind: "add", app: name, scope, added, ...(node ? { node } : {}) });
    },
    /// Deletes an app ("deleted": to Recently Deleted), restores it ("restored") or erases it for good ("erased"),
    /// for someone acting for its owner.
    retire: (name, how) => {
      writing();
      appends.push({ kind: "retire", app: name, how });
    },
    /// Sets where an app's code is kept (a web address or `<cli>://<repo>`), or with "" clears it, for someone acting
    /// for its owner.
    source: (name, link) => {
      writing();
      appends.push({ kind: "source", app: name, link: String(link ?? "") });
    },
    /// Runs an app's service on a device (by name) or stops it there, for the app added in `scope` (by default where the
    /// caller sees it added): for the device's person, or to stop it, for who may add apps where it ran.
    service: (name, device, on = false, scope) => {
      writing();
      appends.push({ kind: "service", name, device, on, ...(scope ? { scope } : {}) });
    },
    /// Switches one of an app's schedules on or off in a scope, with the caller's own rights there.
    schedule: (name, schedule, scope, on = true) => {
      writing();
      appends.push({ kind: "switch", schedule, app: name, scope, on });
    },
  };
}

/// Answers one of the node's requests: `describe`, `screen` (with `name`), or a call, widget, its options, a badge or
/// a hook, for `body`; `send` is how it asks the node, `waitUntil` how work outlives the reply, and `out`, if given,
/// sends the app's requests out. → what the reply says.
async function handle(what, name, body, send, waitUntil, out) {
  if (broken) return { error: broken };
  if (what === "describe") return { ok: describe() };
  if (what === "screen") return { ok: app.screens?.[name]?.html ?? null };
  try {
    const appends = [];
    const later = [];
    const ask = asking(send, body);
    const ctx = context(ask, body, appends, later);
    let run;
    if (what === "call") {
      const tool = tools()[body.tool];
      if (!tool) throw new Error(`${app.name} has no tool ${body.tool}`);
      run = () => tool.run(ctx, body.input ?? {});
    } else if (what === "widget") {
      const widget = app.widgets[body.name];
      ctx.size = body.size || widget.size || "medium";
      ctx.options = { ...optionDefaults(widget), ...(body.options || {}) };
      run = () => widget.render(ctx);
    } else if (what === "options") {
      run = () => widgetOptions(app.widgets[body.name], ctx);
    } else if (what === "badge") {
      run = () => app.badge(ctx);
    } else if (what === "hook") {
      const hook = app.hooks?.[body.name];
      if (!hook) throw new Error(`${app.name} has no hook ${body.name}`);
      run = () => hook(ctx, body.input);
    } else {
      throw new Error(`no such request ${what}`);
    }
    const call = { token: body.token, out };
    const result = await calls.run(call, run);
    const keep = what === "call" && later.length > 0;
    const reply = { ok: result, appends: what === "call" ? appends.splice(0) : [], later: keep };
    if (keep) waitUntil(calls.run(call, () => finish(ask, later, appends)));
    return reply;
  } catch (err) {
    // An error with a `code` (`not_signed_in`, `refused`…: src/protocol.rs) says what kind it is, which callers act on.
    return { error: String(err?.message ?? err), ...(typeof err?.code === "string" ? { code: err.code } : {}) };
  }
}

export default {
  async fetch(req, env, execution) {
    const url = new URL(req.url);
    const what = url.pathname.split("/").pop();
    const body = ["describe", "screen"].includes(what) ? {} : await req.json();
    const send = (name, data) => fetchOut(new Request(`http://one.internal/${name}`, { method: "POST", body: data, headers: { "x-one-call": body.token } }));
    const reply = await handle(what, url.searchParams.get("name"), body, send, (p) => execution.waitUntil(p));
    return json(reply, reply.error === undefined ? 200 : 400);
  },
};

/// The Cloudflare node runs an app through this: `node.ask(name, body)` answers what the node does for a call with a
/// Response, as a machine's node answers it over HTTP.
export class Call extends WorkerEntrypoint {
  async run(what, name, body, node) {
    const send = (name, data) => node.ask(name, data);
    return handle(what, name, body, send, (p) => this.ctx.waitUntil(p), (req) => sent(asking(send, body), req));
  }
}

const b64u = {
  encode: (bytes) => {
    let text = "";
    for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
    return btoa(text).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  },
  decode: (text) => Uint8Array.from(atob(text.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)),
};

/// An app's request, made by the Cloudflare node through the vault, which fills in the Vault's items.
async function sent(ask, req) {
  const body = ["GET", "HEAD"].includes(req.method) ? undefined : b64u.encode(new Uint8Array(await req.arrayBuffer()));
  const r = await ask("fetch", JSON.stringify({ url: req.url, method: req.method, headers: Object.fromEntries(req.headers), body64: body }));
  const got = await r.json();
  const headers = new Headers(got.headers);
  for (const h of ["content-encoding", "content-length", "transfer-encoding"]) headers.delete(h);
  return new Response([101, 204, 205, 304].includes(got.status) ? null : b64u.decode(got.body64), { status: got.status, headers });
}
