// How a tab of the web client reaches its nodes: through the one client every tab of this browser shares
// (net-worker.js), in a SharedWorker; where there's none, in the tab holding the `one.net` lock, which the others ask
// over a BroadcastChannel. index.html loads this first, so the client's WebAssembly is on its way while the page draws,
// and a person already signed in starts their session at once.

const started = self.oneNet || {};
const url = started.url || "";

const waiting = new Map();
let n = 0;
const tab = crypto.getRandomValues(new Uint32Array(2)).join("-");

function received(m) {
  const w = waiting.get(m.id);
  if (!w) return;
  if (m.keysFor !== undefined) {
    Promise.resolve()
      .then(() => w.keys?.(m.keysFor))
      .catch(() => null)
      .then((keys) => send({ op: "keys", id: m.id, keys: keys || null }));
    return;
  }
  if (m.error !== undefined) {
    waiting.delete(m.id);
    const err = Object.assign(new Error(m.error), { trace: m.trace });
    if (w.body) w.body.error(err);
    else w.reject(err);
  } else if (m.status !== undefined) {
    let body;
    const stream = new ReadableStream({ start: (c) => (body = c), cancel: () => (waiting.delete(m.id), send({ op: "cancel", id: m.id })) });
    w.body = body;
    const headers = new Headers();
    for (let i = 0; i + 1 < m.headers.length; i += 2) headers.append(m.headers[i], m.headers[i + 1]);
    const empty = m.status === 204 || m.status === 304;
    // And which node answered, as its connection proved, the one its 428 may be believed from; and in developer mode,
    // the nodes it tried and what the one that answered did (`trace`, JSON).
    w.resolve(Object.assign(new Response(empty ? null : stream, { status: m.status, headers }), { node: m.node, trace: m.trace }));
  } else if (m.part) {
    w.body?.enqueue(m.part);
  } else if (m.done) {
    waiting.delete(m.id);
    try {
      w.body?.close();
    } catch {}
  } else {
    waiting.delete(m.id);
    w.resolve(m);
  }
}

let send = () => {};

function lost() {
  for (const [id, w] of waiting) {
    waiting.delete(id);
    const err = new Error("Your One App network restarted. Try again.");
    if (w.body) w.body.error(err);
    else w.reject(err);
  }
}

let has = false;

/// A new tab leads the others, its client new, with no session: each tab starts its person's again (`starting` said
/// the last leader's), before what it asks next.
function again() {
  const user = starting?.user;
  starting = null;
  if (user) net.start(user).catch(() => {});
}

if (url && typeof SharedWorker === "function") {
  has = true;
  const worker = started.worker || new SharedWorker(url, { type: "module", name: "one" });
  worker.port.onmessage = (e) => received(e.data);
  worker.port.start();
  send = (m, transfer) => worker.port.postMessage(m, transfer || []);
} else if (url && navigator.locks && typeof BroadcastChannel === "function") {
  has = true;
  // The leader answers each tab's messages, its own too; the others' come over the channel, each saying whose it is.
  const channel = new BroadcastChannel("one.net");
  let mine = null;
  let follows = false;
  const queued = [];
  channel.onmessage = (e) => {
    const m = e.data;
    if (m.leader) (lost(), again());
    else if (m.to === tab) received(m.m);
    else if (m.from && mine) mine(m.from, m.m);
  };
  send = (m) => (mine ? mine(tab, m) : follows ? channel.postMessage({ from: tab, m }) : queued.push(m));
  const lead = async () => {
    const { server } = await import(url);
    const s = server(new URL(url, location.href).searchParams.get("client"));
    const tabs = new Map();
    mine = (from, m) => {
      if (!tabs.has(from)) tabs.set(from, s.connect((a) => (from === tab ? received(a) : channel.postMessage({ to: from, m: a }))));
      tabs.get(from)(m);
    };
    // What this tab asked the leader before is lost with it, and its session: started again here.
    if (follows) lost(), again();
    channel.postMessage({ leader: tab });
    for (const m of queued.splice(0)) mine(tab, m);
    await new Promise(() => {});
  };
  navigator.locks.request("one.net", { ifAvailable: true }, async (lock) => {
    if (lock) return lead();
    follows = true;
    for (const m of queued.splice(0)) channel.postMessage({ from: tab, m });
    navigator.locks.request("one.net", lead);
  });
}

