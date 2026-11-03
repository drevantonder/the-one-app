// The page's first script, run inline as the page arrives (Settings::web puts it in, its hash in the page's policy): it
// starts this browser's one client (net-worker.js, by the name one-files gives it) with whoever is signed in here, so
// its WebAssembly and the relay's socket are on their way while the page draws. net.js talks to it.
(() => {
  const meta = (name) => document.querySelector(`meta[name="${name}"]`)?.content || "";
  const client = meta("one-client");
  const named = meta("one-files").split(" ").find((f) => f.startsWith("net-worker.js="));
  if (!client || !named) return;
  self.oneNet = { url: `/net-worker.js?v=${named.slice("net-worker.js=".length)}&client=${encodeURIComponent(client)}` };
  if (typeof SharedWorker !== "function") return;
  const worker = (self.oneNet.worker = new SharedWorker(self.oneNet.url, { type: "module", name: "one" }));
  try {
    const access = JSON.parse(localStorage.getItem("one.signin") || "null")?.access;
    const user = JSON.parse(atob(access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub;
    if (user) worker.port.postMessage({ op: "start", user, id: "start" });
  } catch {}
})();
