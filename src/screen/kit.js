// The kit's pieces, for screens that want them: each is a function on `one` that a screen calls, and none does anything
// until it's called. Most return HTML for a screen's own templates.
(() => {
  const esc = one.esc;

  // A spinner that says what it waits for, to people and to screen readers: for an action, not a screen's first draw.
  one.loading = (label = "Loading") =>
    `<div class="loading" role="status" aria-live="polite"><span class="spinner" aria-hidden="true"></span><span>${esc(label)}</span></div>`;

  // Grey shapes where what's loading will be, so the screen looks ready and doesn't jump when it comes: `rows` rows
  // of a list (the kit's .list), "text" for lines of text, or "cards" for tiles. Screen readers hear `label`.
  one.skeleton = (shape = 4, label = "Loading") => {
    const line = (w, cls = "") => `<span class="sk-line${cls}" style="width:${w}%"></span>`;
    const widths = [62, 48, 70, 55, 40, 66, 52, 58];
    const inner = shape === "text"
      ? `<div class="sk-text">${[92, 100, 84, 96, 60].map((w) => line(w)).join("")}</div>`
      : shape === "cards"
      ? `<div class="sk-cards">${'<span class="sk-card"></span>'.repeat(4)}</div>`
      : `<div class="list">${widths.slice(0, Math.max(1, Math.min(+shape || 4, 8))).map((w) =>
          `<div class="row"><span class="sk-dot"></span><div class="main">${line(w)}${line(w - 18, " sub")}</div></div>`).join("")}</div>`;
    return `<div class="skeleton" role="status" aria-live="polite" aria-label="${esc(label)}">${inner}</div>`;
  };

  // A button for what the kit draws, `action` `{ label, run }`: run when it's tapped.
  const runs = [];
  const button = (action, cls = "button") => `<button class="${cls}" data-run="${runs.push(action.run) - 1}">${esc(action.label)}</button>`;
  // One that waits on what it runs says so in itself until it's done.
  document.addEventListener("click", async (e) => {
    const b = e.target.closest?.(".empty [data-run], .problem [data-run]");
    const waiting = b && !b.disabled && runs[b.dataset.run]?.();
    if (!waiting?.then) return;
    const back = one.busy(b);
    await waiting.catch(() => {});
    back();
  });

  // A button busy with what a tap started: disabled at once, with a small spinner before `label` (what it says
  // already, if none), so a tap is seen before its answer comes. Returns what puts it back, for when it fails or what
  // it shows stays.
  one.busy = (b, label) => {
    const was = b.innerHTML, off = b.disabled;
    b.disabled = true;
    b.setAttribute("aria-busy", "true");
    b.innerHTML = `<span class="spinner small" aria-hidden="true"></span>${esc(label ?? b.textContent.trim())}`;
    return () => {
      b.innerHTML = was;
      b.disabled = off;
      b.removeAttribute("aria-busy");
    };
  };

  // Pressed looks (:active) show on a phone's touch too: iOS gives them only to a page that listens for touches.
  document.addEventListener("touchstart", () => {}, { passive: true });

  // An empty screen or list: one grey line (`html`, escaped by its caller), and a button only when there's an obvious
  // first step.
  one.empty = (html, action) => `<div class="empty">${html}${action ? `<div>${button(action)}</div>` : ""}</div>`;

  // A problem, shown on the thing it's about: a symbol, a short title, one sentence, and the action that fixes it.
  // `warn` for what can wait; otherwise it's red.
  one.problem = ({ title, text = "", action, warn = false }) =>
    `<div class="problem${warn ? " warn" : ""}" role="alert">${one.icon("warning-circle-fill", "problem-icon")}<div class="problem-main">` +
    `<div class="problem-title">${esc(title)}</div>${text ? `<div class="problem-text">${esc(text)}</div>` : ""}` +
    (action ? button(action, "problem-action") : "") + "</div></div>";

  // Errors and statuses are written for people and agents at once: what happened and what to do, in people's words,
  // then after a colon or in brackets what an agent needs (a tool to call, an id, a code). A sentence with any of
  // those is for agents only.
  const forAgents = /\b[a-z][a-z0-9-]*_[a-z0-9_]+\b|\b(user|group):[0-9a-f]{6,}|\b(kernel|node|sidecar|deploy|scope|token|provider|workerd|bucket|lease|ctx)s?\b|\b(an|the|this|that) install\b|install's|[\w.-]*\/[\w.-]+\/|\b[1-5]\d\d\b/i;

  // What a failure means for someone: what happened, in plain words, whether anything was lost, and what to do next.
  // `app` is the title of the app that failed, where a screen knows it.
  one.plain = (err, { app } = {}) => {
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
  };

  // Draws `fn()`'s HTML into `el`. The first time, if it takes longer than `wait` ms, grey shapes stand where it will
  // be (`skeleton`: rows of a list, "text" or "cards", or false for a spinner saying `loading`); a fast load shows
  // nothing in between, so nothing flashes. If it fails, the problem, with Try Again. Later draws (on "changed") replace
  // what's there only once they're ready, and keep the scroll where it was. `fresh` is a draw of something else
  // (another page's items): what's there dims at once, and grey shapes take its place if it's slow.
  one.render = async (el, fn, { loading = "Loading", wait = 100, skeleton = 4, fresh = false } = {}) => {
    if (typeof el === "string") el = document.querySelector(el);
    const turn = (el.__turn = (el.__turn || 0) + 1);
    const waiting = () => (skeleton === false ? one.loading(loading) : one.skeleton(skeleton, loading));
    if (fresh && el.__drawn) {
      el.__drawn = false;
      el.__html = null;
      el.classList.add("waiting");
    }
    const slow = el.__drawn ? null : setTimeout(() => turn === el.__turn && ((el.innerHTML = waiting()), el.classList.remove("waiting")), wait);
    try {
      const html = await fn();
      if (turn !== el.__turn) return;
      if (html !== undefined && html !== el.__html) {
        const y = scrollY;
        el.innerHTML = el.__html = html;
        if (el.__drawn && scrollY !== y) scrollTo(scrollX, y);
      }
      el.__drawn = true;
    } catch (err) {
      if (turn !== el.__turn) return;
      const p = one.plain(err);
      // A redraw that fails leaves what's there, which was right a moment ago; the next change tries again.
      if (el.__drawn && !p.missing) return;
      el.__html = null;
      el.__drawn = false;
      if (p.missing) return void (el.innerHTML = one.notFound({ title: p.title }));
      el.innerHTML = one.problem({ title: p.title, text: p.next, action: { label: "Try Again", run: () => {
        one.tap();
        el.innerHTML = waiting();
        one.render(el, fn, { loading, wait, skeleton });
      } } });
    } finally {
      clearTimeout(slow);
      if (turn === el.__turn) el.classList.remove("waiting");
    }
  };

  // A button that copies `text` and says so: a check and "Copied" for a moment. `label` is its text, or none.
  one.copyButton = (text, { label = "Copy", cls = "" } = {}) =>
    `<button class="copy ${cls}" data-copy="${esc(text)}" aria-label="Copy ${esc(label === "Copy" ? "" : label)}">${one.icon("copy")}${label ? `<span>${esc(label)}</span>` : ""}</button>`;
  document.addEventListener("click", async (e) => {
    const b = e.target.closest?.("[data-copy]");
    if (!b) return;
    e.stopPropagation();
    await one.copy(b.dataset.copy);
    one.tap("success");
    const was = b.innerHTML;
    b.classList.add("copied");
    b.innerHTML = one.icon("check") + (b.querySelector("span") ? "<span>Copied</span>" : "");
    b.setAttribute("aria-label", "Copied");
    clearTimeout(b.__undo);
    b.__undo = setTimeout(() => {
      b.innerHTML = was;
      b.classList.remove("copied");
    }, 1500);
  }, true);

  // A card in `el` before a risky change: what will happen, Cancel, and a named button that runs `run` with a
  // spinner. Resolves to whether it ran.
  one.confirm = (el, { title, detail = "", action = "Confirm", danger = false, run }) =>
    new Promise((done) => {
      if (typeof el === "string") el = document.querySelector(el);
      el.innerHTML = `<div class="confirm" role="group" aria-label="${esc(title)}"><div class="confirm-title">${esc(title)}</div>` +
        (detail ? `<div class="confirm-detail">${detail}</div>` : "") +
        `<div class="confirm-error" role="alert"></div><div class="confirm-buttons"><button class="button plain" data-no>Cancel</button>` +
        `<button class="button ${danger ? "danger" : "primary"}" data-yes>${esc(action)}</button></div></div>`;
      const yes = el.querySelector("[data-yes]");
      el.querySelector("[data-no]").onclick = () => {
        el.innerHTML = "";
        done(false);
      };
      yes.onclick = async () => {
        one.tap(danger ? "medium" : "selection");
        const back = one.busy(yes);
        try {
          await run?.();
          el.innerHTML = "";
          done(true);
        } catch (err) {
          const p = one.plain(err);
          el.querySelector(".confirm-error").textContent = `${p.title} ${p.safe}`;
          back();
        }
      };
    });

  // A search field in `el`: a magnifier, a clear button, and `onSearch(query)` once typing stops for `delay` ms.
  one.search = (el, { placeholder = "Search", value = "", delay = 180, onSearch }) => {
    if (typeof el === "string") el = document.querySelector(el);
    el.innerHTML = `<label class="search">${one.icon("magnifying-glass")}<input type="search" enterkeyhint="search" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="${esc(placeholder)}" aria-label="${esc(placeholder)}" value="${esc(value)}"><button type="button" class="clear" aria-label="Clear" hidden>${one.icon("x")}</button></label>`;
    const input = el.querySelector("input"), clear = el.querySelector(".clear");
    let timer, last = value;
    const fire = () => {
      clearTimeout(timer);
      clear.hidden = !input.value;
      if (input.value !== last) onSearch?.((last = input.value));
    };
    input.addEventListener("input", () => {
      clear.hidden = !input.value;
      clearTimeout(timer);
      timer = setTimeout(fire, delay);
    });
    input.addEventListener("keydown", (e) => e.key === "Enter" && fire());
    clear.onclick = () => {
      input.value = "";
      fire();
      input.focus();
    };
    clear.hidden = !value;
    return input;
  };

  // A large title at the top of a screen, as iOS draws one: the shell's own title stays empty while it shows, and is
  // its text once it scrolls away, so the name shows once. The screen's `.large-title` is followed as it redraws, so
  // calling it once is enough; put what it returns first, or draw a `.large-title` of its own. A shell whose bar always
  // shows the title (Android's, `__oneShell.bar`) hides it and takes the text.
  let titled = false;
  one.largeTitle = (text) => {
    const follows = window.IntersectionObserver && !window.__oneShell?.bar;
    if (window.__oneShell?.bar) one.title(text);
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
    return `<h1 class="large-title">${esc(text)}</h1>`;
  };

  // Information, as at the bottom of an App Store page: `[label, value]` pairs, plain text, each value on one line.
  one.info = (pairs) =>
    '<div class="list info">' + pairs.filter(([, v]) => v != null && v !== "").map(([label, value]) =>
      `<div class="row plain"><span class="label">${esc(label)}</span><span class="value">${esc(value)}</span></div>`).join("") + "</div>";

  // A screen in a frame tells the page holding it how many sheets it has open, so that page dims the rest of its
  // window too, as a sheet dims a Mac window; a click there comes back as `oneSheetsClose`, closing the top one. A
  // hint, not one of the bridge's messages: a page that doesn't know it shows the sheet as before.
  const sheets = [];
  const framed = window.parent !== window && !window.webkit?.messageHandlers?.one && !window.__oneShell && !window.__one?.host;
  const tell = () => framed && window.parent.postMessage({ oneSheets: sheets.length }, "*");
  if (framed) addEventListener("message", (e) => e.source === window.parent && e.data?.oneSheetsClose && sheets.at(-1)?.());

  // A sheet that slides up over the screen, with `title` and Done: `html` is what's in it. Returns its body, to draw
  // into again, and `close`; `onClose` runs once it's gone (Done, a tap outside it, Escape).
  one.sheet = ({ title = "", html = "", done = "Done", onClose } = {}) => {
    const back = document.createElement("div");
    back.className = "sheet-backdrop";
    back.innerHTML = `<div class="sheet" role="dialog" aria-modal="true" aria-label="${esc(title)}"><div class="sheet-bar"><span></span>` +
      `<div class="sheet-title">${esc(title)}</div><button class="button plain" data-done>${esc(done)}</button></div><div class="sheet-body">${html}</div></div>`;
    document.body.append(back);
    const was = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    // Laid out first, so it slides; not with animation frames, which a window that's covered doesn't get.
    void back.offsetHeight;
    back.classList.add("open");
    let closed = false;
    const close = () => {
      if (closed) return;
      closed = true;
      sheets.splice(sheets.indexOf(close), 1);
      tell();
      document.removeEventListener("keydown", key);
      back.classList.remove("open");
      document.body.style.overflow = was;
      setTimeout(() => back.remove(), 300);
      onClose?.();
    };
    sheets.push(close);
    tell();
    const key = (e) => e.key === "Escape" && close();
    document.addEventListener("keydown", key);
    back.addEventListener("click", (e) => e.target === back && close());
    back.querySelector("[data-done]").onclick = close;
    return { body: back.querySelector(".sheet-body"), close };
  };

  // A light haptic on every press of a row, button or tab (phones only), for screens that want them.
  one.haptics = () =>
    document.addEventListener("click", (e) => e.target.closest?.("a, button, .row, .segmented button, [data-tap]") && one.tap(), true);

  // For a page that isn't there: a friendly note and a way home.
  one.notFound = ({ title = "This page isn't here.", text = "The link may be old or mistyped.", home = "Go to Home" } = {}) =>
    one.empty(esc(`${title} ${text}`), { label: home, run: () => one.open("main") });
  // Developer mode: on an app's screen, a small pill at its top with how long it took to open, which opens a panel of
  // what it did, in order: where its page came from, joining the network if it waited on that, and each call, with
  // each device tried, how long it waited and why it moved on, the one that answered, keys handed over, and where the
  // data came from. The client keeps the traces (`one.traces`); out of developer mode, or in a client that keeps none,
  // nothing shows.
  if (one.app) {
    const loaded = Date.now();
    // Requests that start within this long of the last one's end are part of opening the screen.
    const BURST = 500;
    // The client's traces; false once it says it keeps none.
    let traces = null, open = null, pill = null;
    const time = (ms) => (ms >= 1000 ? `${(ms / 1000).toFixed(1)} s` : `${Math.max(0, Math.round(ms))} ms`);

    // This screen's requests: from its page (as last asked for, at or before this screen loaded) on, of its app.
    const mine = (all) => {
      const page = (r) => r.what === "page" && r.app === one.app && (r.screen || "main") === (one.screen || "main");
      // The page as this screen asked for it: the latest asked before it loaded, with any asked beside it (a copy
      // shown while the node's was asked for).
      const pages = all.filter((r) => page(r) && r.at <= loaded);
      const latest = pages.reduce((a, r) => Math.max(a, r.at), 0);
      const from = latest ? Math.min(...pages.filter((r) => r.at + (r.ms || 0) >= latest - 100).map((r) => r.at)) : loaded - 1000;
      return all.filter((r) => r.app === one.app && r.at >= from).sort((a, b) => a.at - b.at);
    };
    // Those that opened it, and those since.
    const split = (rs) => {
      let end = rs[0]?.at || 0, n = 0;
      for (const r of rs) {
        if (r.at > end + BURST) break;
        end = Math.max(end, r.at + (r.ms || 0));
        n++;
      }
      return { opening: rs.slice(0, n), since: rs.slice(n), start: rs[0]?.at || 0, end };
    };

    // Every device's name the traces know, by key: the nodes say theirs, and the install's others.
    const names = (rs) => {
      const out = {};
      const add = (t) => {
        if (!t || typeof t !== "object") return;
        Object.assign(out, t.names || {});
        for (const s of t.steps || []) add(s.trace);
      };
      for (const r of rs) for (const f of r.fetches || []) {
        add(f.node);
        const answered = (f.tries || []).find((t) => t.result === "answered");
        if (answered && f.node?.device) out[answered.node] = f.node.device;
      }
      return out;
    };

    // What one request did, as parts of a line, each `[text, warn]`, and segments of its bar.
    const SAID = { "no connection": "couldn't connect", "not taking requests": "took no requests", "no answer": "didn't answer", unreachable: "unreachable", lost: "lost the connection", failed: "failed", elsewhere: "said elsewhere" };
    function describe(r, known) {
      const parts = [], bar = [];
      const name = (key) => known[key] || `a device (${String(key).slice(0, 6)})`;
      // How joining the network went, in parts: this device's key, the relay's list of devices, the client's code
      // and its connection to the relay.
      const j = r.joined || {};
      const joining = [["key", "key"], ["nodes", "the relay's list of your devices"], ["code", "client code"], ["endpoint", "relay connection"]]
        .filter(([k]) => j[k] >= 1).map(([k, label]) => `${label} ${time(j[k])}`).join(", ");
      if (r.source === "kept") parts.push(["shown from this device's copy", false]);
      if (r.waited >= 1) {
        parts.push([r.first ? `the window loaded and let this device in first, ${time(r.waited)}${joining ? ` (joining the network: ${joining})` : ""}` : `waited ${time(r.waited)} for this device to be let in`, false]);
        bar.push([r.waited, "join"]);
      }
      if (r.network) {
        parts.push([`waited ${time(r.network)} to join the network${joining ? ` (${joining})` : ""}`, false]);
        bar.push([r.network, "join"]);
      }
      const whose = (scope) => (scope === "everyone" ? "everyone's" : scope === "group" ? "the group's" : "your");
      // One thing the client asked for as it got a call's keys.
      const clientStep = (s) => s.step === "members" ? `read who's who again, ${time(s.ms)}`
        : s.step === "newest" ? `asked where ${s.databases === 1 ? "its data starts" : `${s.databases} databases start`}, ${time(s.ms)}`
        : s.step === "keys" ? `${whose(s.scope)} ${s.of === "epochs" ? "keys" : `${s.of} key`}, ${time(s.ms)}`
        : `${s.step}, ${time(s.ms)}`;
      // What a node did: the devices it tried, and where the data came from.
      const node = (t, depth = 0) => {
        if (!t || typeof t !== "object") return;
        for (const s of t.steps || []) {
          if (s.step === "device") {
            if (s.result === "ran") {
              if (!s.here) parts.push([`${t.device} passed it to ${s.device} (${time(s.ms)})`, false]);
              node(s.trace, depth + 1);
            } else if (s.result === "needs keys") parts.push([`${s.device} needed keys`, false]);
            else {
              parts.push([`${t.device} tried ${s.device}: ${SAID[s.result] || s.result}${s.why ? ` (${s.why})` : ""}, after ${time(s.ms)}`, true]);
              node(s.trace, depth + 1);
            }
          } else if (s.step === "open") parts.push([`opened ${s.app}'s data on ${t.device}: ${s.steps} steps, ${time(s.ms)}`, false]);
          else if (s.step === "bucket") parts.push([`${s.found} of ${s.asks} fetched from the bucket in ${time(s.ms)}`, false]);
          else if (s.step === "behind") parts.push([`asked the bucket first, as another device wrote: ${s.result}, ${time(s.ms)}`, s.result !== "caught up"]);
          else if (s.step === "data" && !(t.steps || []).some((o) => o.step === "open")) parts.push([`read ${t.device}'s copy${s.new ? `, ${s.new} new steps` : ""}, ${time(s.ms)}`, false]);
          else if (s.step === "logs" && s.ms >= 20) parts.push([`read who's who on ${t.device}, ${time(s.ms)}`, false]);
          else if (s.step === "run") parts.push([`${s.app} ran on ${t.device} in ${time(s.ms)}`, false]);
        }
      };
      // Asked again: with the keys a node said it needed, or once it had caught up with this device.
      const needed = (f) => (f?.node?.steps || []).some((s) => s.step === "device" && s.result === "needs keys");
      (r.fetches || []).forEach((f, i, all) => {
        // The keys this device opened to hand with it, if it had to ask for them first.
        if (f.handed && typeof f.handed === "object") {
          parts.push([`this device opened the keys it hands in ${time(f.handed.ms)}`, false]);
          for (const s of f.handed.steps || []) parts.push([clientStep(s), false]);
          bar.push([f.handed.ms || 0, "join"]);
        }
        for (const t of f.tries || []) {
          if (t.result === "answered") {
            parts.push([`${i ? (needed(all[i - 1]) ? "again with keys " : "again ") : ""}→ ${name(t.node)}${t.connect ? `, connected in ${time(t.connect)}` : ""}`, false]);
            if (t.connect) bar.push([t.connect, "join"]);
          } else {
            parts.push([`${name(t.node)}: ${SAID[t.result] || t.result}, gave up after ${time(t.ms)}`, true]);
            bar.push([t.ms, "warn"]);
          }
        }
        node(f.node);
        // What this device did between a node asking for keys and sending it again: each thing it asked for, timed.
        if (f.client && typeof f.client === "object") {
          parts.push([`this device got the keys in ${time(f.client.ms)}`, false]);
          for (const s of f.client.steps || []) parts.push([clientStep(s), false]);
          bar.push([f.client.ms || 0, "join"]);
        }
      });
      if (r.failed) parts.push(["failed", true]);
      const used = bar.reduce((a, [ms]) => a + ms, 0);
      bar.push([Math.max(0, (r.ms || 0) - used), "rest"]);
      return { parts, bar };
    }

    const title = (r) => (r.what === "call" ? `${r.app}.${r.op}` : r.what === "page" ? "The page's code" : r.path || "");
    const warned = (r, known) => describe(r, known).parts.some(([, w]) => w);
    function row(r, known, span) {
      const { parts, bar } = describe(r, known);
      const track = span ? `<div class="trace-track"><div class="trace-bar" style="margin-left:${((r.at - span.start) / span.ms) * 100}%;width:${Math.max(0.5, ((r.ms || 0) / span.ms) * 100)}%">` +
        bar.filter(([ms]) => ms > 0).map(([ms, kind]) => `<span class="${kind}" style="flex:${ms}"></span>`).join("") + "</div></div>" : "";
      return `<div class="row plain trace-row"><div class="main"><div class="title"><span>${esc(title(r))}</span><span class="when">${time(r.ms || 0)}</span></div>${track}` +
        `<div class="sub wrap trace-parts">${parts.map(([text, warn]) => `<span${warn ? ' class="warn"' : ""}>${esc(text)}</span>`).join(" · ")}</div></div></div>`;
    }

    function draw() {
      const rs = mine(traces || []);
      const known = names(traces || []);
      const { opening, since, start, end } = split(rs);
      if (!opening.length) return pill?.remove();
      if (!pill || !pill.isConnected) {
        pill = document.createElement("div");
        pill.className = "trace-pill-bar";
        pill.innerHTML = `<button class="trace-pill" type="button"></button>`;
        pill.firstChild.onclick = () => show();
        document.body.prepend(pill);
      }
      const b = pill.firstChild;
      b.textContent = time(end - start);
      b.classList.toggle("warn", opening.some((r) => warned(r, known)));
      b.setAttribute("aria-label", `Opened in ${time(end - start)}: what it did`);
      if (open) {
        const span = { start, ms: Math.max(1, end - start) };
        open.body.innerHTML = `<div class="trace-total">Opened in ${esc(time(end - start))}</div>` +
          `<div class="list">${opening.map((r) => row(r, known, span)).join("")}</div>` +
          (since.length ? `<h2 class="section">Since</h2><div class="list">${since.map((r) => row(r, known)).join("")}</div>` : "");
      }
    }
    function show() {
      open = one.sheet({ title: "What This Page Did", onClose: () => (open = null) });
      draw();
      refresh();
    }
    async function refresh() {
      traces = (await one.traces()) || false;
      if (traces) draw();
      else pill?.remove();
    }
    // Asked again once each call is answered, unless this client keeps no traces.
    const call = one.call;
    one.call = (...args) => {
      const asked = call(...args);
      if (traces !== false) asked.finally(() => setTimeout(refresh)).catch(() => {});
      return asked;
    };
    // And a few times as it opens, as a call answered at once from what this client kept still goes out.
    const opening = () => [50, 500, 1500, 3000, 6000, 12000].forEach((ms) => setTimeout(() => traces !== false && refresh(), ms));
    if (document.readyState === "complete") opening();
    else addEventListener("load", opening);
  }
})();
