// How a tab of the web client reaches its nodes: through the one client every tab of this browser shares
// (net-worker.js), in a SharedWorker; where there's none, in the tab holding the `one.net` lock, which the others ask
// over a BroadcastChannel. index.html loads this first, so the client's WebAssembly is on its way while the page draws,
// and a user already signed in starts their session at once.

import { Yours } from "../shared/protocol.js";

const started = (self.oneNet ||= {});
const url = started.url || "";

// The page's modules are named by their build (net.js?v=…), so an agent's script that imports "/net.js" gets a second
// copy, which would take the SharedWorker port's messages from the page's and leave its requests unanswered.
if (started.net) {
  const why = "net.js is already running in this page: use self.oneNet.net, not import('/net.js')";
  console.error(why);
  throw new Error(why);
}

const waiting = new Map();
let n = 0;
const tab = crypto.getRandomValues(new Uint32Array(2)).join("-");

function received(m) {
  const w = waiting.get(m.id);
  if (!w) return;
  if (m.error !== undefined) {
    waiting.delete(m.id);
    const err = Object.assign(new Error(m.error), { trace: m.trace, notLetIn: m.notLetIn, signedOut: m.signedOut, code: m.code });
    if (w.body) w.body.error(err);
    else w.reject(err);
  } else if (m.status !== undefined) {
    let body;
    const stream = new ReadableStream({ start: (c) => (body = c), cancel: () => (waiting.delete(m.id), send({ op: "cancel", id: m.id })) });
    w.body = body;
    const headers = new Headers();
    for (let i = 0; i + 1 < m.headers.length; i += 2) headers.append(m.headers[i], m.headers[i + 1]);
    const empty = m.status === 204 || m.status === 304;
    // And which node answered, as its connection proved; and in developer mode, each try's nodes and what the one that
    // answered did (`trace`, JSON).
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
    const err = new Error(`${Yours} restarted. Try again.`);
    if (w.body) w.body.error(err);
    else w.reject(err);
  }
}

let has = false;

/// A new tab leads the others, its client new, with no session: each tab starts its user's again (`starting` said
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

  /// A request to the node, as this browser's device: method, path with its query, headers (`{ name: value }`), body
  /// bytes or none, an AbortSignal, until when the keys it hands may be used (ms), and whether to trace it. → a
  /// Response, its body streamed as it comes, and `node`, the key of the node that answered. Fails with `notLetIn` while
  /// the network doesn't list this device.
  fetch(method, path, headers = {}, body, signal, until = Date.now() + 60_000, trace = false) {
    const bytes = body === undefined ? undefined : body instanceof Uint8Array ? body : new TextEncoder().encode(body);
    const buffer = bytes && bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength);
    const id = `${tab}:${++n}`;
    return new Promise((resolve, reject) => {
      if (signal?.aborted) return reject(signal.reason);
      waiting.set(id, { resolve, reject });
      signal?.addEventListener("abort", () => {
        const w = waiting.get(id);
        if (!w) return;
        waiting.delete(id);
        send({ op: "cancel", id });
        if (w.body) w.body.error(signal.reason);
        else reject(signal.reason);
      });
      send({ op: "fetch", id, method, path, headers: Object.entries(headers).flat(), body: buffer, until, trace }, buffer ? [buffer] : []);
    });
  },

  /// The nodes to ask, in the order things run on them, kept for the next session, and each one's run priority
  /// (`ranks`, a number by key), and the Cloudflare node's key (`wakes`), which the relay never says is on.
  nodes(user, nodes, ranks = {}, wakes = null) {
    if (net.has) send({ op: "nodes", user, nodes, ranks, wakes });
  },

  /// A step of signing in at the network's auth (net-worker.js's `auth`): `{ step, … }` → what it gave.
  async auth(step) {
    if (!net.has) throw new Error("This browser can’t sign in here: update it, and try again.");
    return ask({ op: "auth", ...step });
  },

  /// Asks auth to let this browser in as `user`'s device, signed in with `token`, called `name` (and with someone new's
  /// `invite` secret, its proof).
  async ask(user, token, name, invite) {
    await ask({ op: "ask", user, token, name, invite });
  },

  /// What auth says of this browser's request now, acted on as every client does: "signedOut", "approved", "gone" or
  /// "waiting".
  async waited(user, token) {
    return (await ask({ op: "waited", user, token })).said;
  },

  /// As each start of a client does: this user's own things, settled by this device if they aren't.
  async settle() {
    if (net.has) await ask({ op: "settle" });
  },

  /// What `app`'s `tool` answered last for `input` here, if it only reads: `{ answer, at }`, or null.
  async kept(user, app, tool, input) {
    return net.has ? (await ask({ op: "kept", user, app, tool, input })).kept : null;
  },

  /// What this device kept as `user`'s goes, as it signs out.
  async forget(user) {
    if (net.has) await ask({ op: "forget", user });
  },

  /// This user's app schedules of `app`, renewed by this device as they use it.
  async renew(app) {
    if (net.has) await ask({ op: "renew", app });
  },

  /// This browser's push subscription (`{ endpoint, keys }`), registered with the network's push Worker as this device.
  async push(subscription) {
    await ask({ op: "push", subscription });
  },

  still() {
    return net.has ? ask({ op: "still" }).then((m) => m.device) : Promise.resolve(true);
  },

  reconnect() {
    if (net.has) send({ op: "reconnect" });
  },
};

started.net = net;

// The network back, or a frozen tab running again: a fresh endpoint, rather than one waiting out its relay's backoff.
addEventListener("online", () => net.reconnect());
document.addEventListener("resume", () => net.reconnect());

const user = signedIn();
if (user) net.start(user).catch(() => {});
