// Every app runs inside this shim, in workerd. The app module (./app.js) is the app's deployed code; this turns it
// into a Worker that answers the node: tools, widgets, hooks and screens. Its databases, one per scope, are the
// node's: a query asks the node, and a call's writes are handed to it when the call returns, as one write. A node on a
// machine asks it over HTTP (`fetch`); the Cloudflare node by RPC (`Call`), handing it what answers for the node.
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
    ...(app.connector ? { connector: true } : {}), ...(app.advanced ? { advanced: true } : {}), ...(app.needs ? { needs: String(app.needs) } : {}),
    ...(app.vault && Object.keys(app.vault).length ? { vault: app.vault } : {}),
    // Features still being built, each off until an admin switches it on in Settings › Labs (src/labs.rs). A tool
    // with `lab` is there only while its lab is on.
    ...(app.labs && Object.keys(app.labs).length ? { labs: Object.fromEntries(Object.entries(app.labs).map(([name, l]) => [name, { title: String(l.title || name), description: String(l.description || "") }])) } : {}),
    // The wire calls tools `ops`, and a call's tool `op` (tests/contracts.rs), which older devices read.
    ops: Object.entries(tools()).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {}, ...(o.reads ? { reads: true } : {}), ...(o.confirm ? { confirm: true } : {}), ...(o.words ? { words: o.words.map(String) } : {}), ...(o.lab ? { lab: String(o.lab) } : {}) })),
    widgets: Object.entries(app.widgets || {}).map(([name, w]) => {
      const size = w.size || "medium";
      const options = Object.entries(w.options || {}).map(([name, o]) => ({ name, title: o.title || name, default: o.default ?? null }));
      return { name, title: w.title || name, description: w.description || "", size, sizes: w.sizes?.length ? w.sizes : [size], ...(options.length ? { options } : {}) };
    }),
    badge: typeof app.badge === "function",
    hooks: Object.keys(app.hooks || {}),
    screens: Object.entries(app.screens || {}).map(([name, s]) => ({ name, title: s.title || name })),
    uses: { apps: app.uses?.apps || [], services: app.uses?.services || [] },
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

