// The kit's pieces, for screens that want them: each is a function on `one` that a screen calls, and none does anything
// until it's called. Most return HTML for a screen's own templates.
(() => {
  const esc = one.esc;

  // Icons, drawn on a 24-point grid in the text's colour.
  const paths = {
    alert: '<circle cx="12" cy="12" r="9"/><path d="M12 7.5v5.5M12 16.5h.01"/>',
    check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
    copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
    search: '<circle cx="11" cy="11" r="6.5"/><path d="M16 16l4.5 4.5"/>',
    x: '<path d="M6 6l12 12M18 6L6 18"/>',
    folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
    file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
    submodule: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M9 13h6M13 11l2 2-2 2"/>',
    book: '<path d="M4 19V5a2 2 0 0 1 2-2h14v14H6a2 2 0 0 0-2 2zm0 0a2 2 0 0 0 2 2h14"/>',
    history: '<path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1L3.5 8.5"/><path d="M3.5 4v4.5H8M12 7.5V12l3 2"/>',
    branch: '<circle cx="6" cy="5" r="2"/><circle cx="6" cy="19" r="2"/><circle cx="18" cy="7" r="2"/><path d="M6 7v10M18 9v1a4 4 0 0 1-4 4h-4a4 4 0 0 0-4 4"/>',
    commit: '<circle cx="12" cy="12" r="3.5"/><path d="M3 12h5.5M15.5 12H21"/>',
    tray: '<path d="M3 13l3-8h12l3 8v6a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z"/><path d="M3 13h5l1.5 2.5h5L16 13h5"/>',
    chevron: '<path d="M9 6l6 6-6 6"/>',
    down: '<path d="M6 9l6 6 6-6"/>',
    question: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.6v.6M12 17h.01"/>',
    wrap: '<path d="M4 6h16M4 12h13a3 3 0 0 1 0 6h-4M4 18h5"/><path d="M15 16l-2 2 2 2"/>',
    code: '<path d="M8 7l-5 5 5 5M16 7l5 5-5 5"/>',
    diff: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5M12 10v6M9 13h6"/>',
    up: '<path d="M18 15l-6-6-6 6"/>',
  };
  one.icon = (name, cls = "") =>
    `<svg class="i ${cls}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.file}</svg>`;

  // A spinner that says what it waits for, to people and to screen readers.
  one.loading = (label = "Loading") =>
    `<div class="loading" role="status" aria-live="polite"><span class="spinner" aria-hidden="true"></span><span>${esc(label)}</span></div>`;

  // An empty list: an icon, a title, a line of text and a button, each but the title optional. `action.onclick` is
  // the button's script.
  one.empty = ({ icon = "tray", title, text, action } = {}) =>
    `<div class="empty-state"><div class="empty-icon">${one.icon(icon)}</div><div class="empty-title">${esc(title)}</div>` +
    (text ? `<div class="empty-text">${text}</div>` : "") +
    (action ? `<button class="button" onclick="${esc(action.onclick)}">${esc(action.label)}</button>` : "") +
    "</div>";

  // What a failure means for someone: what happened, in plain words, what is safe, and what to do next.
  one.plain = (err) => {
    let msg = String(err?.message ?? err ?? "").replace(/^(Uncaught )?\w*Error:\s*/, "").trim();
    if (/failed to fetch|load failed|networkerror|not connected|offline|unreachable|no route|connection (refused|reset|lost)/i.test(msg))
      return { title: "Couldn't reach your server.", safe: "Nothing was lost.", next: "Check your connection, then try again." };
    if (/timed? ?out|took too long/i.test(msg))
      return { title: "Your server took too long to answer.", safe: "Nothing was lost.", next: "Try again in a moment." };
    if (!msg || msg.length > 200 || /something went wrong|internal (server )?error|unexpected|\b5\d\d\b|undefined|null|is not a function|\[object/i.test(msg))
      return { title: "This app could not complete your request.", safe: "Nothing was lost.", next: "Try again in a moment." };
    msg = msg[0].toUpperCase() + msg.slice(1);
    // Something that isn't there won't be there on a second try: say so, and offer the way home instead.
    const missing = /\b(not found|no such|doesn't exist|isn't here|there's no)\b/i.test(msg);
    return { title: /[.!?]$/.test(msg) ? msg : msg + ".", safe: "Nothing was lost.", next: "Try again in a moment.", missing };
  };

  // A failure as a banner, with Try again unless `retry` is false (one.render wires the button).
  one.banner = (err, { safe, next, retry = true } = {}) => {
    const p = one.plain(err);
    return `<div class="banner" role="alert">${one.icon("alert", "banner-icon")}<div class="banner-text"><strong>${esc(p.title)}</strong>` +
      `<span>${esc(safe ?? p.safe)} ${esc(next ?? p.next)}</span></div>` +
      (retry ? '<button class="button" data-retry>Try again</button>' : "") + "</div>";
  };

  // Draws `fn()`'s HTML into `el`: with a labelled spinner the first time, if it's slow, and a banner with Try again
  // if it fails. Later draws (on "changed") replace what's there only once they're ready.
  one.render = async (el, fn, { loading = "Loading" } = {}) => {
    if (typeof el === "string") el = document.querySelector(el);
    const turn = (el.__turn = (el.__turn || 0) + 1);
    const slow = el.__drawn ? null : setTimeout(() => turn === el.__turn && (el.innerHTML = one.loading(loading)), 120);
    try {
      const html = await fn();
      if (turn !== el.__turn) return;
      if (html !== undefined) el.innerHTML = html;
      el.__drawn = true;
    } catch (err) {
      if (turn !== el.__turn) return;
      const p = one.plain(err);
      if (p.missing) return void (el.innerHTML = one.notFound({ title: p.title }));
      el.innerHTML = one.banner(err);
      el.__drawn = false;
      el.querySelector("[data-retry]").onclick = () => {
        one.tap();
        el.innerHTML = one.loading(loading);
        one.render(el, fn, { loading });
      };
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
    el.innerHTML = `<label class="search">${one.icon("search")}<input type="search" enterkeyhint="search" autocomplete="off" autocapitalize="off" spellcheck="false" placeholder="${esc(placeholder)}" aria-label="${esc(placeholder)}" value="${esc(value)}"><button type="button" class="clear" aria-label="Clear" hidden>${one.icon("x")}</button></label>`;
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

  // A light haptic on every press of a row, button or tab (phones only), for screens that want them.
  one.haptics = () =>
    document.addEventListener("click", (e) => e.target.closest?.("a, button, .row, .segmented button, [data-tap]") && one.tap(), true);

  // For a page that isn't there: a friendly note and a way home.
  one.notFound = ({ title = "This page isn't here.", text = "The link may be old or mistyped.", home = "Go to Home" } = {}) =>
    one.empty({ icon: "question", title, text: esc(text), action: { label: home, onclick: "one.open('main')" } });
})();