function ask(m, transfer) {
  const id = `${tab}:${++n}`;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    send({ ...m, id }, transfer);
  });
}

function signedIn() {
  try {
    const access = JSON.parse(localStorage.getItem("one.signin") || "null")?.access;
    return JSON.parse(atob(access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub || null;
  } catch {
    return null;
  }
}

let starting = null;

export const net = {
  has,

  /// Starts `user`'s session (again, once this browser was just let in): → whether this browser is their device, so
  /// it reaches their nodes itself. `as` is another device of theirs to ask as instead (`{ key, sign }`, a listed
  /// passkey's, while it lets this browser in).
  start(user, again = false, as = undefined) {
    if (!net.has) return Promise.resolve(false);
    if (!starting || starting.user !== user || again) {
      const asked = ask({ op: "start", user, again, as });
      const ready = asked.then((m) => m.device);
      starting = { user, ready, took: asked.then((m) => m.took || null, () => null) };
      ready.catch(() => starting?.ready === ready && (starting = null));
    }
    return starting.ready;
  },

  took() {
    return starting?.took || Promise.resolve(null);
  },

  /// A request to the node: method, path with its query, headers (`{ name: value }`), body bytes or none, an
  /// AbortSignal, and `keys`, which gives the `x-one-keys` for the one node it's sent to (its key, hex → the header's
  /// value or null, or a promise of it), asked once the client picks it. → a Response, its body streamed as it comes,
  /// and `node`, the key of the node that answered.
  fetch(method, path, headers = {}, body, signal, keys) {
    const bytes = body === undefined ? undefined : body instanceof Uint8Array ? body : new TextEncoder().encode(body);
    const buffer = bytes && bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const id = `${tab}:${++n}`;
    return new Promise((resolve, reject) => {
      if (signal?.aborted) return reject(signal.reason);
      waiting.set(id, { resolve, reject, keys });
      signal?.addEventListener("abort", () => {
        const w = waiting.get(id);
        if (!w) return;
        waiting.delete(id);
        send({ op: "cancel", id });
        if (w.body) w.body.error(signal.reason);
        else reject(signal.reason);
      });
      send({ op: "fetch", id, method, path, headers: Object.entries(headers).flat(), body: buffer, keys: !!keys }, buffer ? [buffer] : []);
    });
  },

  /// The nodes to ask, in the order things run on them, kept for the next session, and each one's run priority
  /// (`ranks`, a number by key), and the Cloudflare node's key (`wakes`), which the relay never says is on.
  nodes(user, nodes, ranks = {}, wakes = null) {
    if (net.has) send({ op: "nodes", user, nodes, ranks, wakes });
  },

  /// `user`'s who's who as `global`'s scope database's steps say it, checked here by the client's WebAssembly and
  /// kept: `steps` are those this browser read from the bucket (`Device.bucketSteps`). Throws if one doesn't check, or
  /// they're another network's than the one first kept.
  async global(user, steps) {
    if (!net.has) throw new Error("This device can't check who's who in your One App network.");
    return (await ask({ op: "global", user, steps })).members;
  },

  /// What a new device (`key`, `seal`) signed in with, as its sign-in (`proof`, as sign-in passed it on) shows, checked
  /// by the client at the network's `address`, whose root is `root` (hex) and whose people sign in
  /// with `clients` (its `sign_in_clients` setting, as its entries say): against `accounts`, its person's, for one let in
  /// by it, or with none, for one approved by its code. → the account; throws why it doesn't check out.
  async signedInWith(proof, key, seal, address, { root, clients }, accounts) {
    if (!net.has) throw new Error("This device can't check a sign-in.");
    return (await ask({ op: "signedInWith", proof, key, seal, address, root, clients, accounts })).account;
  },

  still() {
    return net.has ? ask({ op: "still" }).then((m) => m.device) : Promise.resolve(true);
  },

  reconnect() {
    if (net.has) send({ op: "reconnect" });
  },
};

// The network back, or a frozen tab running again: a fresh endpoint, rather than one waiting out its relay's backoff.
addEventListener("online", () => net.reconnect());
document.addEventListener("resume", () => net.reconnect());

const user = signedIn();
if (user) net.start(user).catch(() => {});
