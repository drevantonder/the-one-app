// The browser's one way to its nodes: iroh (client/, as WebAssembly), in one SharedWorker every tab of the web client
// shares, so a browser keeps one connection however many tabs and screens ask; where there's no SharedWorker, in the
// one tab that holds the `one.net` lock, for the others (net.js).
//
// Its endpoint is a key of the session's, made here, which the browser's device key vouches for (`certify`): that key
// can't be read out of WebCrypto, and iroh signs with raw bytes. The relay and nodes take the endpoint as the device.
//
// The browser as a device of its person is one-client's keyring (client/src/keyring.rs, `Keyring` in its WebAssembly),
// here too: who's who as it reads it from the bucket itself, the keys it opens, and each request, with the keys its kind
// needed, sealed to the one node it's sent to. Its seal key's secret stays in WebCrypto: the keyring asks it for each
// secret it agrees (`shared`).
//
// A tab says `start` with its person; then each `fetch` is answered with its status and headers, its body's parts as
// they come, and `done` (or `error`); `cancel` stops one. `members` gives who's who as the keyring read it (with what's
// new in the bucket if `fresh`). `nodes` gives the order to ask nodes in, which is remembered; `reconnect` starts a
// fresh endpoint, after the network came back.

/// How long before a session's certificate ends it's renewed, and how long after a renewal that failed it's tried again.
const RENEW = 3_600_000;
const RENEW_AGAIN = 60_000;
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

