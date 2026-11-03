// The web client's page, kept in this browser so it opens with no trip: each of the client's own places (web.js's
// route()) is answered with the page as it was last, and the page as it is now is kept for next time. Its scripts and
// styles are named by their content (`?v=`), so the browser keeps them for good. Nothing else is touched: a node's
// answers, signing in and every other file go to the network as they would without this.
const KEPT = "one.page";
const PLACES = /^\/($|page\/|services\/|app\/|settings($|\/)|vault$)/;

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
