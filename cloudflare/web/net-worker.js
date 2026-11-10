// The browser's one way to its nodes: iroh (client/, as WebAssembly), in one SharedWorker every tab of the web client
// shares, so a browser keeps one connection however many tabs and screens ask; where there's no SharedWorker, in the
// one tab that holds the `one.net` lock, for the others (net.js).
//
// Its endpoint is a key of the session's, made here, which the browser's device key vouches for (`certify`): that key
// can't be read out of WebCrypto, and iroh signs with raw bytes. The relay and nodes take the endpoint as the device.
//
// The browser as a device of its user is onet's keyring (onet/src/keyring.rs, `Keyring` in its WebAssembly),
// here too: who's who as it reads it from the bucket itself, the keys it opens, and each request, with the keys its kind
// needed, sealed to the one node it's sent to. Its seal key's secret stays in WebCrypto: the keyring asks it for each
// secret it agrees (`shared`).
//
// A tab says `start` with its user; then each `fetch` is answered with its status and headers, its body's parts as
// they come, and `done` (or `error`); `cancel` stops one. What changes who can do what is a `fetch` too, of the path a
// node's route has, which the keyring answers itself, signing it (onet's `settings`). `settle`, `renew` and `push`
// are the rest a device does as itself; `ask` and `waited`, what it does before it's one, asking to be let in; and
// `auth`, signing in. `kept` gives what a call that only reads answered last (onet's
// `answers`), and `forget` forgets what this device kept, as it signs out. `nodes` gives the order to ask nodes in, which is remembered; `reconnect`
// starts a fresh endpoint, after the network came back.

/// How long before a session's certificate ends it's renewed, and how long after a renewal that failed it's tried again.
const RENEW = 3_600_000;
const RENEW_AGAIN = 60_000;
const UNTAKEN = 10_000;

/// The relay's socket, opened by `early` before iroh asks for one (while its WebAssembly loads, or as a fresh endpoint
/// is made), for its next ask: iroh's WebSocket is this one, and what came before it listened is kept for it.
let opened = null;
const Socket = self.WebSocket;

/// The relay's sockets that are open, and each tab, told as the last goes or the first comes: until the connection's own
/// timeout, half a minute on, this is the only sign the way to every node has gone.
const relays = new Set();
const tabs = new Set();
let relayed = false;
function relaySocket(url, protocols) {
  const ws = new Socket(url, protocols);
  const told = () => tabs.forEach((post) => post({ relay: relays.size > 0 }));
  ws.addEventListener("open", () => ((relayed = true), relays.add(ws), relays.size === 1 && told()));
  ws.addEventListener("close", () => relays.delete(ws) && !relays.size && told());
  return ws;
}

self.WebSocket = function (url, protocols) {
  const socket = opened;
  if (new URL(url).pathname !== "/relay") return new Socket(url, protocols);
  if (!socket) return relaySocket(url, protocols);
  opened = null;
  clearTimeout(socket.untaken);
  return socket.face;
};
Object.setPrototypeOf(self.WebSocket, Socket);
self.WebSocket.prototype = Socket.prototype;

