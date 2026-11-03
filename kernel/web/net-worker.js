// The browser's one way to its nodes: iroh (client/, as WebAssembly), in one SharedWorker every tab of the web client
// shares, so a browser keeps one connection however many tabs and screens ask; where there's no SharedWorker, in the
// one tab that holds the `one.net` lock, for the others (net.js).
//
// Its endpoint is a key of the session's, made here, which the browser's device key vouches for (`certify`): that key
// can't be read out of WebCrypto, and iroh signs with raw bytes. The relay and nodes take the endpoint as the device.
//
// A tab says `start` with its person; then each `fetch` is answered with its status and headers, its body's parts as
// they come, and `done` (or `error`); `cancel` stops one. `nodes` gives the order to ask nodes in, which is remembered;
// `reconnect` starts a fresh endpoint, after the network came back.

/// How long before a session's certificate ends it's renewed.
const RENEW = 3_600_000;
/// How long a relay socket opened for iroh waits for it to ask, before it's closed.
const UNTAKEN = 10_000;

/// The relay's socket, opened by `early` before iroh asks for one (while its WebAssembly loads, or as a fresh endpoint
/// is made), for its next ask: iroh's WebSocket is this one, and what came before it listened is kept for it.
let opened = null;
const Socket = self.WebSocket;
self.WebSocket = function (url, protocols) {
  const socket = opened;
  if (!socket || new URL(url).pathname !== "/relay") return new Socket(url, protocols);
  opened = null;
  clearTimeout(socket.untaken);
  return socket.face;
};
Object.setPrototypeOf(self.WebSocket, Socket);
self.WebSocket.prototype = Socket.prototype;

/// Opens the relay's socket with `token` (a certificate and its endpoint's proof) for iroh's next ask, closing one it
/// never took.
function early(token) {
  opened?.face.close();
  const ws = new Socket(`${location.origin.replace(/^http/, "ws")}/relay?token=${token}`, "iroh-relay-v2");
  ws.binaryType = "arraybuffer";
  // Its events, in order, until iroh listens to each kind.
  const on = {};
  const queued = [];
  const flush = () => {
    while (queued.length && on[queued[0][0]]) {
      const [type, e] = queued.shift();
      on[type](e);
    }
  };
  const face = { send: (data) => ws.send(data), close: (...why) => ws.close(...why) };
  for (const name of ["readyState", "protocol", "url", "bufferedAmount", "extensions"]) Object.defineProperty(face, name, { get: () => ws[name] });
  Object.defineProperty(face, "binaryType", { get: () => "arraybuffer", set() {} });
  for (const type of ["open", "message", "close", "error"]) {
    ws[`on${type}`] = (e) => (queued.push([type, e]), flush());
    Object.defineProperty(face, `on${type}`, { get: () => on[type] || null, set: (f) => ((on[type] = f), queueMicrotask(flush)) });
  }
  const socket = { face, untaken: setTimeout(() => opened === socket && ((opened = null), ws.close()), UNTAKEN) };
  opened = socket;
}

