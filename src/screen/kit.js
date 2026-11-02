// The kit's pieces, for screens that want them: each is a function on `one` that a screen calls, and none does anything
// until it's called. Most return HTML for a screen's own templates.
(() => {
  const esc = one.esc;

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
      return { title: "Couldn't reach your device.", safe: "Nothing was lost.", next: "Check your connection, then try again." };
    if (/timed? ?out|took too long/i.test(msg))
      return { title: "Your device took too long to answer.", safe: "Nothing was lost.", next: "Try again in a moment." };
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
    return `<div class="banner" role="alert">${one.icon("warning-circle", "banner-icon")}<div class="banner-text"><strong>${esc(p.title)}</strong>` +
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

  // A large title at the top of a screen, as iOS draws one: the shell's own title stays empty until it scrolls away,
  // so the name shows once. Call it once per screen, and put what it returns first.
  one.largeTitle = (text) => {
    one.title("");
    setTimeout(() => {
      const el = document.querySelector(".large-title");
      if (!el || !window.IntersectionObserver) return void one.title(text);
      new IntersectionObserver(([e]) => one.title(e.isIntersecting ? "" : text)).observe(el);
    });
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
    one.empty({ icon: "question", title, text: esc(text), action: { label: home, onclick: "one.open('main')" } });
})();
