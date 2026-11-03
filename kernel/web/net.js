// How a tab of the web client reaches its nodes: through the one client every tab of this browser shares
// (net-worker.js), in a SharedWorker; where there's none, in the tab holding the `one.net` lock, which the others ask
// over a BroadcastChannel. index.html loads this first, so the client's WebAssembly is on its way while the page draws,
// and a person already signed in starts their session at once.

/// The client's build (`/client/<version>/`), which the kernel's build writes into index.html: none, and there's no
/// client, so the web client asks through the address instead.
const version = document.querySelector('meta[name="one-client"]')?.content || "";

/// Each request's id, and what's waiting on it: `{ answered, body }`.
const waiting = new Map();
let n = 0;
const tab = crypto.getRandomValues(new Uint32Array(2)).join("-");

/// What a message from the client means for the request it answers.
function received(m) {
  const w = waiting.get(m.id);
  if (!w) return;
  if (m.error !== undefined) {
    waiting.delete(m.id);
    const err = new Error(m.error);
    if (w.body) w.body.error(err);
    else w.reject(err);
  } else if (m.status !== undefined) {
    let body;
    const stream = new ReadableStream({ start: (c) => (body = c), cancel: () => (waiting.delete(m.id), send({ op: "cancel", id: m.id })) });
    w.body = body;
    const headers = new Headers();
    for (let i = 0; i + 1 < m.headers.length; i += 2) headers.append(m.headers[i], m.headers[i + 1]);
    const empty = m.status === 204 || m.status === 304;
    // And which node answered, as its connection proved, the one its 428 may be believed from.
    w.resolve(Object.assign(new Response(empty ? null : stream, { status: m.status, headers }), { node: m.node }));
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

/// Sends the client a message: through the SharedWorker's port, to this tab's own client, or to the tab that has it.
let send = () => {};

/// Fails what's waiting, for a tab that lost the one it asked.
function lost() {
  for (const [id, w] of waiting) {
    waiting.delete(id);
    const err = new Error("Your One App's network restarted. Try again.");
    if (w.body) w.body.error(err);
    else w.reject(err);
  }
}

/// Whether this browser has a client.
let has = false;

if (version && typeof SharedWorker === "function") {
  has = true;
  const worker = new SharedWorker(`/net-worker.js?client=${encodeURIComponent(version)}`, { type: "module", name: "one" });
  worker.port.onmessage = (e) => received(e.data);
  worker.port.start();
  send = (m, transfer) => worker.port.postMessage(m, transfer || []);
} else if (version && navigator.locks && typeof BroadcastChannel === "function") {
  has = true;
  // The leader answers each tab's messages, its own too; the others' come over the channel, each saying whose it is.
  const channel = new BroadcastChannel("one.net");
  let mine = null;
  let follows = false;
  const queued = [];
  channel.onmessage = (e) => {
    const m = e.data;
    if (m.leader) lost();
    else if (m.to === tab) received(m.m);
    else if (m.from && mine) mine(m.from, m.m);
  };
  send = (m) => (mine ? mine(tab, m) : follows ? channel.postMessage({ from: tab, m }) : queued.push(m));
  const lead = async () => {
    const { server } = await import(`/net-worker.js?client=${encodeURIComponent(version)}`);
    const s = server(version);
    const tabs = new Map();
    // What this tab asked the leader before is lost with it.
    if (follows) lost();
    mine = (from, m) => {
      if (!tabs.has(from)) tabs.set(from, s.connect((a) => (from === tab ? received(a) : channel.postMessage({ to: from, m: a }))));
      tabs.get(from)(m);
    };
    channel.postMessage({ leader: tab });
    for (const m of queued.splice(0)) mine(tab, m);
    // Held while this tab lives.
    await new Promise(() => {});
  };
  navigator.locks.request("one.net", { ifAvailable: true }, async (lock) => {
    if (lock) return lead();
    follows = true;
    for (const m of queued.splice(0)) channel.postMessage({ from: tab, m });
    navigator.locks.request("one.net", lead);
  });
}

/// Asks the client: → its answer, or for a fetch, the response.
function ask(m, transfer) {
  const id = `${tab}:${++n}`;
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject });
    send({ ...m, id }, transfer);
  });
}

/// The person signed in here, from their access token, for their session to start before the page asks for it.
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
  /// Whether this browser has a client at all.
  has,

  /// Starts `user`'s session (again, once this browser was just let in): → whether this browser is their device, so
  /// it reaches their nodes itself.
  start(user, again = false) {
    if (!net.has) return Promise.resolve(false);
    if (!starting || starting.user !== user || again) {
      const ready = ask({ op: "start", user, again }).then((m) => m.device);
      starting = { user, ready };
      ready.catch(() => starting?.ready === ready && (starting = null));
    }
    return starting.ready;
  },

  /// A request to the node: method, path with its query, headers (`{ name: value }`), body bytes or none, and an
  /// AbortSignal. → a Response, its body streamed as it comes, and `node`, the key of the node that answered.
  fetch(method, path, headers = {}, body, signal) {
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
      send({ op: "fetch", id, method, path, headers: Object.entries(headers).flat(), body: buffer }, buffer ? [buffer] : []);
    });
  },

  /// The nodes to ask, in the order things run on them, kept for the next session.
  nodes(user, nodes) {
    if (net.has) send({ op: "nodes", user, nodes });
  },

  /// A fresh endpoint, after the network came back.
  reconnect() {
    if (net.has) send({ op: "reconnect" });
  },
};

// The network back, or a frozen tab running again: a fresh endpoint, rather than one waiting out its relay's backoff.
addEventListener("online", () => net.reconnect());
document.addEventListener("resume", () => net.reconnect());

// Someone signed in: their session starts now, while the page loads.
const user = signedIn();
if (user) net.start(user).catch(() => {});