function early(token) {
  opened?.face.close();
  const ws = new Socket(`${location.origin.replace(/^http/, "ws")}/relay?token=${token}`, "iroh-relay-v2");
  ws.binaryType = "arraybuffer";
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

export function server(version) {
  const base = `/client/${version}/`;
  const compiled = WebAssembly.compileStreaming(fetch(`${base}one_client_bg.wasm`));
  const glue = import(`${base}one_client.js`).then(async (g) => (await g.default({ module_or_path: await compiled }), g));
  glue.catch(() => {});
  // Imported here, not above, so the WebAssembly is fetched first.
  const code = Promise.all([import("/shared/crypto.js"), import("/shared/protocol.js"), import("/kept.js")]);
  /// Of `keys`, the nodes the network's signed steps, as `device` last read them, list as running something of its
  /// person's: all of them before it's read any. A node the relay or a node names besides isn't asked.
  function theirs(device, keys) {
    const listed = device && JSON.parse(device.nodes()).nodes;
    return listed?.length ? keys.filter((key) => listed.includes(key)) : keys;
  }

  /// What `user`'s device (`me`, its keys as kept) reaches through JavaScript (one-client's `js::On`): the bucket,
  /// through the URL signer's `get`, for who's who and the keys sealed there; what it keeps, in IndexedDB by person, or
  /// in `memory` for one asked as for a while; whether the relay still lets it in (`still`); and the secret its seal key
  /// agrees with another's.
  async function reaches(user, me, still, memory) {
    const [{ b64u, hex, signedRequest }, { READS, URLS, Yours }, { del, get, put }] = await code;
    const { following } = await glue;
    // Steps and what's sealed beside them never change once written: those read with one asked for, kept until asked.
    const ahead = new Map();
    const bucket = {
      async get(name) {
        if (ahead.has(name)) {
          const had = ahead.get(name);
          ahead.delete(name);
          return had;
        }
        const names = [name, ...JSON.parse(following(name, 15))].slice(0, 32);
        const body = JSON.stringify(await signedRequest(me.sign, hex.decode(me.key), URLS, { get: names, reads: READS }));
        const r = await fetch("/urls", { method: "POST", headers: { "content-type": "application/json" }, body });
        const out = await r.json().catch(() => ({}));
        if (!r.ok) throw Object.assign(new Error(out.error || `${Yours}'s storage didn't answer. Try again.`), { unanswered: r.status >= 500 });
        const object = (n) => (out.get?.[n] ? { bytes: b64u.decode(out.get[n]), tag: "" } : null);
        for (const n of names.slice(1)) if (out.get?.[n]) ahead.set(n, object(n));
        return object(name);
      },
      wait: (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
    };
    const text = new TextEncoder();
    const kept = memory ? { get: async (k) => memory.get(k), put: async (k, v) => void memory.set(k, v), del: async (k) => void memory.delete(k) } : { get, put, del };
    return {
      bucket,
      get: async (name) => {
        const v = await kept.get(`${name}:${user}`);
        return v == null ? null : typeof v === "string" ? text.encode(v) : v;
      },
      put: (name, bytes) => kept.put(`${name}:${user}`, bytes),
      delete: (name) => kept.del(`${name}:${user}`),
      letIn: () => still(),
      async shared(share) {
        const theirs = await crypto.subtle.importKey("raw", share, { name: "X25519" }, false, []);
        return new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: theirs }, me.seal, 256));
      },
    };
  }

  /// The session: none until a tab starts it (this worker, or this tab leading the others, is new); then its person, and
  /// `ready`, its client once it's started, or null if this browser isn't a device of theirs.
  let session = null;
  let renewal = null;

  async function begin(user, as) {
    const took = {};
    let mark = performance.now();
    const step = (name) => ((took[name] = Math.round(performance.now() - mark)), (mark = performance.now()));
    const [{ certify, ed25519Public, hex, proveSocket, signedRequest, signingKey, untilTaken }, { FRESH, RELAY, SESSION, Yours }, { del, getAll, put }] = await code;
    const [mine, nodes, global, root] = await getAll([`device:${user}`, `nodes:${user}`, `global:${user}`, `root:${user}`]);
    // Who's who as this browser checked it before its keyring kept it.
    if (global) {
      if (global.steps) await put(`who.json:${user}`, new TextEncoder().encode(JSON.stringify(global.steps)));
      if (global.root && !root) await put(`root:${user}`, global.root);
      await del(`global:${user}`);
    }
    const device = as || mine;
    if (!device) return null;
    // Its keyring, made while the session's key is: this browser's, or another device's of its person's asked as while
    // it lets this one in (a passkey's), which keeps nothing.
    let made = null;
    const making = glue.then(async ({ Keyring }) => {
      const k = new Keyring(device.key, device.sealKey, await reaches(user, device, () => made?.still() ?? true, as ? new Map() : null));
      await k.restore();
      return k;
    });
    // As this browser last kept them, of those its keyring said (`theirs`).
    const known = nodes || [];
    const secret = crypto.getRandomValues(new Uint8Array(32));
    const endpoint = await ed25519Public(secret);
    const key = await signingKey(secret);
    // A minute short of a day, as no certificate lasts longer than a day from when the relay or a node checks it.
    const until = Date.now() + SESSION - FRESH;
    const certificate = await certify(device.sign, hex.decode(device.key), endpoint, until);
    const socket = async (certificate) => early(`${certificate}.${await proveSocket(key, endpoint)}`);
    await socket(certificate);
    step("key");
    // The relay takes each request once, so one refused as replayed (another process of this device signed in the same
    // millisecond) is signed again.
    const ask = async (certificate) => {
      let answer;
      await untilTaken(async (at) => {
        const body = JSON.stringify(await signedRequest(key, endpoint, RELAY, { computes: true, certificate }, at));
        answer = await fetch("/relay/computes", { method: "POST", headers: { "content-type": "application/json" }, body });
        return answer.status === 403 || answer.ok ? null : await answer.clone().json().catch(() => null);
      });
      return answer;
    };
    // The computes, a new client's first nodes, which say whether this browser is let in at all, and which of them are
    // on the relay's network now (`online`, raced first): waited for only the first time; after that, the nodes it
    // asked before are asked at once, and the computes only checked beside them.
    const computes = ask(certificate).then(async (asked) => {
      // Not let in: this browser isn't one of its person's devices (yet, or any more).
      if (asked.status === 403) return null;
      if (!asked.ok) throw new Error(`${Yours} isn't answering.`);
      const said = await asked.json();
      return { all: theirs(await making, said.computes), online: said.online };
    });
    const order = (computes) => [...known.filter((k) => computes.includes(k)), ...computes.filter((k) => !known.includes(k))];
    const first = known.length ? known : (await computes.catch((err) => (opened?.face.close(), Promise.reject(err))))?.all;
    if (!known.length) step("nodes");
    if (!first) {
      opened?.face.close();
      return null;
    }
    if (!known.length) put(`nodes:${user}`, first).catch(() => {});
    const { Client } = await glue;
    step("code");
    const [client, keyring] = await Promise.all([Client.start(secret, certificate, location.origin, known.length ? first : order(first)), making]);
    keyring.connect(client);
    step("endpoint");
    computes.then(
      (c) => {
        if (c) {
          if (known.length) client.nodes(order(c.all));
          if (Array.isArray(c.online)) client.online(c.online);
          return;
        }
        // Taken out: it forgets its nodes, and the session says it isn't a device. A session started since is that
        // one's.
        put(`nodes:${user}`, []);
        keyring.forget();
        client.close();
        const was = session;
        was?.ready.then((s) => s?.client === client && session === was && (session = { user, ready: Promise.resolve(null) }), () => {});
      },
      () => {},
    );
    secret.fill(0);
    clearTimeout(renewal);
    if (!as) renewal = setTimeout(() => renew(user, client, endpoint, socket), until - RENEW - Date.now());
    made = { client, keyring, endpoint, socket, certificate, took };
    made.still = async () => {
      const asked = await ask(made.certificate);
      const online = asked.ok && (await asked.json().catch(() => null))?.online;
      if (Array.isArray(online)) client.online(online);
      return asked.status !== 403;
    };
    return made;
  }

  /// A new certificate for the session's endpoint before its last ends, while it's the session: one that fails is tried
  /// again in a minute, so a moment offline doesn't end it.
  async function renew(user, client, endpoint, socket) {
    const s = await session?.ready.catch(() => null);
    if (s?.client !== client) return;
    try {
      const [{ certify, hex }, { FRESH, SESSION }, { get }] = await code;
      const device = await get(`device:${user}`);
      if (!device) return;
      const until = Date.now() + SESSION - FRESH;
      const certificate = await certify(device.sign, hex.decode(device.key), endpoint, until);
      await socket(certificate);
      await client.certify(certificate);
      s.certificate = certificate;
      renewal = setTimeout(() => renew(user, client, endpoint, socket), until - RENEW - Date.now());
    } catch (err) {
      console.warn("renewing this browser's session:", err);
      renewal = setTimeout(() => renew(user, client, endpoint, socket), RENEW_AGAIN);
    }
  }

  function started(user, again, as) {
    if (!session || session.user !== user || again) {
      const old = session;
      const mine = (session = { user, ready: begin(user, as) });
      old?.ready.then((s) => s?.client.close()).catch(() => {});
      mine.ready.catch(() => session === mine && (session = null));
    }
    return session.ready;
  }

  function connect(post) {
    const reading = new Map();
    return async (m) => {
      const { id } = m;
      try {
        switch (m.op) {
          case "start": {
            const s = await started(m.user, m.again, m.as);
            return post({ id, device: !!s, took: s?.took });
          }
          // Whether the relay still lets this browser in: null, unknown, while it has no session, as it hasn't started
          // one or the last failed, which says nothing about whether it's a device.
          case "still": {
            if (!session) return post({ id, device: null });
            const s = await session.ready.catch(() => undefined);
            return post({ id, device: s === undefined ? null : s ? await s.still() : false });
          }
          case "nodes": {
            const s = await session?.ready;
            const nodes = theirs(s?.keyring, m.nodes);
            await (await code)[2].put(`nodes:${m.user}`, nodes);
            // Each one's run priority, where the state said it: those on the same one are raced at once.
            const ranks = nodes.filter((k) => Number.isInteger(m.ranks?.[k]));
            if (ranks.length) s?.client.ranks(ranks, ranks.map((k) => m.ranks[k]));
            if (m.wakes && nodes.includes(m.wakes)) s?.client.wakes(m.wakes);
            return s?.client.nodes(nodes);
          }
          // Who's who as this device reads it, from what's new in the bucket if `fresh`.
          case "members": {
            const s = await session?.ready;
            if (!s) throw new Error("This browser isn't one of your devices yet.");
            await (m.fresh ? s.keyring.read() : s.keyring.load());
            return post({ id, members: JSON.parse(s.keyring.members() || "null") });
          }
          // A new device's sign-in, checked by the client (`signedInWith`), its issuer's keys fetched from where it
          // publishes them.
          case "signedInWith": {
            const { issuerKeysAt, signedInWith } = await glue;
            const proof = JSON.stringify(m.proof);
            const at = issuerKeysAt(proof, m.address);
            const keys = at ? await (await fetch(at)).text() : "";
            return post({ id, account: JSON.parse(signedInWith(proof, m.key, m.seal, m.address, m.root, m.clients || "", keys, m.accounts ? JSON.stringify(m.accounts) : undefined, Date.now())) });
          }
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
            const body = new Uint8Array(m.body || []);
            const r = await s.keyring.fetch(m.method, m.path, m.headers, body, m.until, !!m.trace);
            reading.set(id, r);
            post({ id, node: r.node, status: r.status, headers: r.headers, trace: r.trace });
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
        post({ id, error: String(err?.message || err), trace: err?.trace || "", notLetIn: !!err?.notLetIn, code: err?.code || undefined });
      }
    };
  }

  return { connect };
}

if (typeof SharedWorkerGlobalScope !== "undefined" && self instanceof SharedWorkerGlobalScope) {
  const s = server(new URL(self.location.href).searchParams.get("client"));
  self.onconnect = (e) => {
    const port = e.ports[0];
    const handle = s.connect((m, transfer) => port.postMessage(m, transfer || []));
    port.onmessage = (e) => handle(e.data);
  };
}
