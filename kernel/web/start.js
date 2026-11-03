// The page's first script, run inline as the page arrives (Settings::web puts it in, its hash in the page's policy): it
// starts this browser's one client (net-worker.js, by the name one-files gives it) with whoever is signed in here, so
// its WebAssembly and the relay's socket are on their way while the page draws. net.js talks to it.
(() => {
  const meta = (name) => document.querySelector(`meta[name="${name}"]`)?.content || "";
  const client = meta("one-client");
  const named = meta("one-files").split(" ").find((f) => f.startsWith("net-worker.js="));
  if (!client || !named) return;
  self.oneNet = { url: `/net-worker.js?v=${named.slice("net-worker.js=".length)}&client=${encodeURIComponent(client)}` };
  // The client's WebAssembly (and its glue) is asked for now, not once the worker runs: the worker's own request, when it
  // gets there, is answered by this one as it arrives, from the browser's cache (the files are named by their build).
  // One the browser already has isn't read again.
  for (const file of ["one_client_bg.wasm", "one_client.js"]) {
    const url = `/client/${client}/${file}`;
    fetch(url, { cache: "only-if-cached", mode: "same-origin" })
      .then((r) => (r.ok ? r.body.cancel() : Promise.reject()))
      .catch(() => fetch(url).then((r) => r.arrayBuffer()))
      .catch(() => {});
  }
  if (typeof SharedWorker !== "function") return;
  const worker = (self.oneNet.worker = new SharedWorker(self.oneNet.url, { type: "module", name: "one" }));
  try {
    const access = JSON.parse(localStorage.getItem("one.signin") || "null")?.access;
    const user = JSON.parse(atob(access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub;
    if (user) worker.port.postMessage({ op: "start", user, id: "start" });
  } catch {}
})();