/// A server of tabs' requests, with the client's WebAssembly of `version` (`/client/<version>/`), which it starts
/// fetching and compiling at once.
export function server(version) {
  const base = `/client/${version}/`;
  const compiled = WebAssembly.compileStreaming(fetch(`${base}one_client_bg.wasm`));
  const glue = import(`${base}one_client.js`).then(async (g) => (await g.default({ module_or_path: await compiled }), g));
  glue.catch(() => {});
  // Imported here, not above, so the WebAssembly is fetched first.
  const code = Promise.all([import("/shared/crypto.js"), import("/kept.js")]);
  /// Of `keys`, the nodes the install's signed entries, as this browser last checked them (`global`), list as running
  /// something of `user`'s: all of them before it's checked any. A node the relay or a node names besides isn't asked.
  async function theirs(user, keys, kept) {
    const k = (kept === undefined ? await (await code)[1].get(`global:${user}`) : kept)?.members;
    if (!k) return keys;
    const uses = Object.keys(k.roles).filter((s) => k.roles[s][user]);
    return keys.filter((key) => (k.runs_for[key] || []).some((s) => uses.includes(s)));
  }

  /// `user`'s members from `global`'s entries kept and those `given` (a pack), checked and replayed by the client
  /// (src/members.rs), then kept: the entries, the install's first, what to ask for next, and the members.
  async function global(user, given) {
    const [, { get, put }] = await code;
    const kept = await get(`global:${user}`);
    if (!given && kept) return kept.members;
    const { Global } = await glue;
    const g = new Global(kept?.root || "", kept?.pack || new Uint8Array(), given || new Uint8Array());
    const members = JSON.parse(g.members);
    await put(`global:${user}`, { root: g.root, pack: g.pack, have: g.have, members });
    g.free();
    return members;
  }

  /// The session: its person and its client, once it's started; null if this browser isn't a device of theirs.
  let session = null;
  let renewal = null;

  /// A session for `user`: a key of its own, vouched for by this browser's device key (or `as`, another device of
  /// theirs: `{ key, sign }`), and the nodes to ask.
  async function begin(user, as) {
    const [{ certify, ed25519Public, hex, proveSocket, signedRequest, signingKey, FRESH, SESSION }, { getAll, put }] = await code;
    // In one read: the device's key, the nodes it asked before and who runs what, as it replayed it.
    const [mine, nodes, members] = await getAll([`device:${user}`, `nodes:${user}`, `global:${user}`]);
    const device = as || mine;
    if (!device) return null;
    const known = await theirs(user, nodes || [], members || null);
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const endpoint = await ed25519Public(secret);
    // What proves the session's key to the relay, now and when a fresh endpoint opens a socket.
    const key = await signingKey(secret);
    // A minute short of a day, as no certificate lasts longer than a day from when the relay or a node checks it.
    const until = Date.now() + SESSION - FRESH;
    const certificate = await certify(device.sign, hex.decode(device.key), endpoint, until);
    // The relay's socket, opening while the WebAssembly loads.
    const socket = async (certificate) => early(`${certificate}.${await proveSocket(key, endpoint)}`);
    await socket(certificate);
    const request = JSON.stringify(await signedRequest(key, endpoint, "one/relay", { computes: true, certificate }));
    // The computes, a new client's first nodes, which say whether this browser is let in at all: waited for only the
    // first time; after that, the nodes it asked before are asked at once, and the computes only checked beside them.
    const computes = fetch("/relay/computes", { method: "POST", headers: { "content-type": "application/json" }, body: request }).then(async (asked) => {
      // Not let in: this browser isn't one of its person's devices (yet, or any more).
      if (asked.status === 403) return null;
      if (!asked.ok) throw new Error("Your One App's network isn't answering.");
      return theirs(user, (await asked.json()).computes);
    });
    const order = (computes) => [...known.filter((k) => computes.includes(k)), ...computes.filter((k) => !known.includes(k))];
    const first = known.length ? known : await computes.catch((err) => (opened?.face.close(), Promise.reject(err)));
    if (!first) {
      opened?.face.close();
      return null;
    }
    // Kept, so the next open asks them at once.
    if (!known.length) put(`nodes:${user}`, first).catch(() => {});
    const { Client } = await glue;
    const client = await Client.start(secret, certificate, location.origin, known.length ? first : order(first));
    if (known.length) {
      computes.then(
        (all) => {
          if (all) return client.nodes(order(all));
          // Taken out: it forgets its nodes, and the next start finds out it isn't a device.
          put(`nodes:${user}`, []);
          client.close();
          session = null;
        },
        () => {},
      );
    }
    secret.fill(0);
    clearTimeout(renewal);
    if (!as) renewal = setTimeout(() => renew(user, client, endpoint, socket), until - RENEW - Date.now());
    return { client, endpoint, socket, certificate };
  }

  /// A new certificate for the session's endpoint before its last ends.
  async function renew(user, client, endpoint, socket) {
    const [{ certify, hex, FRESH, SESSION }, { get }] = await code;
    const device = await get(`device:${user}`);
    if (!device) return;
    const until = Date.now() + SESSION - FRESH;
    const certificate = await certify(device.sign, hex.decode(device.key), endpoint, until);
    const s = await session?.ready;
    if (s) s.certificate = certificate;
    await socket(certificate);
    await client.certify(certificate);
    renewal = setTimeout(() => renew(user, client, endpoint, socket), until - RENEW - Date.now());
  }

  /// The session for `user`, started once; started again if `again` (this browser was just let in), as `as` if given.
  function started(user, again, as) {
    if (!session || session.user !== user || again) {
      const old = session;
      const mine = (session = { user, ready: begin(user, as) });
      old?.ready.then((s) => s?.client.close()).catch(() => {});
      // A start that failed is tried again by the next.
      mine.ready.catch(() => session === mine && (session = null));
    }
    return session.ready;
  }

  /// A tab's connection: → what to do with each of its messages; `post(message, transfer)` answers it.
  function connect(post) {
    const reading = new Map();
    return async (m) => {
      const { id } = m;
      try {
        switch (m.op) {
          case "start": {
            const s = await started(m.user, m.again, m.as);
            return post({ id, device: !!s });
          }
          case "nodes": {
            const nodes = await theirs(m.user, m.nodes);
            await (await code)[1].put(`nodes:${m.user}`, nodes);
            const s = await session?.ready;
            return s?.client.nodes(nodes);
          }
          case "global":
            return post({ id, members: await global(m.user, m.given && new Uint8Array(m.given)) });
          case "reconnect": {
            const s = await session?.ready;
            if (!s) return;
            await s.socket(s.certificate);
            return s.client.reconnect();
          }
          case "cancel":
            return reading.get(id)?.cancel();
          case "fetch": {
            const s = await session?.ready;
            if (!s) throw new Error("This browser isn't one of your devices yet.");
            const r = await s.client.fetch(m.method, m.path, m.headers, new Uint8Array(m.body || []));
            reading.set(id, r);
            post({ id, node: r.node, status: r.status, headers: r.headers });
            try {
              for (let part; (part = await r.chunk()); ) post({ id, part }, [part.buffer]);
              post({ id, done: true });
            } finally {
              reading.delete(id);
            }
            return;
          }
        }
      } catch (err) {
        post({ id, error: String(err?.message || err) });
      }
    };
  }

  return { connect };
}

// As the SharedWorker: its version is in its URL, and each tab is a port.
if (typeof SharedWorkerGlobalScope !== "undefined" && self instanceof SharedWorkerGlobalScope) {
  const s = server(new URL(self.location.href).searchParams.get("client"));
  self.onconnect = (e) => {
    const port = e.ports[0];
    const handle = s.connect((m, transfer) => port.postMessage(m, transfer || []));
    port.onmessage = (e) => handle(e.data);
  };
}