/// Asks the node something for the call with `token`: other apps' and services' operations, blobs, the install's apps.
async function askNode(token, path, init = {}) {
  const r = await fetchOut(new Request(`http://one.internal/${path}`, { ...init, headers: { "x-one-call": token } }));
  if (!r.ok) throw new Error(await r.text());
  return r;
}

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
    if (appends.length) await ask("appends", { method: "POST", body: JSON.stringify({ appends: appends.splice(0) }) });
  } catch (err) {
    error ||= String(err?.message ?? err);
  }
  await ask("done", { method: "POST", body: JSON.stringify({ error }) }).catch(() => {});
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
  const handle = (s) => ({
    id: s.id, name: s.name, role: s.role,
    /// Reads this scope's database: the rows, each an object by column. It sees what the call writes only once the
    /// call has returned.
    query: async (q, params = []) => (await (await ask("query", { method: "POST", body: JSON.stringify({ scope: s.id, sql: q, params }) })).json()).ok,
    /// Whether the caller may do `permission` (a verb) to `object` (`<type>:<id>`), as the core's rows and this scope's
    /// `relations`, by the app's own `types`, say: `may("push", "repo:" + id)`, `may("use", "group:" + id)`. Given a
    /// list of [permission, object], whether to each, in order, asked at once: `may([["push", a], ["manage", a]])`.
    may: async (permission, object) => {
      const asks = Array.isArray(permission) ? permission : [[permission, object]];
      const ok = (await (await ask("may", { method: "POST", body: JSON.stringify({ scope: s.id, asks }) })).json()).ok;
      return Array.isArray(permission) ? ok : ok[0] === true;
    },
    /// Writes this scope's database once the call returns, with every other statement it ran, as one transaction;
    /// `blobs` are blobs the scope keeps from now on.
    exec: (q, params = [], blobs = []) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ scope: s.id, exec: String(q), params, blobs });
    },
    /// Says the write holds only while a query's first value is true, as it runs: when it isn't (another call wrote
    /// first), the call fails with `message` and writes nothing.
    check: (q, params = [], message = "the data changed meanwhile: try again") => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ scope: s.id, exec: String(q), params, check: String(message) });
    },
    /// Says this scope no longer needs a blob it kept: once no scope does, devices delete it.
    forget: (hash) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ forget: String(hash), scope: s.id });
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
        get(_, op) {
          if (typeof op !== "string" || op === "then") return undefined;
          return async (input = {}, { device, ...other } = {}) => {
            // An option it doesn't know (one renamed, say) fails, rather than running the call somewhere else.
            const [unknown] = Object.keys(other);
            if (unknown) throw new Error(`${unknown} isn't an option of ctx.${kind} calls: give device`);
            return (await (await ask("call", { method: "POST", body: JSON.stringify({ app: name, service: kind === "services", op, input, device }) })).json()).ok;
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
    /// The install's address (`https://…`, no last `/`), or null on a node from before it was given: a sign-in at
    /// another site sends the person back to `<address>/back?state=<app>/<screen>&…`, which opens that screen of this
    /// app with `params.back`, that whole address.
    address: body.address ?? null,
    /// Everyone's and every group's handle in the install, what apps call them by in addresses and lists:
    /// [{ handle, id: "user:<id>" | "group:<id>", name }], a removed person's with removed: true, a group's with its
    /// members: { <user id>: "admin" | "member" }; null on a node from before handles. `me.handle` is the caller's.
    handles: body.handles ?? null,
    /// Says what this call spent on AI, { model, usd, input, output }: it shows in the insights of the app that called
    /// it (ctx.from), or else of this one.
    cost: (spent) => {
      if (body.read_only) return;
      appends.push({ cost: { model: String(spent?.model ?? "?"), usd: Number(spent?.usd) || 0, input: Number(spent?.input) || 0, output: Number(spent?.output) || 0 } });
    },
    /// Keeps work going after the call returns (an AI read, a copy to Drive), up to 10 minutes: its requests and
    /// calls still go out as this call's, and what it writes is written when it's done.
    waitUntil: (promise) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
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
      put: async (bytes) => (await (await ask("blob", { method: "POST", body: bytes })).json()).blob,
      get: async (hash) => new Uint8Array(await (await ask(`blob/${hash}`)).arrayBuffer()),
    },
    /// The Vault, as this app may use it: the items it's switched on for (a secret field as { secret: true }, never its
    /// value), and making, changing and letting go of items. A field goes into a request as {{<item id>:<field>}}.
    vault: {
      items: async () => (await ask("vault")).json(),
      /// Makes an item switched on for this app ({ scope?, title, type?, domain?, fields: { name: { value, secret? } },
      /// hosts }), in the caller's scope by default; with `same`, a plain field's name, changes the item of that type
      /// whose field has that value instead (every secret field given anew). Gives its id.
      async create({ scope = "user", same, ...item }) {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        const at = scope === "user" ? scopes.find((s) => s.id.startsWith("user:"))?.id : scope;
        if (!at) throw new Error("this device doesn't keep your data: use one of your own devices");
        let id = null;
        if (same) id = (await (await ask("vault/match", { method: "POST", body: JSON.stringify({ scope: at, type: item.type ?? "", field: same, value: item.fields?.[same]?.value }) })).json()).id;
        id ||= [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
        appends.push({ vault: id, scope: at, item, same: same ?? null });
        return id;
      },
      /// Changes an item it's switched on for: { title?, type?, domain?, fields?, hosts? }, its hosts only with every
      /// secret field given anew.
      update: (id, changes) => {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        appends.push({ vault: String(id), item: changes ?? {} });
      },
      /// Switches this app off for an item (signing out an account); the item goes once no app uses it.
      release: (id) => {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        appends.push({ vault: String(id), release: true });
      },
    },
    /// Pushes a notification to each of the caller's devices that gets them, { title, body?, screen?, params? };
    /// tapping it opens this app's screen. → { sent, devices, why? }: how many it went to, and why none, if none.
    notify: async (push) => {
      if (body.read_only) throw new Error("this call only reads: it can't push");
      return (await (await ask("notify", { method: "POST", body: JSON.stringify(push) })).json()).ok;
    },
    /// Every app in the install, as `GET /v1/apps` gives them to the caller.
    catalog: async () => (await ask("catalog")).json(),
    /// The schedules the caller may switch, of the apps they may use, that are on and failed their last run, each as
    /// `catalog` gives an app's `schedules`, with its `app`.
    failing: async () => (await ask("failing")).json(),
    /// The install's devices, as `GET /v1/state` gives them to the caller: online or not, and each one's load.
    devices: async () => (await ask("devices")).json(),
    /// Run priority (src/rank.rs): `{ order, admin }`, every device and the Cloudflare node (key "cloudflare") in the
    /// order things go to them, each with its `kind` and its `priority` (a number from 1, or "last"), and whether the
    /// caller is an admin. Given a device (its name or key, or "cloudflare") and a priority, gives it that one first,
    /// for an admin.
    order: async (device, priority) => {
      if (device !== undefined) {
        if (body.read_only) throw new Error("this call only reads: it can't change run priority");
        await ask("priority", { method: "POST", body: JSON.stringify({ device, priority }) });
      }
      return (await ask("order")).json();
    },
    /// Installs (or with `installed: false`, uninstalls) an app in a scope, with the caller's own rights there.
    install: (name, scope, installed = true) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ install: name, scope, installed });
    },
    /// Deletes an app ("deleted": to Recently Deleted), restores it ("restored") or erases it for good ("erased"),
    /// for someone acting for its owner.
    retire: (name, how) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ retire: name, how });
    },
    /// Sets where an app's code is kept (a web address or `<cli>://<repo>`), or with "" clears it, for someone acting
    /// for its owner.
    source: (name, link) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ app_source: name, link: String(link ?? "") });
    },
    /// Switches a service on or off on a device (by name), for the device's person.
    service: (name, device, on = false) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ service: name, device, on });
    },
    /// Switches one of an app's schedules on or off in a scope, with the caller's own rights there.
    schedule: (name, schedule, scope, on = true) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ switch: schedule, app: name, scope, on });
    },
  };
}

