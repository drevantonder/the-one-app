// The page's first script, run inline as the page arrives (Settings::web puts it in, its hash in the page's policy): it
// starts this browser's one client (net-worker.js, by the name one-files gives it) with whoever is signed in here, so
// its WebAssembly and the relay's socket are on their way while the page draws. net.js talks to it.
(() => {
  // Light or dark as the user chose, where it isn't the system's: the page's own colour now, and nothing drawn until
  // web.js has switched its rules to it (`schemed`), so no frame is the other's. Shown anyway after a second.
  const look = localStorage.getItem("one.appearance");
  if (look === "light" || look === "dark") {
    const root = document.documentElement;
    root.style.colorScheme = look;
    root.style.visibility = "hidden";
    setTimeout(() => (root.style.visibility = ""), 1000);
  }
  // A page that draws nothing is a problem, not a black or white screen: if no text, image or control is on it a few seconds
  // after it starts (web.js never ran, or a frame was laid out at no size), it says so, with the way out, and the app's log
  // (`one logs`) gets a line. This is the page's first script and needs nothing else of it.
  const drawn = () => {
    for (const el of document.body.querySelectorAll("*")) {
      const r = el.getBoundingClientRect();
      if (r.width < 1 || r.height < 1 || el.closest("#one-blank") || getComputedStyle(el).visibility === "hidden") continue;
      if (el.matches("img, svg, canvas, video, iframe, input, button, textarea, select") || [...el.childNodes].some((n) => n.nodeType === 3 && n.textContent.trim())) return true;
    }
    return false;
  };
  const blank = () => {
    if (document.hidden) return addEventListener("visibilitychange", () => setTimeout(blank, 3000), { once: true });
    if (drawn()) return;
    const root = document.documentElement;
    root.style.visibility = "";
    const box = document.createElement("div");
    box.id = "one-blank";
    box.setAttribute("role", "alert");
    box.style.cssText = "position:fixed;inset:0;display:flex;align-items:center;justify-content:center;padding:1rem;color-scheme:light dark;background:Canvas;color:CanvasText;font:14px system-ui,sans-serif";
    box.innerHTML =
      '<div style="max-width:28rem;display:grid;grid-template-columns:1.25rem 1fr;gap:.25rem .75rem;padding:1rem;border:1px solid color-mix(in srgb,CanvasText 15%,transparent);border-radius:.5rem;color:#dc2626">' +
      '<svg width="20" height="20" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"><path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm-8,56a8,8,0,0,1,16,0v56a8,8,0,0,1-16,0Zm8,104a12,12,0,1,1,12-12A12,12,0,0,1,128,184Z"/></svg>' +
      '<div style="font-weight:500">This screen didn\u2019t draw</div><div></div>' +
      '<div style="color:color-mix(in srgb,CanvasText 60%,transparent)">Nothing showed after it started. Reloading usually fixes it.<br><button style="margin-top:.5rem;padding:.25rem .75rem;font:inherit;color:CanvasText;background:transparent;border:1px solid color-mix(in srgb,CanvasText 25%,transparent);border-radius:.375rem" onclick="location.reload()">Reload</button></div></div>';
    document.body.append(box);
    const note = `blank: nothing drawn ${Math.round(performance.now() / 1000)} s after it started (${innerWidth}x${innerHeight}, ${devicePixelRatio}x, ${document.body.querySelectorAll("*").length} elements, zoom ${getComputedStyle(root).zoom})`;
    if (self.__oneNative?.log) self.__oneNative.log(note);
    else console.error(note);
    // A page that was only slow takes the box away once it has drawn.
    const watching = new MutationObserver(() => drawn() && (box.remove(), watching.disconnect()));
    watching.observe(document.body, { childList: true, subtree: true });
  };
  setTimeout(blank, 6000);
  const meta = (name) => document.querySelector(`meta[name="${name}"]`)?.content || "";
  const client = meta("one-client");
  const named = meta("one-files").split(" ").find((f) => f.startsWith("net-worker.js="));
  if (!client || !named) return;
  self.oneNet = { url: `/net-worker.js?v=${named.slice("net-worker.js=".length)}&client=${encodeURIComponent(client)}` };
  // The client's WebAssembly (and its glue) is asked for now, not once the worker runs: the worker's own request, when it
  // gets there, is answered by this one as it arrives, from the browser's cache (the files are named by their build).
  // Before anything else the page asks for (the icons wait: web.js), as the first answer waits for this.
  for (const file of ["one_client_bg.wasm", "one_client.js"]) fetch(`/client/${client}/${file}`, { priority: "high" }).then((r) => r.blob()).catch(() => {});
  if (typeof SharedWorker !== "function") return;
  const worker = (self.oneNet.worker = new SharedWorker(self.oneNet.url, { type: "module", name: "one" }));
  try {
    const access = JSON.parse(localStorage.getItem("one.signin") || "null")?.access;
    const user = JSON.parse(atob(access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub;
    if (user) worker.port.postMessage({ op: "start", user, id: "start" });
  } catch {}
})();
