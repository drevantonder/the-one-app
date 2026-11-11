// The bridge every app screen gets as `one`. In a sandboxed frame (the web client's, the app's window, a chat's), the
// page holding the frame answers its messages; a screen never knows which. The web client's own page loads it too,
// answering it itself (`__one.host`).
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  // A page that answers its own (`__one.host`: the web client's, a chat's view) isn't framed, even in a chat's frame.
  const framed = !ctx.host && window.parent !== window;
  // The language a screen reader reads it in; the page this is in (screen.rs) names none.
  document.documentElement.lang ||= "en";

  // These are the page's, not @one/ui's, which may have run before this page was written (frame.html's `oneKit`).
  // iOS only shows :active (ui.css's press) where a touch is being listened for.
  document.addEventListener("touchstart", () => {}, { passive: true });
  // Windows' shell zooms its pages to 14/17 of the user's text scale, sizing the old kit's 17px text to its own (ui.css
  // undoes the others'); this kit is sized as it is, so it keeps only their scale.
  if (window.chrome?.webview) {
    document.addEventListener("DOMContentLoaded", () => {
      const zoom = parseFloat(document.documentElement.style.zoom);
      if (zoom) document.documentElement.style.zoom = String((zoom * 17) / 14);
    });
  }

  // Framed: each message goes up as `{ one: message, id }`, answered with `{ oneReply: id, ok }` or `{ oneReply: id,
  // error, code?, trace? }`; node events come down as `{ oneEvent: event }`. Only the page holding the frame is listened to.
  let asked = 0;
  // What's out to the user's devices, and whether the page holding the frame was told it all came (`answered`).
  let asking = 0, toldFresh = false;
  const waiting = {};
  // What had focus here, for the page holding the frame to give it back after a menu, alert or sheet it opened for this
  // screen: a frame's own focus goes when another element of that page takes it.
  let lastFocus = null;
  addEventListener("focusin", (e) => (lastFocus = e.target));
  if (framed) {
    addEventListener("message", (e) => {
      const m = e.source === window.parent && e.data;
      if (!m) return;
      if (m.oneEvent) one.__event(m.oneEvent);
      if (m.oneFocus && lastFocus?.isConnected) lastFocus.focus({ preventScroll: true });
      if (m.oneBar !== undefined) ((barStart = m.oneBar), placeLargeTitle("margin-inline-start var(--move-settle) var(--ease-settle)"));
      if (m.oneEscape) (document.activeElement || document.body).dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
      if (m.oneLeft) (handlers.left || []).forEach((f) => f());
      if (m.oneOffline !== undefined) {
        const back = windowSays && !m.oneOffline;
        windowSays = m.oneOffline;
        note();
        // What the screen shows may have changed while the window couldn't hear of it, and what failed can work now.
        if (back) (landed.clear(), handlers.changed.forEach((f) => f({ type: "changed", apps: [ctx.app] })));
      }
      const w = waiting[m.oneReply];
      if (!w) return;
      delete waiting[m.oneReply];
      m.error === undefined ? w.ok(m.ok) : w.fail(Object.assign(new Error(m.error), { code: m.code, trace: m.trace }));
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
    // A dialog of this screen's own over it, for the page holding the frame to dim what's around it to match; and how
    // many dialogs, menus and lists it has open, for Back to close them (the page counts its own alike).
    let dialog = false, open = 0;
    const dimming = () => {
      const now = !!document.querySelector("[data-slot=dialog-overlay], [data-slot=sheet-overlay], [data-slot=alert-dialog-overlay]");
      if (now !== dialog) window.parent.postMessage({ oneDimmed: (dialog = now) }, "*");
      const n = document.querySelectorAll("[role=dialog][data-state=open], [role=alertdialog][data-state=open], [role=menu][data-state=open], [role=listbox][data-state=open]").length;
      if (n !== open) window.parent.postMessage({ oneOver: (open = n) }, "*");
    };
    addEventListener("load", () => {
      loaded = true;
      window.parent.postMessage({ oneShown: true }, "*");
      settled();
      setTimeout(() => ((patient = false), settled()), 120);
      if (window.ResizeObserver && document.body) new ResizeObserver(measure).observe(document.body);
      new MutationObserver(() => (later(), settled(), dimming())).observe(document.documentElement, { childList: true, subtree: true, characterData: true, attributes: true });
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
  // or refuses it by name. Anything a user touches natively is one of these: menus,
  // alerts, sheets, files, scanning, links.
  const messages = ["call", "open", "opening", "close", "title", "menu", "alert", "save", "preview", "copy", "link", "scan", "vault", "request",
    "icons", "haptic", "api", "signOut", "appearance", "device", "notify", "cache", "kept", "thrown"];

  async function host(message) {
    if (!messages.includes(message.type)) throw new Error(`No such message: ${message.type}`);
    if (ctx.host) return ctx.host(message);
    if (framed) {
      // What asks the user's devices, counted until it's answered, for `answered`.
      const toDevices = message.type === "call" || message.type === "api";
      if (toDevices) asking++;
      const answer = new Promise((ok, fail) => {
        const id = ++asked;
        waiting[id] = { ok, fail };
        window.parent.postMessage({ one: message, id }, "*");
      });
      return toDevices ? answer.finally(() => (asking--, answered())) : answer;
    }
    // Nothing else answers: the node refuses browsers, which would run a screen as this machine's own programs.
    throw new Error("Open this in the app.");
  }

  // What goes wrong in a screen, for its host to report (`thrown`): what threw, where, and what it said, which the node
  // cleans before keeping. A few per screen, as one loop of errors is one problem.
  let threw = 0;
  function thrown(err) {
    if (threw++ < 10) host({ type: "thrown", exception: described(err) }).catch(() => {});
  }
  function described(err) {
    const stack = String(err?.stack ?? "");
    const message = String(err?.message ?? "");
    const after = message && stack.includes(message) ? stack.slice(stack.indexOf(message) + message.length) : stack;
    const frameOf = (line) => {
      const at = /(?:^\s*at .*?\(?|@)([^()\s@]+):(\d+):(\d+)\)?\s*$/.exec(line);
      return at && { at: at[1], file: at[1].split(/[?#]/)[0].split("/").pop().replace(/^about:|[<>]/g, ""), line: Number(at[2]), col: Number(at[3]) };
    };
    // Where the screen's module starts, which it says in its own first line (screens.mjs): the same in every browser.
    const screen = globalThis.__oneScreen;
    const first = String(screen?.at ?? "").split("\n").map(frameOf).find(Boolean);
    const frames = [];
    for (const line of after.split("\n")) {
      const at = frameOf(line);
      if (at && first && at.at === first.at && at.line >= first.line) frames.push({ file: `${screen.name}.screen.js`, line: at.line - first.line + 1, col: at.col });
      else if (at && /^[\w.-]{1,64}$/.test(at.file)) frames.push({ file: at.file, line: at.line, col: at.col });
      if (frames.length === 8) break;
    }
    return { type: String(err?.name ?? "Error"), message: message.slice(0, 1024), frames };
  }
  addEventListener("error", (e) => e.target === window && thrown(e.error));
  addEventListener("unhandledrejection", (e) => thrown(e.reason));

  // The screen has drawn, for the page holding the frame to show it in place of what shows there: it's loaded and has
  // what it asked for, a moment after its last call came back; or, once that's kept the user waiting (`patient` is over),
  // it has something to show for now: its title, or grey shapes.
  let loaded = false, out = 0, told = "", quiet = 0, patient = true;
  const showsSomething = () => !!document.body && (!!document.body.innerText.trim() || !!document.querySelector("[data-slot=skeleton], img, svg, canvas, input, button"));
  // Done: what it asked has come. Early: it's kept the user waiting, and has only its title or grey shapes to show, which
  // a page holding a still of the screen won't swap it for.
  const stage = () => (!out ? "done" : !patient && showsSomething() ? "early" : "");
  function settled() {
    if (told === "done" || !loaded || !framed || !stage() || stage() === told) return;
    clearTimeout(quiet);
    quiet = setTimeout(() => {
      const now = stage();
      if (!now || now === told) return;
      told = now;
      window.parent.postMessage({ oneDrawn: true, early: now === "early" }, "*");
      answered();
    }, out ? 0 : 50);
  }
  // Once it has drawn, and nothing it asked the user's devices is still out (what it showed from what was kept is asked
  // again), the page holding the frame is told, once: how long its newest took to come, for how it opened.
  function answered() {
    if (toldFresh || told !== "done" || asking) return;
    toldFresh = true;
    window.parent.postMessage({ oneFresh: true }, "*");
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
  // sealed on its device for its user, and gives it (`kept`, with when it was answered) to a screen that asks that
  // call again. The first time a screen asks a call, it's answered at once with what was kept, while the call goes out
  // as ever; if its answer differs, the screen is told its app changed, as by a node, and asking again gives it.
  // Asked again after that, a call goes out, so nothing a screen does after a write is answered from before it. A host
  // that keeps nothing (an older one, a chat's) refuses `kept`, and every call goes out.
  const first = new Set();
  const landed = new Map();
  // Calls answered from what was kept a while ago, by their key, while their answer is still out: what the note says.
  const old = new Map();
  // Whether the page holding the frame already says this for the whole window (its offline banner).
  let windowSays = false;
  async function call(tool, input) {
    const m = { type: "call", app: ctx.app, tool, input };
    const key = JSON.stringify([tool, input]);
    const got = landed.get(key);
    landed.clear();
    if (got && Date.now() - got.at < 10_000) return got.answer;
    const sent = () => host(m).then((answer) => (old.delete(key) && note(), answer));
    if (first.has(key)) return sent();
    first.add(key);
    const kept = await host({ type: "kept", app: ctx.app, tool, input }).catch(() => null);
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
  // How old what shows is, while its fresh answer is late: one grey line at its foot, gone once it's in.
  function note() {
    let el = document.getElementById("one-kept");
    if (!old.size || windowSays) return el?.remove();
    if (!document.body) return;
    if (!el) document.body.prepend((el = Object.assign(document.createElement("div"), { id: "one-kept", className: "kept-note" })));
    el.textContent = `As of ${one.ago(Math.min(...old.values()), { long: true })}`;
  }

  // Errors and statuses are written for users and agents at once: what happened and what to do, in users' words,
  // then after a colon or in brackets what an agent needs (a tool to call, an id, a code). A sentence with any of
  // those is for agents only.
  const forAgents = /\b[a-z][a-z0-9-]*_[a-z0-9_]+\b|\b(user|group):[0-9a-f]{6,}|\b(kernel|node|deploy|scope|token|provider|workerd|bucket|lease|ctx)s?\b|\b(an|the|this|that) install\b|network’s|[\w.-]*\/[\w.-]+\/|\b[1-5]\d\d\b/i;

  // Whether a screen has asked for a large title (`largeTitle`): it sets up the bar's title once.
  let titled = false;
  // The large title, and how far the window's own buttons reach along its row, in the window's pixels.
  let largeTitle = null, barStart = 0;
  /// The large title starts past the window's buttons, where it needs to. `zoom` makes this page's pixels the window's
  /// differently by engine, so one is measured.
  function placeLargeTitle(transition = "") {
    if (!largeTitle) return;
    const unit = Object.assign(document.createElement("div"), { style: "position:fixed;width:100px;visibility:hidden" });
    document.documentElement.append(unit);
    const scale = unit.getBoundingClientRect().width / 100;
    unit.remove();
    const zoom = parseFloat(getComputedStyle(document.documentElement).zoom) || 1;
    const margin = parseFloat(largeTitle.style.marginInlineStart) || 0;
    const left = largeTitle.getBoundingClientRect().left / scale - margin;
    largeTitle.style.transition = transition;
    largeTitle.style.marginInlineStart = barStart / zoom > left ? `${barStart / zoom - left}px` : "";
  }

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, me: ctx.me,
    // The product: its `name` and `cli`, and `made_here`, what an App Store calls the apps made in the network.
    config: ctx.config,
    // The page the window opened this from ("user", "group:<id>", "global"), or null: a hint, which the app may follow.
    scope: ctx.scope ?? null,
    call: (tool, input = {}) => (out++, call(tool, input).finally(() => (out--, settled()))),
    // Opens one of this app's screens; or with `app`, another app's (a notification's link, say). `scope` is the page
    // it's opened for (else this screen's). With `sheet`, it slides up over this one, and this resolves once it's gone,
    // to what it gave `close`; otherwise it's pushed on top, with Back. `from`, the element it was opened from (an app's
    // icon), which the window may grow it out of.
    open: (screen, params = {}, app, { scope, sheet, from } = {}) => {
      const rect = from instanceof Element && from.getBoundingClientRect();
      return host({ type: "open", screen, params, ...(app && app !== ctx.app ? { app } : {}), ...(scope ? { scope } : {}), ...(sheet ? { sheet: true } : {}), ...(rect ? { rect: [rect.left, rect.top, rect.width, rect.height].map(Math.round) } : {}) });
    },
    // `open`'s `screen`, `params`, `app` and `scope`, as a press that will open it comes down: the window may start on it
    // before the finger lifts. If nothing is opened after all, it's as if this was never said.
    opening: (screen, params = {}, app, { scope } = {}) => void host({ type: "opening", screen, params, ...(app && app !== ctx.app ? { app } : {}), ...(scope ? { scope } : {}) }).catch(() => {}),
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
    // for the window's path, Back and window name. `large` is for `largeTitle`.
    title: (text, name, large) => host({ type: "title", text, ...(name ? { name: String(name) } : {}), ...(large ? { large: true } : {}) }),
    // What a failure means for someone: what happened, in plain words, whether anything was lost, and what to do next.
    // `app` is the title of the app that failed, where a screen knows it.
    plain: (err, { app } = {}) => {
      let msg = String(err?.message ?? err ?? "").replace(/^(Uncaught )?\w*Error:\s*/, "").trim();
      while (/\([^()]*\)/.test(msg)) msg = msg.replace(/\s*\([^()]*\)/g, "");
      if (/failed to fetch|load failed|networkerror|not connected|offline|unreachable|no route|connection (refused|reset|lost)/i.test(msg))
        return { title: "Couldn’t reach your devices.", safe: "Nothing was lost.", next: "Check your connection." };
      if (/timed? ?out|took too long|took longer than/i.test(msg))
        return { title: app ? `${app} took too long, so it stopped.` : "That took too long, so it stopped.", safe: "Nothing was lost.", next: "" };
      if (/may already be done/i.test(msg)) return { title: "This may already be done.", safe: "", next: "Check before trying again." };
      // The leading sentences before any colon that are for users; none if the first is for agents.
      const parts = [];
      for (const part of msg.split(/:\s/)[0].split(/(?<=[.!?])\s+|;\s+/)) {
        if (!part || forAgents.test(part)) break;
        parts.push(part);
      }
      const said = parts.map((s) => (/^\S*@/.test(s) ? s : s[0].toUpperCase() + s.slice(1)).replace(/([^.!?])$/, "$1.").replace(/\b'\b/g, "’")).join(" ");
      if (!said || said.length > 200 || /something went wrong|internal (server )?error|unexpected|undefined|null|is not a function|\[object/i.test(said))
        return { title: "That didn’t work.", safe: "Nothing was lost.", next: "" };
      // Something that isn't there won't be there on a second try,
      // and neither will what isn't allowed, or needs signing in first: `final` says trying again can't work.
      const final = err?.code === "refused" || err?.code === "not_signed_in" || err?.code === "not_added" || /\b(not found|no such|no \w+ named|doesn't exist|isn't here|there's no)\b|^no \w+ [\w./-]+\.?$/i.test(msg);
      // What is on its way needs nobody: it's said in no colour (Problem's `quiet`). An app that isn't added has one fix,
      // adding it, which `notAdded` leads to.
      return { title: said, safe: "Nothing was lost.", next: "", final, mends: err?.code === "restarting" && /\bstarting\b/i.test(msg), notAdded: err?.code === "not_added" };
    },
    // A large title at the top of a screen, as iOS draws one: the window's own title stays empty while it shows, and is
    // its text once it scrolls away, so the name shows once. The screen's `.large-title` is followed as it redraws, so
    // calling it once is enough; put what it returns first, or draw a `.large-title` of its own.
    largeTitle: (text) => {
      const follows = !!window.IntersectionObserver;
      if (!titled) {
        titled = true;
        one.title(follows ? "" : text, text, true);
        if (follows) {
          const seen = new IntersectionObserver((all) => {
            const e = all.filter((x) => x.target === largeTitle).pop();
            if (e) one.title(e.isIntersecting ? "" : largeTitle.textContent, largeTitle.textContent, true);
          });
          const follow = () => {
            const now = document.querySelector(".large-title");
            if (now === largeTitle) return;
            if (largeTitle) seen.unobserve(largeTitle);
            if ((largeTitle = now)) seen.observe(largeTitle), placeLargeTitle();
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
    // JPEG `{ name, type, size, base64 }`; none if the user cancels.
    scan: () => host({ type: "scan" }),
    // Asks the user for what this app's `vault` declares as `need`: they pick one of their items or make one, in a
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
    // What React's error boundary caught (@one/ui's `createRoot`), which never reaches the window.
    __thrown: thrown,
    // The host calls this with node events: an app screen's "changed" is its app's data changing; a Settings
    // screen's, any event, since what it shows is the network's.
    __event(e) {
      if (!ctx.app || (e.type === "changed" && (e.apps || []).includes(ctx.app))) landed.clear(), handlers.changed.forEach((f) => f(e));
    },
    // For Settings screens only (no app's: a client answers these for its own Settings screens and refuses an app
    // screen). `api` asks the node's /api API as the user looking ("POST", "/api/group", { name }) and gives its JSON,
    // failing with its `error`. `signOut` signs this client out. `appearance` gives this client's ("automatic",
    // "light" or "dark"), first setting it to `value` if given. `device` is this client:
    // `{ name ("mac", "Chrome on Mac"), signOut (whether it can sign out), notify? }`, `notify` a browser's notifications
    // permission ("default", "granted" or "denied"), which `notify` asks the user for and then gets notifications with.
    api: (method, path, body) => host({ type: "api", method, path, body }),
    // Opens the client's own Vault sheet for a request already made (as an agent's link does): true once it's
    // answered with an item, false if declined or closed.
    request: (scope, id) => host({ type: "request", scope, id }),
    signOut: () => host({ type: "signOut" }),
    // What this client keeps for its own screens between launches, by `key`: with `value` (JSON), keeps it; without,
    // gives what was kept, or null. Forgotten when it signs out.
    cache: (key, value) => host(value === undefined ? { type: "cache", key: String(key) } : { type: "cache", key: String(key), value }),
    appearance: (value) => host({ type: "appearance", value }),
    device: () => host({ type: "device" }),
    notify: () => host({ type: "notify" }),
    esc: (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]),
    // Whether a device is on, for a row's grey line: "Online" while it's online, "Seen 2 minutes ago" once it isn't,
    // and "" for one never seen. `d` is a device as the node's state gives it: `{ online, seen }` (ms).
    seen: (d) => (d.online ? "Online" : d.seen ? `Seen ${one.ago(d.seen, { long: true })}` : ""),
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
