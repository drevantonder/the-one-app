// The web client's page, kept in this browser so it opens with no trip: each of the client's own places (web.js's
// route()) is answered with the page as it was last, and the page as it is now is kept for next time. Its scripts and
// styles are named by their content (`?v=`), so the browser keeps them for good. Nothing else is touched: a node's
// answers, signing in and every other file go to the network as they would without this.
//
// And its notifications: a push (kernel/push/) carries a note sealed to one of this browser's devices (one per person
// signed in here), which it opens with that device's seal key, kept in IndexedDB where it can't be read out
// (device.js), and shows; tapping it opens the screen it names. A note it can't open still shows that one came, as
// browsers require of every push.
const KEPT = "one.page";
const PLACES = /^\/($|page\/|services\/|app\/|settings($|\/)|vault$)/;
/// The white label's name, which the kernel writes in (Settings::web).
const NAME = "";

addEventListener("install", (e) => e.waitUntil(caches.open(KEPT).then((c) => c.add("/index.html")).catch(() => {}).then(() => skipWaiting())));
addEventListener("activate", (e) => e.waitUntil(clients.claim()));

addEventListener("fetch", (e) => {
  const url = new URL(e.request.url);
  if (e.request.mode !== "navigate" || e.request.method !== "GET" || url.origin !== location.origin || !PLACES.test(url.pathname)) return;
  const fresh = fetch("/index.html", { cache: "no-cache" }).then(async (r) => {
    if (r.ok) await (await caches.open(KEPT)).put("/index.html", r.clone());
    return r;
  });
  e.waitUntil(fresh.catch(() => {}));
  e.respondWith(caches.open(KEPT).then((c) => c.match("/index.html")).then((kept) => kept || fresh).catch(() => fetch(e.request)));
});

// A note comes as base64url text (kernel/push/senders.js).
addEventListener("push", (e) => e.waitUntil(show(e.data ? unb64u(e.data.text()) : null)));

async function show(sealed) {
  const note = sealed && (await opened(sealed).catch(() => null));
  if (!note?.title) return registration.showNotification(NAME, { body: "You have a new notification.", icon: "/icon.png" });
  const link = note.app ? { app: note.app, screen: note.screen || "main", params: note.params || {} } : null;
  return registration.showNotification(note.title, { body: note.body || "", tag: note.id || undefined, icon: "/icon.png", data: { link } });
}

addEventListener("notificationclick", (e) => {
  e.notification.close();
  const link = e.notification.data?.link;
  const q = new URLSearchParams();
  if (link && Object.keys(link.params).length) q.set("params", JSON.stringify(link.params));
  // Settings' own screens (Settings › Devices, which a device asking to join opens) are the client's, not an app's.
  const place = link?.app === "settings" ? `/settings/${encodeURIComponent(link.screen)}` : link ? `/app/${encodeURIComponent(link.app)}/${encodeURIComponent(link.screen)}` : "/";
  const to = `${place}${link && q.size ? `?${q}` : ""}`;
  e.waitUntil(
    clients.matchAll({ type: "window", includeUncontrolled: true }).then(async (open) => {
      const here = open.find((c) => new URL(c.url).origin === location.origin);
      if (!here) return clients.openWindow(to);
      await here.focus();
      return here.navigate(to).catch(() => clients.openWindow(to));
    }),
  );
});

// MARK: Opening a note

/// The note sealed to one of this browser's devices, which are kept as `device:<user>` in IndexedDB's `one` (kept.js).
async function opened(sealed) {
  for (const d of await devices()) {
    try {
      return JSON.parse(new TextDecoder().decode(await openSealed(d.seal, unhex(d.sealKey), sealed)));
    } catch {}
  }
  return null;
}

function devices() {
  return new Promise((resolve, reject) => {
    const open = indexedDB.open("one", 1);
    open.onupgradeneeded = () => open.result.createObjectStore("kept");
    open.onerror = () => reject(open.error);
    open.onsuccess = () => {
      const out = [];
      const tx = open.result.transaction("kept", "readonly");
      const range = IDBKeyRange.bound("device:", "device:￿");
      tx.objectStore("kept").openCursor(range).onsuccess = (e) => {
        const cursor = e.target.result;
        if (!cursor) return;
        if (cursor.value?.seal && cursor.value.sealKey) out.push(cursor.value);
        cursor.continue();
      };
      tx.oncomplete = () => (open.result.close(), resolve(out));
      tx.onerror = () => (open.result.close(), reject(tx.error));
    };
  });
}

const unhex = (s) => Uint8Array.from(s.match(/../g) || [], (b) => parseInt(b, 16));
const unb64u = (s) => {
  try {
    return Uint8Array.from(atob(s.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
};

/// Opens what was sealed to an X25519 key pair (`secret`, a CryptoKey that can't be read out, and `public`): as
/// shared/crypto.js's `openSealed`, which a service worker, not a module, can't import. share ‖ AES-256-GCM(HKDF-SHA256
/// of the shared secret, salted with share ‖ public, "one/sealed-to"; zero nonce).
async function openSealed(secret, public_, sealed) {
  if (sealed.length < 48) throw new Error("not sealed");
  const share = sealed.slice(0, 32);
  const them = await crypto.subtle.importKey("raw", share, { name: "X25519" }, false, []);
  const shared = await crypto.subtle.deriveBits({ name: "X25519", public: them }, secret, 256);
  const salt = new Uint8Array(64);
  salt.set(share), salt.set(public_, 32);
  const ikm = await crypto.subtle.importKey("raw", shared, "HKDF", false, ["deriveBits"]);
  const bits = await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt, info: new TextEncoder().encode("one/sealed-to") }, ikm, 256);
  const key = await crypto.subtle.importKey("raw", bits, "AES-GCM", false, ["decrypt"]);
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: new Uint8Array(12) }, key, sealed.slice(32)));
}
