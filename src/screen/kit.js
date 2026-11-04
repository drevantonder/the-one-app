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
  const forAgents = /\b[a-z][a-z0-9-]*_[a-z0-9_]+\b|\b(user|group):[0-9a-f]{6,}|\b(kernel|node|deploy|scope|token|provider|workerd|bucket|lease|ctx)s?\b|\b(an|the|this|that) install\b|install's|[\w.-]*\/[\w.-]+\/|\b[1-5]\d\d\b/i;

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
})();
