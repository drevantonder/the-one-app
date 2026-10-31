// The bridge every app screen gets as `one`. Inside a shell, the shell answers it (WKWebView's
// `one` message handler, WebView2's `chrome.webview`); in a plain browser on the device it talks
// to the node directly. A screen never knows which.
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  const shell = window.webkit?.messageHandlers?.one;

  async function host(message) {
    if (shell) return shell.postMessage(message);
    if (message.type === "call") {
      const r = await fetch("/v1/call", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ app: message.app, op: message.op, input: message.input }),
      });
      const body = await r.json();
      if (body.error) throw new Error(body.error);
      return body.ok;
    }
    if (message.type === "open") {
      const q = new URLSearchParams({ params: JSON.stringify(message.params || {}) });
      if (ctx.scope) q.set("scope", ctx.scope);
      location.href = `/v1/screens/${message.app || ctx.app}/${message.screen}?${q}`;
    }
    if (message.type === "title") document.title = message.text;
    // A browser saves a file as a download, and shows one in a tab of its own.
    if (message.type === "save" || message.type === "preview") {
      const f = message.file || {};
      const url = f.base64 ? `data:${f.type || "application/octet-stream"};base64,${f.base64}` : `/v1/blobs/${f.blob || f.file}`;
      if (message.type === "preview") return void window.open(url, "_blank");
      const a = Object.assign(document.createElement("a"), { href: url, download: f.name || "file" });
      document.body.append(a);
      a.click();
      a.remove();
    }
    if (message.type === "scan") throw new Error("Scanning documents needs the iPhone or iPad app. Choose the files instead.");
    if (message.type === "copy") return navigator.clipboard.writeText(message.text);
  }

  // Browser only: the node's events, to refresh when this app's data changes.
  if (!shell && window.EventSource) {
    new EventSource("/v1/events").onmessage = (m) => one.__event(JSON.parse(m.data));
  }

  const colors = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"];

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, me: ctx.me,
    // The product: its `name`, and `made_here`, what an App Store calls the apps made in the install.
    brand: ctx.brand,
    // The page the shell opened this from ("user", "group:<id>", "global"), or null: a hint, which the app may follow.
    scope: ctx.scope ?? null,
    call: (op, input = {}) => host({ type: "call", app: ctx.app, op, input }),
    // Opens one of this app's screens; or with `app`, another app's (a notification's link, say).
    open: (screen, params = {}, app) => host({ type: "open", screen, params, ...(app && app !== ctx.app ? { app } : {}) }),
    title: (text) => host({ type: "title", text }),
    // Files, as `{ name, type, base64 }` or `{ name, type, blob }`: save (or share) one, or show it (Quick Look,
    // Preview, a tab).
    save: (file) => host({ type: "save", file }),
    preview: (file) => host({ type: "preview", file }),
    // The document camera, where the shell has one (one.can.scan): each page found, straightened and cropped, as a
    // JPEG `{ name, type, size, base64 }`; none if the person cancels.
    scan: () => host({ type: "scan" }),
    // What this place can do beyond the basics.
    can: { scan: !!window.__oneShell?.scan },
    // Copies text: through the shell (which a page's clipboard can't always reach), else the browser's clipboard.
    async copy(text) {
      try {
        await host({ type: "copy", text: String(text) });
      } catch {
        const t = Object.assign(document.createElement("textarea"), { value: String(text) });
        document.body.append(t);
        t.select();
        document.execCommand("copy");
        t.remove();
      }
    },
    // A haptic on a phone: "selection" (a press), "medium" (a long press), "success" or "error". Elsewhere, nothing.
    tap: (style = "selection") => shell?.postMessage({ type: "haptic", style }).catch(() => {}),
    on: (event, fn) => (handlers[event] ||= []).push(fn),
    // The shell calls this with node events.
    __event(e) {
      if (e.type === "changed" && (e.apps || []).includes(ctx.app)) handlers.changed.forEach((f) => f(e));
    },
    esc: (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]),
    // How long ago: short ("5 min", "Tue", "3 Oct") for tight rows, or with `{ long: true }` for where there's room
    // ("5 minutes ago", "3 days ago", "2 years ago").
    ago(ms, opts = {}) {
      const now = typeof opts === "number" ? opts : opts.now ?? Date.now();
      const s = Math.max(0, (now - ms) / 1000);
      if (opts.long) {
        const steps = [[60, "second"], [60, "minute"], [24, "hour"], [30.44, "day"], [12, "month"], [Infinity, "year"]];
        if (s < 45) return "just now";
        let n = s;
        for (const [size, unit] of steps) {
          if (n < size) {
            const k = Math.max(1, Math.round(n));
            if (k > 1) return `${k} ${unit}s ago`;
            return { minute: "a minute ago", hour: "an hour ago", day: "yesterday", month: "last month", year: "last year" }[unit];
          }
          n /= size;
        }
      }
      if (s < 60) return "now";
      if (s < 3600) return `${Math.floor(s / 60)} min`;
      if (s < 86400) return `${Math.floor(s / 3600)} h`;
      const d = new Date(ms);
      return s < 7 * 86400 ? d.toLocaleDateString(undefined, { weekday: "short" }) : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
    },
    // An app's icon, `{ name, title, icon, color, image }` as catalog() gives it: the shells' tile, drawn in by the
    // shell; elsewhere its picture, or its first letter on its colour.
    appIcon(app, size = 36) {
      const letter = String(app.title || app.name || "?")[0].toUpperCase();
      const look = app.image && !shell ? `;background-image:url(${app.image})` : app.color ? `;--tile:${app.color}` : "";
      return `<span class="app-icon${look.includes("url(") ? " drawn" : ""}" style="--size:${size}px${one.esc(look)}" data-symbol="${one.esc(app.icon || "")}" data-seed="${one.esc(app.name)}" data-color="${one.esc(app.color || "")}" data-image="${one.esc(app.image || "")}" data-size="${size}">${one.esc(letter)}</span>`;
    },
    // A steady colour and initials for someone, from their id and name.
    avatar(id, name) {
      let h = 0;
      for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0;
      const initials = String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
      return `<div class="avatar" style="background:${colors[h % colors.length]}">${one.esc(initials)}</div>`;
    },
  };

  // Inside a shell, app icons are drawn by it, once per symbol, seed and size.
  if (shell) {
    const drawn = {};
    const draw = (el) => {
      const { symbol, seed, color, image, size } = el.dataset;
      const key = `${symbol}|${seed}|${color}|${image.length}|${size}`;
      (drawn[key] ||= host({ type: "icon", symbol, seed, color, image, size: Number(size) }).catch(() => null)).then((url) => {
        if (url) el.style.backgroundImage = `url(${url})`, el.classList.add("drawn");
      });
    };
    new MutationObserver(() => document.querySelectorAll(".app-icon:not([data-asked])").forEach((el) => (el.dataset.asked = "1", draw(el))))
      .observe(document.documentElement, { childList: true, subtree: true });
  }
})();
