// The bridge every app screen gets as `one`. Inside a shell, the shell answers it (WKWebView's
// `one` message handler, or `__oneShell.postMessage` where another web view puts one, as Android's and Windows' do); in a sandboxed frame (the web client's, a chat's), the page
// holding the frame answers its messages; in a plain browser on the device it talks to the node directly. A screen
// never knows which. The web client's own page loads it too, for the kit, answering it itself (`__one.host`).
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  // Either answers each message with a promise of its reply.
  const shell = window.webkit?.messageHandlers?.one || (window.__oneShell?.postMessage ? window.__oneShell : null);
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
      // A Settings screen opens Settings screens (another, or itself with other params).
      location.href = ctx.app || message.app ? `/v1/screens/${message.app || ctx.app}/${message.screen}?${q}` : `/v1/settings-screens/${message.screen}?${q}`;
    }
    if (message.type === "api") {
      const r = await fetch(message.path, {
        method: message.method,
        headers: { "content-type": "application/json" },
        body: message.method === "GET" || message.body === undefined ? undefined : JSON.stringify(message.body),
      });
      const body = await r.json();
      if (body?.error) throw new Error(body.error);
      return body;
    }
    // A browser on the device is no sign-in, and follows the system's appearance.
    if (message.type === "signOut") throw new Error("Sign out in the app.");
    if (message.type === "appearance") return "automatic";
    if (message.type === "device") return { name: "This browser", build: null, signOut: false };
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
    if (message.type === "scan") throw new Error("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.");
    if (message.type === "vault" || message.type === "request") throw new Error("Open this in the app to choose it.");
    if (message.type === "copy") return navigator.clipboard.writeText(message.text);
    if (message.type === "icons") {
      icons ||= fetch("/v1/icons").then((r) => r.json());
      const all = await icons;
      return Object.fromEntries(message.names.map((n) => [n, all[n]]));
    }
  }
  let icons;

  // Browser only: the node's events, to refresh when this app's data changes.
  if (!shell && !framed && !ctx.host && window.EventSource) {
    new EventSource("/v1/events").onmessage = (m) => one.__event(JSON.parse(m.data));
  }

  // Icons are Phosphor's (vendor/phosphor), the same in every shell: each a path on a 256-point grid, filled in the
  // text's colour. The host has them all; a screen asks it for the ones it draws, once each, and fills them in.
  const paths = {};
  let wanted = null;
  function want(name) {
    if (name in paths || wanted?.has(name)) return;
    if (!wanted) {
      wanted = new Set();
      queueMicrotask(async () => {
        const names = [...wanted];
        const got = await host({ type: "icons", names }).catch(() => null);
        wanted = null;
        // Unanswered (signed out, say), they're asked for again when next drawn.
        if (!got) return;
        for (const n of names) paths[n] = got[n] || "";
        document.querySelectorAll("svg[data-icon]:empty").forEach(fill);
      });
    }
    wanted.add(name);
  }
  // An icon drawn before its path came; one that isn't in the set gives way to its `or`, if it has one.
  function fill(el) {
    const d = paths[el.dataset.icon];
    if (d) el.innerHTML = `<path d="${d}"/>`;
    else if (d === "" && el.dataset.or) el.replaceWith(el.dataset.or);
    else if (d === undefined) want(el.dataset.icon);
  }
  function glyph(name, cls, or) {
    if (!paths[name]) want(name);
    const d = paths[name];
    if (d === "" && or) return one.esc(or);
    return `<svg class="${cls}" viewBox="0 0 256 256" fill="currentColor" aria-hidden="true" data-icon="${one.esc(name)}"${or ? ` data-or="${one.esc(or)}"` : ""}>${d ? `<path d="${d}"/>` : ""}</svg>`;
  }

  const colors = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"];

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, me: ctx.me,
    // The product: its `name` and `cli`, and `made_here`, what an App Store calls the apps made in the install (its setting).
    config: ctx.config,
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
    // The shell calls this with node events: an app screen's "changed" is its app's data changing; a Settings
    // screen's, any event, since what it shows is the install's.
    __event(e) {
      if (!ctx.app || (e.type === "changed" && (e.apps || []).includes(ctx.app))) handlers.changed.forEach((f) => f(e));
    },
    // For Settings screens only (no app's: a client answers these for its own Settings screens and refuses an app
    // screen). `api` asks the node's /v1 API as the person looking ("POST", "/v1/group", { name }) and gives its JSON,
    // failing with its `error`. `signOut` signs this client out. `appearance` gives this client's ("automatic",
    // "light" or "dark"), first setting it to `value` if given. `device` is this client:
    // `{ name ("mac", "Chrome on Mac"), build, signOut (whether it can sign out) }`.
    api: (method, path, body) => host({ type: "api", method, path, body }),
    // Opens the client's own Vault sheet for a request already made (as an agent's link does): true once it's
    // answered with an item, false if declined or closed.
    request: (scope, id) => host({ type: "request", scope, id }),
    signOut: () => host({ type: "signOut" }),
    appearance: (value) => host({ type: "appearance", value }),
    device: () => host({ type: "device" }),
    // A Phosphor icon by its name ("magnifying-glass", "check-circle-fill"), at the text's size and colour.
    icon: (name, cls = "") => glyph(name, `i ${cls}`.trim()),
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
    // by the shell; elsewhere its picture, or the filled one of its icon (else its first letter) on its colour.
    // `image_kind` "icon" is a finished app icon, drawn edge to edge; otherwise the picture is a logo, inset on white.
    appIcon(app, size = 36) {
      const letter = String(app.title || app.name || "?")[0].toUpperCase();
      const face = app.icon && !shell ? glyph(`${app.icon}-fill`, "", letter).replace("<svg ", `<svg width="${Math.round(size * 0.5)}" height="${Math.round(size * 0.5)}" `) : one.esc(letter);
      const look = app.image && !shell ? `;background-image:url(${app.image})` : app.color ? `;--tile:${app.color}` : "";
      const logo = app.image && !shell && app.image_kind !== "icon" ? " logo" : "";
      return `<span class="app-icon${look.includes("url(") ? " drawn" : ""}${logo}" style="--size:${size}px${one.esc(look)}" data-icon="${one.esc(app.icon || "")}" data-seed="${one.esc(app.name)}" data-color="${one.esc(app.color || "")}" data-image="${one.esc(app.image || "")}" data-kind="${one.esc(app.image_kind || "")}" data-size="${size}">${face}</span>`;
    },
    // A steady colour and initials for someone, from their id and name.
    avatar(id, name) {
      let h = 0;
      for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0;
      const initials = String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
      return `<div class="avatar" style="background:${colors[h % colors.length]}">${one.esc(initials)}</div>`;
    },
  };

  new MutationObserver(() => document.querySelectorAll("svg[data-icon]:empty").forEach(fill))
    .observe(document.documentElement, { childList: true, subtree: true });

  // Inside a shell, app icons are drawn by it, once per icon, seed and size.
  if (shell) {
    const drawn = {};
    const draw = (el) => {
      const { icon, seed, color, image, kind, size } = el.dataset;
      const key = `${icon}|${seed}|${color}|${image.length}|${kind}|${size}`;
      (drawn[key] ||= host({ type: "icon", icon, seed, color, image, kind, size: Number(size) }).catch(() => null)).then((url) => {
        if (url) el.style.backgroundImage = `url(${url})`, el.classList.add("drawn");
      });
    };
    new MutationObserver(() => document.querySelectorAll(".app-icon:not([data-asked])").forEach((el) => (el.dataset.asked = "1", draw(el))))
      .observe(document.documentElement, { childList: true, subtree: true });
  }
})();