function early(address, token) {
  opened?.face.close();
  const ws = relaySocket(`${address.replace(/^http/, "ws")}/relay?token=${token}`, "iroh-relay-v2");
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

/// Where this client stands (`server`'s `host`): the network's address, its device for a user (`{ key, sealKey, sign, seal }`,
/// `sign` and `seal` a browser's WebCrypto keys, or the app's functions), and what its keyring keeps for that user.
function browserHost(keptJs) {
  return {
    address: location.origin,
    device: async (user) => (await keptJs).get(`device:${user}`),
    keep: (user) => ({
      get: async (name) => (await keptJs).get(`${name}:${user}`),
      put: async (name, bytes) => (await keptJs).put(`${name}:${user}`, bytes),
      del: async (name) => (await keptJs).del(`${name}:${user}`),
    }),
  };
}

export function server(version, givenHost) {
  const base = `/onet/${version}/`;
  /// What's fetched once: if the connection drops while it comes, whoever asks next fetches it again, as `unreachable` says.
  const fetched = (get) => {
    let got = null;
    return () => (got ||= get().catch((err) => ((got = null), Promise.reject(Object.assign(err, { unreachable: true })))));
  };
  const glue = fetched(async () => {
    const compiled = WebAssembly.compileStreaming(fetch(`${base}onet_bg.wasm`));
    const g = await import(`${base}onet.js`);
    await g.default({ module_or_path: await compiled });
    return g;
  });
  glue().catch(() => {});
  // Imported here, not above, so the WebAssembly is fetched first.
  const code = fetched(() => Promise.all([import("/shared/crypto.js"), import("/shared/protocol.js"), import("/kept.js")]));
  const host = givenHost || browserHost(code().then((c) => c[2]));
  /// Of `keys`, the nodes the network's signed steps, as `device` last read them, list as running something of its
  /// user's: all of them before it's read any. A node the relay or a node names besides isn't asked.
  function theirs(device, keys) {
    const listed = device && JSON.parse(device.nodes()).nodes;
    return listed?.length ? keys.filter((key) => listed.includes(key)) : keys;
  }

  /// What `user`'s device (`me`, its keys as kept) reaches through JavaScript (onet's `js::On`, which reads the
  /// bucket through the URL signer with these): the network's address, and a JSON request to it; what it keeps (the host's); whether the relay still lets it in (`still`); its key's signature, and the secret
  /// its seal key agrees with another's, neither key read.
  async function reaches(user, me, still) {
    const [{ signBytes }] = await code();
    const text = new TextEncoder();
    const kept = host.keep(user);
    return {
      get: async (name) => {
        const v = await kept.get(name);
        return v == null ? null : typeof v === "string" ? text.encode(v) : v;
      },
      put: (name, bytes) => kept.put(name, bytes),
      delete: (name) => kept.del(name),
      letIn: () => still(),
      address: host.address,
      async ask(method, url, name, value, body) {
        const headers = { ...(name ? { [name]: value } : {}), ...(body != null ? { "content-type": "application/json" } : {}) };
        const r = await fetch(url, { method, headers, body: body ?? undefined });
        return { status: r.status, body: await r.text() };
      },
      sign: (bytes) => signBytes(me.sign, bytes),
      async shared(share) {
        if (typeof me.seal === "function") return me.seal(share);
        const theirs = await crypto.subtle.importKey("raw", share, { name: "X25519" }, false, []);
        return new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: theirs }, me.seal, 256));
      },
    };
  }

  /// Where this browser keeps its calls' answers for `user` (onet's `answers::Shelf`): IndexedDB's `one.answers`,
  /// each sealed (AES-GCM) under a key of its own that WebCrypto never lets out, made for one user: another's
  /// sign-in finds only what it can't open, which goes.
  function shelf(user) {
    const done = (req) => new Promise((ok, fail) => ((req.onsuccess = () => ok(req.result)), (req.onerror = () => fail(req.error))));
    const finished = (tx) => new Promise((ok, fail) => ((tx.oncomplete = () => ok()), (tx.onerror = tx.onabort = () => fail(tx.error))));
    let ready = null;
    const open = () =>
      (ready ||= (async () => {
        const opened = indexedDB.open("one.answers", 1);
        opened.onupgradeneeded = () => {
          opened.result.createObjectStore("key");
          opened.result.createObjectStore("answers").createIndex("at", "at");
        };
        const db = await done(opened);
        // Another tab forgetting them closes this one's hold, so they go at once.
        db.onversionchange = () => (db.close(), (ready = null));
        let kept = await done(db.transaction("key").objectStore("key").get("key"));
        if (kept?.user !== user || !(kept.key instanceof CryptoKey)) {
          kept = { user, key: await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]) };
          const tx = db.transaction(["key", "answers"], "readwrite");
          tx.objectStore("answers").clear();
          tx.objectStore("key").put(kept, "key");
          await finished(tx);
        }
        return { db, key: kept.key };
      })().catch((err) => ((ready = null), Promise.reject(err))));
    const aad = (name) => new TextEncoder().encode(`${user}\n${name}`);
    return {
      async get(name) {
        const { db, key } = await open();
        const got = await done(db.transaction("answers").objectStore("answers").get(name));
        if (!got) return null;
        return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: got.iv, additionalData: aad(name) }, key, got.sealed)).slice();
      },
      async put(name, plain, most) {
        const { db, key } = await open();
        const iv = crypto.getRandomValues(new Uint8Array(12));
        const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad(name) }, key, plain);
        const tx = db.transaction("answers", "readwrite");
        const answers = tx.objectStore("answers");
        answers.put({ at: Date.now(), iv, sealed }, name);
        const n = await done(answers.count());
        if (n > most) {
          let over = n - most;
          answers.index("at").openKeyCursor().onsuccess = (e) => {
            const c = e.target.result;
            if (!c || over-- <= 0) return;
            answers.delete(c.primaryKey);
            c.continue();
          };
        }
        await finished(tx);
      },
      async forget() {
        const was = ready;
        ready = null;
        await was?.then((o) => o.db.close(), () => {});
        await done(indexedDB.deleteDatabase("one.answers")).catch(() => {});
      },
    };
  }

  /// Each user's answers kept in this browser (`Answers`), made the first time they're asked for.
  const answering = new Map();
  const answersOf = async (user) => {
    const { Answers } = await glue();
    if (!answering.has(user)) answering.set(user, new Answers(shelf(user)));
    return answering.get(user);
  };

  /// `user`'s device asking to be let in, before it's one: its keyring, which reaches auth and keeps what it's told.
  async function asking(user) {
    const { Keyring } = await glue();
    const device = await host.device(user);
    if (!device) throw new Error("This browser has no keys yet. Sign in again.");
    return new Keyring(device.key, device.sealKey, await reaches(user, device, () => true, null));
  }

  /// The session: none until a tab starts it (this worker, or this tab leading the others, is new); then its user, and
  /// `ready`, its client once it's started, or null if this browser isn't a device of theirs.
  let session = null;
  let renewal = null;

  async function begin(user) {
    const took = {};
    let mark = performance.now();
    const step = (name) => ((took[name] = Math.round(performance.now() - mark)), (mark = performance.now()));
    const [{ certify, ed25519Public, hex, proveSocket, signedRequest, signingKey, untilTaken }, { FRESH, RELAY, SESSION, Yours }, { getAll, put }] = await code();
    const [device, [nodes]] = await Promise.all([host.device(user), getAll([`nodes:${user}`])]);
    if (!device) return null;
    // Its keyring, made while the session's key is.
    let made = null;
    const making = glue().then(async ({ Keyring }) => {
      const k = new Keyring(device.key, device.sealKey, await reaches(user, device, () => made?.still() ?? true));
      k.keeping(await answersOf(user));
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
    const socket = async (certificate) => early(host.address, `${certificate}.${await proveSocket(key, endpoint)}`);
    await socket(certificate);
    step("key");
    // The relay takes each request once, so one refused as replayed (another process of this device signed in the same
    // millisecond) is signed again.
    const ask = async (certificate) => {
      let answer;
      await untilTaken(async (at) => {
        const body = JSON.stringify(await signedRequest(key, endpoint, RELAY, { computes: true, certificate }, at));
        answer = await fetch(`${host.address}/relay/computes`, { method: "POST", headers: { "content-type": "application/json" }, body });
        return answer.status === 403 || answer.ok ? null : await answer.clone().json().catch(() => null);
      });
      return answer;
    };
    // The computes, a new client's first nodes, which say whether this browser is let in at all, and which of them are
    // on the relay's network now (`online`, raced first): waited for only the first time; after that, the nodes it
    // asked before are asked at once, and the computes only checked beside them.
    const computes = ask(certificate).then(async (asked) => {
      // Not let in: this browser isn't one of its user's devices (yet, or any more).
      if (asked.status === 403) return null;
      if (!asked.ok) throw new Error(`${Yours} isn’t answering.`);
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
    const { Client } = await glue();
    step("code");
    const [client, keyring] = await Promise.all([Client.start(secret, certificate, host.address, known.length ? first : order(first)), making]);
    keyring.connect(client);
    step("endpoint");
    computes.then(
      (c) => {
        if (c) {
          if (known.length) client.nodes(order(c.all));
          if (Array.isArray(c.online)) client.online(c.online);
          return;
        }
        // Taken out: it forgets its nodes, and the session says it isn't a device, keeping what it kept for if the relay
        // lets it in again. A session started since is that one's.
        put(`nodes:${user}`, []);
        client.close();
        const was = session;
        was?.ready.then((s) => s?.client === client && session === was && (session = { user, ready: Promise.resolve(null) }), () => {});
      },
      () => {},
    );
    secret.fill(0);
    clearTimeout(renewal);
    renewal = setTimeout(() => renew(user, client, endpoint, socket), until - RENEW - Date.now());
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
      const [{ certify, hex }, { FRESH, SESSION }] = await code();
      const device = await host.device(user);
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

  function started(user, again) {
    if (!session || session.user !== user || again) {
      const old = session;
      const mine = (session = { user, ready: begin(user) });
      old?.ready.then((s) => s?.client.close()).catch(() => {});
      mine.ready.catch(() => session === mine && (session = null));
    }
    return session.ready;
  }

  function connect(post) {
    const reading = new Map();
    tabs.add(post);
    if (relayed) post({ relay: relays.size > 0 });
    return async (m) => {
      const { id } = m;
      try {
        switch (m.op) {
          case "start": {
            const s = await started(m.user, m.again);
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
            await (await code())[2].put(`nodes:${m.user}`, nodes);
            // Each one's run priority, where the state said it: those on the same one are raced at once.
            const ranks = nodes.filter((k) => Number.isInteger(m.ranks?.[k]));
            if (ranks.length) s?.client.ranks(ranks, ranks.map((k) => m.ranks[k]));
            if (m.wakes && nodes.includes(m.wakes)) s?.client.wakes(m.wakes);
            return s?.client.nodes(nodes);
          }
          // Signing in at the network's auth, a step at a time (onet's `sign_in`): this browser registered (as
          // `name`, coming back to `back`) unless it has its `client` id, and an authorization asked for (`start`); what
          // came back exchanged for tokens (`finish`); its refresh token exchanged for new ones (`refresh`), or revoked.
          case "auth": {
            const { SignIn } = await glue();
            const http = {
              async send(method, url, type, body) {
                const r = await fetch(url, { method, headers: type ? { "content-type": type } : {}, body: body ?? undefined });
                return { status: r.status, body: await r.text() };
              },
            };
            const auth = await SignIn.at(host.address, http);
            if (m.step === "start") {
              const client = m.client || (await auth.register(m.name, m.back));
              return post({ id, client, ...JSON.parse(auth.ask(client, m.back, m.join)) });
            }
            if (m.step === "finish") return post({ id, tokens: JSON.parse(await auth.finish(JSON.stringify(m.asked), JSON.stringify(m.got))) });
            if (m.step === "refresh") return post({ id, tokens: JSON.parse(await auth.refresh(m.client, m.refresh)) });
            await auth.revoke(m.client, m.refresh);
            return post({ id });
          }
          // Asking auth to let this browser in, and what auth says of it while it waits (onet's `join`).
          case "ask": {
            return post({ id, invited: await (await asking(m.user)).ask(m.token, m.name, m.platform ?? undefined, m.invite ?? undefined) });
          }
          case "waited":
            return post({ id, said: await (await asking(m.user)).waited(m.token) });
          // What a call that only reads answered last, kept here: at once, whether or not a session started.
          case "kept": {
            const got = await (await answersOf(m.user)).kept(m.app, m.tool, JSON.stringify(m.input ?? {}));
            return post({ id, kept: got ? JSON.parse(got) : null });
          }
          // Signed out: what this device kept as its user's goes.
          case "forget": {
            const s = await session?.ready.catch(() => null);
            await s?.keyring.forget();
            await (await answersOf(m.user || "")).forget();
            return post({ id });
          }
          // As each start of a client does: its user's own things, settled if they aren't.
          case "settle": {
            const s = await session?.ready;
            if (s) await s.keyring.settle(m.platform);
            return post({ id });
          }
          // Its user's app schedules of `app`, renewed as they use it.
          case "renew": {
            const s = await session?.ready;
            if (s) await s.keyring.renewAppSchedules(m.app);
            return post({ id });
          }
          // This browser's push subscription, registered with the network's push Worker.
          case "push": {
            const s = await session?.ready;
            if (!s) throw new Error("This browser isn’t one of your devices yet.");
            await s.keyring.registerPush(JSON.stringify(m.subscription));
            return post({ id });
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
            if (!s) throw new Error("This browser isn’t one of your devices yet.");
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
        post({ id, error: String(err?.message || err), trace: err?.trace || "", notLetIn: !!err?.notLetIn, signedOut: !!err?.signedOut, unreachable: !!err?.unreachable, code: err?.code || undefined });
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
