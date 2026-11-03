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

/// A server of tabs' requests, with the client's WebAssembly of `version` (`/client/<version>/`), which it starts
/// fetching and compiling at once.
export function server(version) {
  const base = `/client/${version}/`;
  const compiled = WebAssembly.compileStreaming(fetch(`${base}one_client_bg.wasm`));
  const glue = import(`${base}one_client.js`).then(async (g) => (await g.default({ module_or_path: await compiled }), g));
  glue.catch(() => {});
  // Imported here, not above, so the WebAssembly is fetched first.
  const code = Promise.all([import("../shared/crypto.js"), import("./kept.js")]);
  /// The session: its person and its client, once it's started; null if this browser isn't a device of theirs.
  let session = null;
  let renewal = null;

  /// A session for `user`: a key of its own, vouched for by this browser's device key, and the nodes to ask.
  async function begin(user) {
    const [{ certify, ed25519Public, hex, SESSION }, { get, put }] = await code;
    const device = await get(`device:${user}`);
    if (!device) return null;
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const endpoint = await ed25519Public(secret);
    const until = Date.now() + SESSION;
    const certificate = await certify(device.sign, hex.decode(device.key), endpoint, until);
    // The computes, a new client's first nodes, which say whether this browser is let in at all: waited for only the
    // first time; after that, the nodes it asked before are asked at once, and the computes only checked beside them.
    const computes = fetch("/relay/computes", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ certificate }) }).then(async (asked) => {
      // Not let in: this browser isn't one of its person's devices (yet, or any more).
      if (asked.status === 403) return null;
      if (!asked.ok) throw new Error("Your One App's network isn't answering.");
      return (await asked.json()).computes;
    });
    const known = (await get(`nodes:${user}`)) || [];
    const order = (computes) => [...known.filter((k) => computes.includes(k)), ...computes.filter((k) => !known.includes(k))];
    const first = known.length ? known : await computes;
    if (!first) return null;
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
    renewal = setTimeout(() => renew(user, client, endpoint), until - RENEW - Date.now());
    return { client, endpoint };
  }

  /// A new certificate for the session's endpoint before its last ends.
  async function renew(user, client, endpoint) {
    const [{ certify, hex, SESSION }, { get }] = await code;
    const device = await get(`device:${user}`);
    if (!device) return;
    const until = Date.now() + SESSION;
    await client.certify(await certify(device.sign, hex.decode(device.key), endpoint, until));
    renewal = setTimeout(() => renew(user, client, endpoint), until - RENEW - Date.now());
  }

  /// The session for `user`, started once; started again if `again` (this browser was just let in).
  function started(user, again) {
    if (!session || session.user !== user || again) {
      const old = session;
      const mine = (session = { user, ready: begin(user) });
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
            const s = await started(m.user, m.again);
            return post({ id, device: !!s });
          }
          case "nodes": {
            await (await code)[1].put(`nodes:${m.user}`, m.nodes);
            const s = await session?.ready;
            return s?.client.nodes(m.nodes);
          }
          case "reconnect":
            return (await session?.ready)?.client.reconnect();
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