/// Answers one of the node's requests: `describe`, `screen` (with `name`), or a call, widget, its options, a badge or
/// a hook, for `body`; `ask` is how it asks the node, `waitUntil` how work outlives the reply, and `out`, if given,
/// sends the app's requests out. → what the reply says.
async function handle(what, name, body, ask, waitUntil, out) {
  if (broken) return { error: broken };
  if (what === "describe") return { ok: describe() };
  if (what === "screen") return { ok: app.screens?.[name]?.html ?? null };
  try {
    const appends = [];
    const later = [];
    const ctx = context(ask, body, appends, later);
    let run;
    if (what === "call") {
      const tool = tools()[body.op];
      if (!tool) throw new Error(`${app.name} has no tool ${body.op}`);
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
    return { error: String(err?.message ?? err) };
  }
}

export default {
  async fetch(req, env, execution) {
    const url = new URL(req.url);
    const what = url.pathname.split("/").pop();
    const body = ["describe", "screen"].includes(what) ? {} : await req.json();
    const reply = await handle(what, url.searchParams.get("name"), body, (path, init) => askNode(body.token, path, init), (p) => execution.waitUntil(p));
    return json(reply, reply.error === undefined ? 200 : 400);
  },
};

/// The Cloudflare node runs an app through this: `node.ask(path, init)` answers what the node does for a call, as
/// `askNode`'s requests are answered on a machine, with a Response.
export class Call extends WorkerEntrypoint {
  async run(what, name, body, node) {
    const ask = async (path, init = {}) => {
      const r = await node.ask(path, init.method || "GET", init.body ?? null);
      if (!r.ok) throw new Error(await r.text());
      return r;
    };
    return handle(what, name, body, ask, (p) => this.ctx.waitUntil(p), (req) => sent(ask, req));
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
  const r = await ask("fetch", { method: "POST", body: JSON.stringify({ url: req.url, method: req.method, headers: Object.fromEntries(req.headers), body64: body }) });
  const got = await r.json();
  const headers = new Headers(got.headers);
  for (const h of ["content-encoding", "content-length", "transfer-encoding"]) headers.delete(h);
  return new Response([101, 204, 205, 304].includes(got.status) ? null : b64u.decode(got.body64), { status: got.status, headers });
}
