// The last answer of each call screens make that only reads, kept in this browser for its person, for bridge.js's
// `kept`: a page opened before shows at once. IndexedDB's `one.answers` holds them sealed (AES-GCM) under a key of its
// own that WebCrypto never lets out of this browser, made for one person: another's sign-in finds only what it can't
// open, which goes. Forgotten with the sign-in.

const DB = "one.answers";
const MOST = 300;
const BIGGEST = 1_000_000;

const done = (req) => new Promise((ok, fail) => ((req.onsuccess = () => ok(req.result)), (req.onerror = () => fail(req.error))));
const finished = (tx) => new Promise((ok, fail) => ((tx.oncomplete = () => ok()), (tx.onerror = tx.onabort = () => fail(tx.error))));

const canonical = (v) => JSON.stringify(v, (_, x) => (x && typeof x === "object" && !Array.isArray(x) ? Object.fromEntries(Object.entries(x).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) : x));

let opening = null;

function ready(user) {
  if (opening?.user === user) return opening.ready;
  opening?.ready.then((o) => o.db.close(), () => {});
  const ready = (async () => {
    const open = indexedDB.open(DB, 1);
    open.onupgradeneeded = () => {
      open.result.createObjectStore("key");
      open.result.createObjectStore("answers").createIndex("at", "at");
    };
    const db = await done(open);
    // Another tab forgetting them closes this one's hold, so they go at once.
    db.onversionchange = () => (db.close(), opening?.ready === ready && (opening = null));
    let kept = await done(db.transaction("key").objectStore("key").get("key"));
    if (kept?.user !== user || !(kept.key instanceof CryptoKey)) {
      kept = { user, key: await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, false, ["encrypt", "decrypt"]) };
      const tx = db.transaction(["key", "answers"], "readwrite");
      tx.objectStore("answers").clear();
      tx.objectStore("key").put(kept, "key");
      await finished(tx);
    }
    return { db, key: kept.key };
  })();
  opening = { user, ready };
  ready.catch(() => opening?.ready === ready && (opening = null));
  return ready;
}

async function named(user, app, op, input) {
  const id = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical([app, op, input])))), (b) => b.toString(16).padStart(2, "0")).join("");
  return { id, aad: new TextEncoder().encode(`${user}\n${id}`) };
}

export async function kept(user, app, op, input) {
  if (!user) return null;
  const { db, key } = await ready(user);
  const { id, aad } = await named(user, app, op, input);
  const got = await done(db.transaction("answers").objectStore("answers").get(id));
  if (!got) return null;
  try {
    const plain = await crypto.subtle.decrypt({ name: "AES-GCM", iv: got.iv, additionalData: aad }, key, got.sealed);
    return { answer: JSON.parse(new TextDecoder().decode(plain)), at: got.at };
  } catch {
    return null;
  }
}

export async function keep(user, app, op, input, answer) {
  const json = JSON.stringify(answer ?? null);
  if (!user || json.length > BIGGEST) return;
  const { db, key } = await ready(user);
  const { id, aad } = await named(user, app, op, input);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv, additionalData: aad }, key, new TextEncoder().encode(json));
  const tx = db.transaction("answers", "readwrite");
  const answers = tx.objectStore("answers");
  answers.put({ at: Date.now(), iv, sealed }, id);
  const n = await done(answers.count());
  if (n > MOST) {
    let over = n - MOST;
    answers.index("at").openKeyCursor().onsuccess = (e) => {
      const c = e.target.result;
      if (!c || over-- <= 0) return;
      answers.delete(c.primaryKey);
      c.continue();
    };
  }
  await finished(tx);
}

export async function forget() {
  const was = opening;
  opening = null;
  await was?.ready.then((o) => o.db.close(), () => {});
  await done(indexedDB.deleteDatabase(DB)).catch(() => {});
}
