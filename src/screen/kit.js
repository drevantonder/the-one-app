// The kit's pieces, for screens that want them: each is a function on `one` that a screen calls, and none does anything
// until it's called. Most return HTML for a screen's own templates.
(() => {
  const esc = one.esc;

  // A spinner that says what it waits for, to people and to screen readers.
  one.loading = (label = "Loading") =>
    `<div class="loading" role="status" aria-live="polite"><span class="spinner" aria-hidden="true"></span><span>${esc(label)}</span></div>`;

  // A button for what the kit draws, `action` `{ label, run }`: run when it's tapped.
  const runs = [];
  const button = (action, cls = "button") => `<button class="${cls}" data-run="${runs.push(action.run) - 1}">${esc(action.label)}</button>`;
  document.addEventListener("click", (e) => runs[e.target.closest?.(".empty [data-run], .problem [data-run]")?.dataset.run]?.());

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

  // Draws `fn()`'s HTML into `el`: with a labelled spinner the first time, if it's slow (`wait` ms), and the problem,
  // with Try Again, if it fails. Later draws (on "changed") replace what's there only once they're ready.
  one.render = async (el, fn, { loading = "Loading", wait = 120 } = {}) => {
    if (typeof el === "string") el = document.querySelector(el);
    const turn = (el.__turn = (el.__turn || 0) + 1);
    const slow = el.__drawn ? null : setTimeout(() => turn === el.__turn && (el.innerHTML = one.loading(loading)), wait);
    try {
      const html = await fn();
      if (turn !== el.__turn) return;
      if (html !== undefined) el.innerHTML = html;
      el.__drawn = true;
    } catch (err) {
      if (turn !== el.__turn) return;
      const p = one.plain(err);
      if (p.missing) return void (el.innerHTML = one.notFound({ title: p.title }));
      el.innerHTML = one.problem({ title: p.title, text: p.next, action: { label: "Try Again", run: () => {
        one.tap();
        el.innerHTML = one.loading(loading);
        one.render(el, fn, { loading });
      } } });
      el.__drawn = false;
    } finally {
      clearTimeout(slow);
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
        yes.disabled = true;
        yes.innerHTML = `<span class="spinner small" aria-hidden="true"></span>${esc(action)}`;
        try {
          await run?.();
          el.innerHTML = "";
          done(true);
        } catch (err) {
          const p = one.plain(err);
          el.querySelector(".confirm-error").textContent = `${p.title} ${p.safe}`;
          yes.disabled = false;
          yes.textContent = action;
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
  // calling it once is enough; put what it returns first, or draw a `.large-title` of its own.
  let titled = false;
  one.largeTitle = (text) => {
    if (!titled) {
      titled = true;
      one.title(window.IntersectionObserver ? "" : text);
      if (window.IntersectionObserver) {
        let el = null;
        const seen = new IntersectionObserver((all) => {
          const e = all.filter((x) => x.target === el).pop();
          if (e) one.title(e.isIntersecting ? "" : el.textContent);
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
      document.removeEventListener("keydown", key);
      back.classList.remove("open");
      document.body.style.overflow = was;
      setTimeout(() => back.remove(), 300);
      onClose?.();
    };
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
