// The bridge every app screen gets as `one`. In a sandboxed frame (the web client's, the app's window, a chat's), the
// page holding the frame answers its messages; a screen never knows which. The web client's own page loads it too,
// answering it itself (`__one.host`).
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  // A page that answers its own (`__one.host`: the web client's, a chat's view) isn't framed, even in a chat's frame.
  const framed = !ctx.host && window.parent !== window;

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
    // A still of the screen as it looks, a second after it last changed, for the page holding the frame to show
    // while this screen next opens: its page without its scripts, at its scroll.
    let still = 0;
    const snap = () => {
      const page = document.documentElement.cloneNode(true);
      page.querySelectorAll('script, meta[http-equiv="Content-Security-Policy"]').forEach((s) => s.remove());
      if (scrollY) page.dataset.stillY = Math.round(scrollY);
      window.parent.postMessage({ oneStill: `<!doctype html>${page.outerHTML}` }, "*");
    };
    const later = () => (clearTimeout(still), (still = setTimeout(snap, 1000)));
    // How tall its content is, for a page that fits a window to it (the native app's menu bar panel).
    let tall = 0;
    const measure = () => {
      const now = Math.ceil(document.body?.scrollHeight || 0);
      if (now !== tall) window.parent.postMessage({ oneHeight: (tall = now) }, "*");
    };
    addEventListener("load", () => {
      if (window.ResizeObserver && document.body) new ResizeObserver(measure).observe(document.body);
      new MutationObserver(later).observe(document.documentElement, { childList: true, subtree: true, characterData: true, attributes: true });
      addEventListener("scroll", later, { passive: true });
      later();
    });
    // Links leave the frame for a tab of their own, as the app sends them to the browser.
    document.addEventListener("click", (e) => {
      const a = e.target.closest?.("a[href]");
      if (!a || e.defaultPrevented || a.hasAttribute("download") || /^(#|javascript:)/i.test(a.getAttribute("href"))) return;
      e.preventDefault();
      host({ type: "link", url: a.href }).catch(() => {});
    });
  }

  // Every message a screen sends its host, and every host (the web client, a chat's view) answers each,
  // or refuses it by name. Anything a person touches natively is one of these: menus,
  // alerts, sheets, files, scanning, links.
  const messages = ["call", "open", "close", "title", "menu", "alert", "save", "preview", "copy", "link", "scan", "vault", "request",
    "icons", "haptic", "api", "signOut", "appearance", "device", "notify", "cache", "kept", "traces", "developer", "menubar"];

  async function host(message) {
    if (!messages.includes(message.type)) throw new Error(`No such message: ${message.type}`);
    if (ctx.host) return ctx.host(message);
    if (framed) {
      return new Promise((ok, fail) => {
        const id = ++asked;
        waiting[id] = { ok, fail };
        window.parent.postMessage({ one: message, id }, "*");
      });
    }
    // Nothing else answers: the node refuses browsers, which would run a screen as this machine's own programs.
    throw new Error("Open this in the app.");
  }

  // Icons are Phosphor's (vendor/phosphor), the same in every client: each a path on a 256-point grid, filled in the
  // text's colour. The host has them all; a screen draws `<svg data-icon="<name>">` (@one/ui's Icon), and the bridge
  // asks it for the ones drawn, once each, and fills them in.
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
  // An icon drawn before its path came.
  function fill(el) {
    const d = paths[el.dataset.icon];
    if (d) el.innerHTML = `<path d="${d}"/>`;
    else if (d === undefined) want(el.dataset.icon);
  }

  // What a call answered last time: the client keeps the last answer of each call that only reads (`reads: true`),
  // sealed on its device for its person, and gives it (`kept`, with when it was answered) to a screen that asks that
  // call again. The first time a screen asks a call, it's answered at once with what was kept, while the call goes out
  // as ever; if its answer differs, the screen is told its app changed, as by a node, and asking again gives it.
  // Asked again after that, a call goes out, so nothing a screen does after a write is answered from before it. A host
  // that keeps nothing (an older one, a chat's) refuses `kept`, and every call goes out.
  const first = new Set();
  const landed = new Map();
  // Calls answered from what was kept a while ago, by their key, while their answer is still out: what the note says.
  const old = new Map();
  async function call(tool, input) {
    const m = { type: "call", app: ctx.app, op: tool, input };
    const key = JSON.stringify([tool, input]);
    const got = landed.get(key);
    landed.clear();
    if (got && Date.now() - got.at < 10_000) return got.answer;
    const sent = () => host(m).then((answer) => (old.delete(key) && note(), answer));
    if (first.has(key)) return sent();
    first.add(key);
    const kept = await host({ type: "kept", app: ctx.app, op: tool, input }).catch(() => null);
    if (!kept || !("answer" in kept)) return sent();
    const fresh = sent();
    if (Date.now() - kept.at > 60_000) {
      old.set(key, kept.at);
      setTimeout(note, 1500);
    }
    fresh.then((answer) => {
      if (JSON.stringify(answer) === JSON.stringify(kept.answer)) return;
      landed.set(key, { answer, at: Date.now() });
      handlers.changed.forEach((f) => f({ type: "changed", apps: [ctx.app] }));
    }, () => {});
    return kept.answer;
  }
  // How old what shows is, while its fresh answer is late: one grey line at the top, gone once it's in.
  function note() {
    let el = document.getElementById("one-kept");
    if (!old.size) return el?.remove();
    if (!document.body) return;
    if (!el) document.body.prepend((el = Object.assign(document.createElement("div"), { id: "one-kept", className: "kept-note" })));
    el.textContent = `As of ${one.ago(Math.min(...old.values()), { long: true })}`;
  }

  // Errors and statuses are written for people and agents at once: what happened and what to do, in people's words,
  // then after a colon or in brackets what an agent needs (a tool to call, an id, a code). A sentence with any of
  // those is for agents only.
  const forAgents = /\b[a-z][a-z0-9-]*_[a-z0-9_]+\b|\b(user|group):[0-9a-f]{6,}|\b(kernel|node|deploy|scope|token|provider|workerd|bucket|lease|ctx)s?\b|\b(an|the|this|that) install\b|network's|[\w.-]*\/[\w.-]+\/|\b[1-5]\d\d\b/i;

  // Whether a screen has asked for a large title (`largeTitle`): it sets up the bar's title once.
  let titled = false;

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, me: ctx.me,
    // The product: its `name` and `cli`, and `made_here`, what an App Store calls the apps made in the network.
    config: ctx.config,
    // The page the window opened this from ("user", "group:<id>", "global"), or null: a hint, which the app may follow.
    scope: ctx.scope ?? null,
    call: (tool, input = {}) => call(tool, input),
    // Opens one of this app's screens; or with `app`, another app's (a notification's link, say). `scope` is the page
    // it's opened for (else this screen's). With `sheet`, it slides up over this one, and this resolves once it's gone,
    // to what it gave `close`; otherwise it's pushed on top, with Back.
    open: (screen, params = {}, app, { scope, sheet } = {}) =>
      host({ type: "open", screen, params, ...(app && app !== ctx.app ? { app } : {}), ...(scope ? { scope } : {}), ...(sheet ? { sheet: true } : {}) }),
    // Closes this screen: a sheet goes, giving `value` to who opened it; a pushed screen goes back.
    close: (value = null) => host({ type: "close", value }),
    // The window's own menu, at `at` ({ x, y } in this page, or the element it's for), of `items`, each
    // `{ title, danger, checked, icon, row, section }` → the index of the one chosen, or null. `icon` is a Phosphor
    // icon's name, which the window draws, or shows as its own symbol for it; items with `row`, first, are choices side by
    // side (a widget's sizes), shown by their icons; `section` starts a group of its own. A menu for an element comes
    // from it (`rect`, its bounds), opening away from the edge it's near; a host without that opens at the point:
    // the element's bottom middle, or its top in the lower half of the page.
    menu: (items, at) => {
      let rect;
      if (at instanceof Element) {
        const r = at.getBoundingClientRect();
        rect = [r.left, r.top, r.width, r.height].map(Math.round);
        at = { x: (r.left + r.right) / 2, y: r.top > innerHeight / 2 ? r.top : r.bottom };
      }
      const item = (i) => ({ title: String(i.title), danger: !!i.danger, checked: !!i.checked, ...(i.icon ? { icon: String(i.icon) } : {}), ...(i.row ? { row: true } : {}), ...(i.section ? { section: true } : {}) });
      return host({ type: "menu", items: items.map(item), x: Math.round(at?.x || 0), y: Math.round(at?.y || 0), ...(rect ? { rect } : {}) });
    },
    // The window's own alert: `title` and `text`, then OK; with `action` (its button's name; `danger` if it can't be
    // undone), Cancel and that → whether it was chosen.
    alert: ({ title = "", text = "", action, danger = false }) => host({ type: "alert", title, text, ...(action ? { action, danger } : {}) }).then((ok) => !!ok),
    // Opens a web page's address in the browser.
    link: (url) => host({ type: "link", url: String(url) }),
    // The bar's title; empty while the screen shows its own heading, when `name` is still what the screen is called,
    // for the window's path, Back and window name.
    title: (text, name) => host({ type: "title", text, ...(name ? { name: String(name) } : {}) }),
    // What a failure means for someone: what happened, in plain words, whether anything was lost, and what to do next.
    // `app` is the title of the app that failed, where a screen knows it.
    plain: (err, { app } = {}) => {
      let msg = String(err?.message ?? err ?? "").replace(/^(Uncaught )?\w*Error:\s*/, "").trim();
      while (/\([^()]*\)/.test(msg)) msg = msg.replace(/\s*\([^()]*\)/g, "");
      if (/failed to fetch|load failed|networkerror|not connected|offline|unreachable|no route|connection (refused|reset|lost)/i.test(msg))
        return { title: "Couldn't reach your devices.", safe: "Nothing was lost.", next: "Check your connection." };
      if (/timed? ?out|took too long|took longer than/i.test(msg))
        return { title: app ? `${app} took too long, so it stopped.` : "That took too long, so it stopped.", safe: "Nothing was lost.", next: "" };
      if (/may already be done/i.test(msg)) return { title: "This may already be done.", safe: "", next: "Check before trying again." };
      // The leading sentences before any colon that are for people; none if the first is for agents.
      const parts = [];
      for (const part of msg.split(/:\s/)[0].split(/(?<=[.!?])\s+|;\s+/)) {
        if (!part || forAgents.test(part)) break;
        parts.push(part);
      }
      const said = parts.map((s) => (/^\S*@/.test(s) ? s : s[0].toUpperCase() + s.slice(1)).replace(/([^.!?])$/, "$1.")).join(" ");
      if (!said || said.length > 200 || /something went wrong|internal (server )?error|unexpected|undefined|null|is not a function|\[object/i.test(said))
        return { title: "That didn't work.", safe: "Nothing was lost.", next: "" };
      // Something that isn't there won't be there on a second try: say so, and offer the way home instead.
      const missing = /\b(not found|no such|no \w+ named|doesn't exist|isn't here|there's no)\b/i.test(msg);
      return { title: said, safe: "Nothing was lost.", next: "", missing };
    },
    // A large title at the top of a screen, as iOS draws one: the window's own title stays empty while it shows, and is
    // its text once it scrolls away, so the name shows once. The screen's `.large-title` is followed as it redraws, so
    // calling it once is enough; put what it returns first, or draw a `.large-title` of its own.
    largeTitle: (text) => {
      const follows = !!window.IntersectionObserver;
      if (!titled) {
        titled = true;
        one.title(follows ? "" : text, text);
        if (follows) {
          let el = null;
          const seen = new IntersectionObserver((all) => {
            const e = all.filter((x) => x.target === el).pop();
            if (e) one.title(e.isIntersecting ? "" : el.textContent, el.textContent);
          });
          const follow = () => {
            const now = document.querySelector(".large-title");
            if (now === el) return;
            if (el) seen.unobserve(el);
            if ((el = now)) seen.observe(el);
          };
          setTimeout(follow);
          new MutationObserver(follow).observe(document.documentElement, { childList: true, subtree: true });
        }
      }
      return `<h1 class="large-title">${one.esc(text)}</h1>`;
    },
    // Files, as `{ name, type, base64 }` or `{ name, type, blob }`: save (or share) one, or show it (Quick Look,
    // Preview, a tab).
    save: (file) => host({ type: "save", file }),
    preview: (file) => host({ type: "preview", file }),
    // The document camera, where the device has one (one.can.scan): each page found, straightened and cropped, as a
    // JPEG `{ name, type, size, base64 }`; none if the person cancels.
    scan: () => host({ type: "scan" }),
    // Asks the person for what this app's `vault` declares as `need`: they pick one of their items or make one, in a
    // sheet of the window's. True once it's allowed; false if they declined or closed it.
    vault: (need) => host({ type: "vault", need }),
    // What this place can do beyond the basics.
    // (Asked as it's read: the page that holds this one may say so after this runs.)
    can: { get scan() { return !!window.__oneCan?.scan; } },
    // Copies text: through the host (which a frame's clipboard can't always reach), else the browser's clipboard.
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
    tap: (style = "selection") => (framed ? host({ type: "haptic", style }) : Promise.resolve()).catch(() => {}),
    on: (event, fn) => (handlers[event] ||= []).push(fn),
    // The host calls this with node events: an app screen's "changed" is its app's data changing; a Settings
    // screen's, any event, since what it shows is the network's.
    __event(e) {
      if (!ctx.app || (e.type === "changed" && (e.apps || []).includes(ctx.app))) landed.clear(), handlers.changed.forEach((f) => f(e));
    },
    // For Settings screens only (no app's: a client answers these for its own Settings screens and refuses an app
    // screen). `api` asks the node's /v1 API as the person looking ("POST", "/v1/group", { name }) and gives its JSON,
    // failing with its `error`. `signOut` signs this client out. `appearance` gives this client's ("automatic",
    // "light" or "dark"), first setting it to `value` if given. `device` is this client:
    // `{ name ("mac", "Chrome on Mac"), signOut (whether it can sign out), notify? }`, `notify` a browser's notifications
    // permission ("default", "granted" or "denied"), which `notify` asks the person for and then gets pushes with.
    api: (method, path, body) => host({ type: "api", method, path, body }),
    // Opens the client's own Vault sheet for a request already made (as an agent's link does): true once it's
    // answered with an item, false if declined or closed.
    request: (scope, id) => host({ type: "request", scope, id }),
    signOut: () => host({ type: "signOut" }),
    // What this client keeps for its own screens between launches, by `key`: with `value` (JSON), keeps it; without,
    // gives what was kept, or null. Forgotten when it signs out.
    cache: (key, value) => host(value === undefined ? { type: "cache", key: String(key) } : { type: "cache", key: String(key), value }),
    appearance: (value) => host({ type: "appearance", value }),
    // Developer mode, on this client alone: whether it's on, first turning it on or off if `on` is given. Settings'.
    developer: (on) => host(on === undefined ? { type: "developer" } : { type: "developer", on: !!on }),
    // In developer mode, this client's latest requests, each with how long it took and where it went (Developer's
    // panel, src/screen/settings/devtools.jsx, draws them): null out of developer mode. A host that doesn't know it refuses it, as one out of it.
    traces: () => host({ type: "traces" }).catch(() => null),
    device: () => host({ type: "device" }),
    notify: () => host({ type: "notify" }),
    // A Mac's widget (`<app>.<widget>`) shown in its menu bar again (`show`), or hidden: the menu bar panel's.
    // → the widgets hidden there now.
    menubar: (kind, show) => host({ type: "menubar", kind: String(kind), show: !!show }),
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
  };

  new MutationObserver(() => document.querySelectorAll("svg[data-icon]:empty").forEach(fill))
    .observe(document.documentElement, { childList: true, subtree: true });
})();
