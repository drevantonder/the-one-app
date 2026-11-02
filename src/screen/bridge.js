// The bridge every app screen gets as `one`. Inside a shell, the shell answers it (WKWebView's
// `one` message handler, WebView2's `chrome.webview`); in a sandboxed frame (the web client's, a chat's), the page
// holding the frame answers its messages; in a plain browser on the device it talks to the node directly. A screen
// never knows which. The web client's own page loads it too, for the kit, answering it itself (`__one.host`).
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  const shell = window.webkit?.messageHandlers?.one;
  const framed = !shell && window.parent !== window;

  // Framed: each message goes up as `{ one: message, id }`, answered with `{ oneReply: id, ok }` or `{ oneReply: id,
  // error }`; node events come down as `{ oneEvent: event }`. Only the page holding the frame is listened to.
  let asked = 0;
  const waiting = {};
  if (framed) {
    addEventListener("message", (e) => {
      const m = e.source === window.parent && e.data;
      if (!m) return;
      if (m.oneEvent) one.__event(m.oneEvent);
      const w = waiting[m.oneReply];
      if (!w) return;
      delete waiting[m.oneReply];
      m.error === undefined ? w.ok(m.ok) : w.fail(new Error(m.error));
    });
    // Links leave the frame for a tab of their own, as a shell sends them to the browser.
    document.addEventListener("click", (e) => {
      const a = e.target.closest?.("a[href]");
      if (!a || e.defaultPrevented || a.hasAttribute("download") || /^(#|javascript:)/i.test(a.getAttribute("href"))) return;
      e.preventDefault();
      host({ type: "link", url: a.href }).catch(() => {});
    });
  }

  async function host(message) {
    if (shell) return shell.postMessage(message);
    if (ctx.host) return ctx.host(message);
    if (framed) {
      return new Promise((ok, fail) => {
        const id = ++asked;
        waiting[id] = { ok, fail };
        window.parent.postMessage({ one: message, id }, "*");
      });
    }
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
    if (message.type === "vault") throw new Error("Open this in the app to choose it.");
    if (message.type === "copy") return navigator.clipboard.writeText(message.text);
  }

  // Browser only: the node's events, to refresh when this app's data changes.
  if (!shell && !framed && !ctx.host && window.EventSource) {
    new EventSource("/v1/events").onmessage = (m) => one.__event(JSON.parse(m.data));
  }

  // Outside a shell, which draws apps' SF Symbols itself, the symbols the built-in apps use, drawn on a 24-point grid.
  const glyphs = {
    "square.grid.2x2": '<rect x="4" y="4" width="7" height="7" rx="1.8"/><rect x="13" y="4" width="7" height="7" rx="1.8"/><rect x="4" y="13" width="7" height="7" rx="1.8"/><rect x="13" y="13" width="7" height="7" rx="1.8"/>',
    "text.bubble": '<path d="M4 6.5A2.5 2.5 0 0 1 6.5 4h11A2.5 2.5 0 0 1 20 6.5v8a2.5 2.5 0 0 1-2.5 2.5H11l-4.5 3.5V17A2.5 2.5 0 0 1 4 14.5z"/><path d="M8 8.5h8M8 12.5h5"/>',
    calendar: '<rect x="3.5" y="5" width="17" height="15.5" rx="2.5"/><path d="M3.5 10h17M8 3v4M16 3v4"/>',
    "cursorarrow.rays": '<path d="M10.5 10.5l9 3.6-4 1.5-1.5 4z"/><path d="M7 3v3M3 7h3M4 4l1.8 1.8M10.5 4.5L9 6M4.5 10.5L6 9"/>',
    "person.crop.circle": '<circle cx="12" cy="12" r="9"/><circle cx="12" cy="10" r="3.2"/><path d="M6.3 18.2a6.6 6.6 0 0 1 11.4 0"/>',
    "macwindow.on.rectangle": '<rect x="7.5" y="8" width="13" height="11" rx="2"/><path d="M7.5 11.5h13M16.5 8V6a2 2 0 0 0-2-2h-9a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h2"/>',
    desktopcomputer: '<rect x="3" y="4" width="18" height="12.5" rx="2"/><path d="M9 20.5h6M12 16.5v4"/>',
    "doc.text": '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M9 13h6M9 17h6"/>',
    externaldrive: '<rect x="3" y="7" width="18" height="10" rx="2.5"/><path d="M16.5 12h.01M3.5 12h9"/>',
    envelope: '<rect x="3" y="5.5" width="18" height="13" rx="2.2"/><path d="M3.8 7l8.2 6.3L20.2 7"/>',
    "message.fill": '<path fill="currentColor" stroke="none" d="M12 3.8c-5 0-9 3.4-9 7.6 0 2.3 1.2 4.3 3 5.7l-.7 3.4 3.8-2c.9.3 1.9.4 2.9.4 5 0 9-3.4 9-7.5S17 3.8 12 3.8z"/>',
    "phone.bubble.fill": '<path fill="currentColor" stroke="none" d="M12 3.8c-5 0-9 3.4-9 7.6 0 2.3 1.2 4.3 3 5.7l-.7 3.4 3.8-2c.9.3 1.9.4 2.9.4 5 0 9-3.4 9-7.5S17 3.8 12 3.8z"/>',
    cpu: '<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9.5" y="9.5" width="5" height="5" rx="1"/><path d="M9.5 3v3M14.5 3v3M9.5 18v3M14.5 18v3M3 9.5h3M3 14.5h3M18 9.5h3M18 14.5h3"/>',
    checklist: '<path d="M4 6.5l1.5 1.5L8.5 5M4 12.5l1.5 1.5 3-3M4 18.5L5.5 20l3-3M12 6.5h8M12 12.5h8M12 18.5h8"/>',
    "arrow.triangle.branch": '<path d="M7 20V9a4 4 0 0 1 4-4h6M14 2l3 3-3 3M7 13a4 4 0 0 0 4 4h6M14 14l3 3-3 3"/>',
    tablecells: '<rect x="3.5" y="4.5" width="17" height="15" rx="2"/><path d="M3.5 9.5h17M3.5 14.5h17M12 4.5v15"/>',
    terminal: '<rect x="3" y="4.5" width="18" height="15" rx="2.5"/><path d="M7 9.5l3 2.5-3 2.5M12.5 15H17"/>',
    sparkles: '<path d="M10 3.5l1.7 4.3 4.3 1.7-4.3 1.7L10 15.5l-1.7-4.3L4 9.5l4.3-1.7z"/><path d="M17.5 13.5l.9 2.1 2.1.9-2.1.9-.9 2.1-.9-2.1-2.1-.9 2.1-.9z"/>',
    brain: '<path d="M12 5.2a3 3 0 0 0-5.6-1A3 3 0 0 0 4 8.4a3.2 3.2 0 0 0 .3 4.8A3.4 3.4 0 0 0 8 18.6a3 3 0 0 0 4 1.4zM12 5.2a3 3 0 0 1 5.6-1A3 3 0 0 1 20 8.4a3.2 3.2 0 0 1-.3 4.8 3.4 3.4 0 0 1-3.7 5.4 3 3 0 0 1-4 1.4"/>',
    "puzzlepiece.extension": '<path d="M5 8.5h3.2a2.3 2.3 0 1 1 4.6 0H16v3.2a2.3 2.3 0 1 1 0 4.6V20H5z"/>',
  };

  const colors = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"];

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, me: ctx.me,
    // The product: its `name` and `cli`, and `made_here`, what an App Store calls the apps made in the install (its setting).
    config: ctx.config,
    // The same, for apps written before it was `config`.
    brand: ctx.config,
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
    // Asks the person for what this app's `vault` declares as `need`: they pick one of their items or make one, in a
    // sheet of the shell's. True once it's allowed; false if they declined or closed it.
    vault: (need) => host({ type: "vault", need }),
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
    // An app's icon, `{ name, title, icon, color, image, image_kind }` as catalog() gives it: the shells' tile, drawn in
    // by the shell; elsewhere its picture, or its first letter on its colour. `image_kind` "icon" is a finished app icon,
    // drawn edge to edge; otherwise the picture is a logo, inset on white.
    appIcon(app, size = 36) {
      const glyph = glyphs[app.icon];
      const face = glyph && !shell
        ? `<svg viewBox="0 0 24 24" width="${Math.round(size * 0.56)}" height="${Math.round(size * 0.56)}" fill="none" stroke="currentColor" stroke-width="1.9" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${glyph}</svg>`
        : one.esc(String(app.title || app.name || "?")[0].toUpperCase());
      const look = app.image && !shell ? `;background-image:url(${app.image})` : app.color ? `;--tile:${app.color}` : "";
      const logo = app.image && !shell && app.image_kind !== "icon" ? " logo" : "";
      return `<span class="app-icon${look.includes("url(") ? " drawn" : ""}${logo}" style="--size:${size}px${one.esc(look)}" data-symbol="${one.esc(app.icon || "")}" data-seed="${one.esc(app.name)}" data-color="${one.esc(app.color || "")}" data-image="${one.esc(app.image || "")}" data-kind="${one.esc(app.image_kind || "")}" data-size="${size}">${face}</span>`;
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
      const { symbol, seed, color, image, kind, size } = el.dataset;
      const key = `${symbol}|${seed}|${color}|${image.length}|${kind}|${size}`;
      (drawn[key] ||= host({ type: "icon", symbol, seed, color, image, kind, size: Number(size) }).catch(() => null)).then((url) => {
        if (url) el.style.backgroundImage = `url(${url})`, el.classList.add("drawn");
      });
    };
    new MutationObserver(() => document.querySelectorAll(".app-icon:not([data-asked])").forEach((el) => (el.dataset.asked = "1", draw(el))))
      .observe(document.documentElement, { childList: true, subtree: true });
  }
})();
