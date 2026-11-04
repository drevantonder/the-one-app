// What a web page that holds screens does for them, the same in the web client (kernel/web/web.js) and in a chat's
// MCP view (src/mcp_view.html): their look in its frames, and the answer to each of the bridge's messages, with its
// menu, alert and files drawn in HTML where a native shell uses its own. Its functions need the kit (kit.css, bridge.js, kit.js) on the page, though it
// may load before them. Its styles come with it.
window.oneHost = (() => {
  const css = `
  .menu-backdrop { position: fixed; inset: 0; z-index: 59; }
  .menu { outline: none; position: fixed; z-index: 60; min-width: 200px; padding: 5px; border-radius: 10px; background: var(--card); box-shadow: 0 8px 28px rgba(0, 0, 0, 0.25), 0 0 0 0.5px var(--line); }
  .menu-item { display: flex; align-items: center; gap: 8px; width: 100%; text-align: left; border: 0; background: none; font: inherit; font-size: 13px; color: var(--text); padding: 5px 10px; border-radius: 5px; cursor: pointer; }
  .menu-item:hover { background: var(--tint); color: white; }
  .menu-item.danger { color: var(--danger); }
  .menu-item.danger:hover { background: var(--danger); color: white; }
  .menu-check { display: inline-block; width: 16px; }
  .menu-title { flex: 1; }
  .menu-icon { width: 15px; height: 15px; flex: none; opacity: 0.85; }
  .menu-line { height: 0.5px; margin: 5px 8px; background: var(--line); }
  .menu-row { display: flex; gap: 4px; padding: 2px 2px 6px; margin-bottom: 4px; border-bottom: 0.5px solid var(--line); }
  .menu-choice { flex: 1; height: 34px; display: grid; place-items: center; border: 0; border-radius: 7px; background: none; color: var(--text); cursor: pointer; font: inherit; font-size: 12px; }
  .menu-choice .menu-icon { width: 18px; height: 18px; }
  .menu-choice:hover { background: var(--line); }
  .menu-choice.on { background: var(--tint); color: white; }
  .sheet-text { margin: 4px 4px 16px; color: var(--muted); }
  .sheet-actions { display: flex; justify-content: flex-end; gap: 8px; padding-bottom: 8px; }`;
  (document.head || document.documentElement).append(Object.assign(document.createElement("style"), { textContent: css }));
  const esc = (s) => one.esc(s);

  /// `one.menu` from the screen in `frame`: a small menu at its point in the frame, or beside the element it's for
  /// (`rect`) → the index chosen, or null once it's closed by a click elsewhere or Escape. Its `row` items lead, side by
  /// side as their icons; a `section` starts after a line.
  function menu(frame, m) {
    return new Promise((done) => {
      document.querySelector(".menu-backdrop")?.click();
      const entries = Array.isArray(m.items) ? m.items : [];
      if (!entries.length) return done(null);
      // Over everything, screens' frames too, so a click anywhere else closes it.
      const back = document.createElement("div");
      back.className = "menu-backdrop";
      const el = document.createElement("div");
      el.className = "menu";
      el.tabIndex = -1;
      const icon = (e) => (e.icon ? one.icon(String(e.icon), "menu-icon") : "");
      const checks = entries.some((e) => e.checked && !e.row);
      const row = entries.map((e, i) => [e, i]).filter(([e]) => e.row);
      el.innerHTML = (row.length ? `<div class="menu-row">${row.map(([e, i]) => `<button class="menu-choice${e.checked ? " on" : ""}" data-i="${i}" title="${esc(e.title)}" aria-label="${esc(e.title)}">${icon(e) || esc(e.title)}</button>`).join("")}</div>` : "") +
        entries.map((e, i) => (e.row ? "" : `${e.section && i > row.length ? '<div class="menu-line"></div>' : ""}<button class="menu-item${e.danger ? " danger" : ""}" data-i="${i}">` +
          `${checks ? `<span class="menu-check">${e.checked ? "✓" : ""}</span>` : ""}<span class="menu-title">${esc(e.title)}</span>${icon(e)}</button>`)).join("");
      back.append(el);
      document.body.append(back);
      // Its point in the frame, in the window: beside its element, opening away from the edge it's near.
      const box = frame.getBoundingClientRect();
      const r = Array.isArray(m.rect) && m.rect.length === 4 ? m.rect.map(Number) : null;
      let x = box.left + Number(m.x || 0), y = box.top + Number(m.y || 0);
      if (r) {
        const top = box.top + r[1], bottom = top + r[3];
        x = box.left + r[0] + r[2] / 2 - el.offsetWidth / 2;
        y = bottom + el.offsetHeight + 8 > innerHeight && top - el.offsetHeight - 6 > 0 ? top - el.offsetHeight - 6 : bottom + 6;
      }
      el.style.top = `${Math.max(8, Math.min(y, innerHeight - el.offsetHeight - 8))}px`;
      el.style.left = `${Math.max(8, Math.min(x, innerWidth - el.offsetWidth - 8))}px`;
      const close = (i) => { back.remove(); done(i); };
      back.addEventListener("mousedown", (e) => e.target === back && close(null));
      back.addEventListener("click", (e) => e.target === back && close(null));
      back.addEventListener("contextmenu", (e) => { e.preventDefault(); close(null); });
      el.addEventListener("keydown", (e) => e.key === "Escape" && close(null));
      el.focus();
      el.querySelectorAll("[data-i]").forEach((b) => (b.onclick = () => close(Number(b.dataset.i))));
    });
  }

  /// `one.alert`: its title and text, then OK; or Cancel and its action → whether that was chosen.
  function alert(m) {
    return new Promise((done) => {
      let chose = false;
      const { body, close } = one.sheet({ title: String(m.title || ""), done: m.action ? "Cancel" : "OK", onClose: () => done(chose) });
      body.innerHTML = (m.text ? `<p class="sheet-text">${esc(m.text)}</p>` : "") +
        (m.action ? `<div class="sheet-actions"><button class="button ${m.danger ? "danger" : "primary"}" data-yes>${esc(m.action)}</button></div>` : "");
      const yes = body.querySelector("[data-yes]");
      if (yes) yes.onclick = () => { chose = true; close(); };
    });
  }

  /// `one.save` and `one.preview`: a download, or a tab showing it. Only what can't run script shows in a tab, as
  /// this page's: anything else is saved. `blob(id)` gives a file the node keeps (`{ blob }`), where the page can reach
  /// it.
  async function file(how, f, blob) {
    let data;
    if (f.base64) data = new Blob([Uint8Array.from(atob(f.base64), (c) => c.charCodeAt(0))]);
    else if (f.blob || f.file) data = await blob(String(f.blob || f.file));
    else throw new Error("No file to show.");
    const type = String(f.type || "");
    const viewable = /^(image\/(png|jpeg|gif|webp|avif)|application\/pdf|text\/plain|audio\/|video\/)/.test(type);
    const url = URL.createObjectURL(new Blob([data], { type: viewable ? type : "application/octet-stream" }));
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    if (how === "preview" && viewable) return void window.open(url, "_blank", "noopener");
    const a = Object.assign(document.createElement("a"), { href: url, download: f.name || "file" });
    document.body.append(a);
    a.click();
    a.remove();
  }

  /// Light or dark whatever the system says: a document's rules for the other are switched off and its own on, and
  /// "automatic" puts them back. Its source also runs in screens' frames (`look`), so it keeps to itself.
  function scheme() {
    const kept = new Map();
    return (doc, value) => {
      doc.documentElement.style.colorScheme = value === "automatic" ? "" : value;
      const walk = (rules) => {
        for (const rule of rules) {
          if (rule.media && /prefers-color-scheme/.test(kept.get(rule) ?? rule.media.mediaText)) {
            if (!kept.has(rule)) kept.set(rule, rule.media.mediaText);
            rule.media.mediaText = value === "automatic" ? kept.get(rule)
              : kept.get(rule).replace(/\(\s*prefers-color-scheme\s*:\s*(light|dark)\s*\)/g, (_, s) => (s === value ? "(min-width: 0px)" : "(max-width: 0px) and (min-width: 1px)"));
          }
          if (rule.cssRules) walk(rule.cssRules);
        }
      };
      for (const sheet of doc.styleSheets) {
        try { walk(sheet.cssRules); } catch {}
      }
    };
  }

  /// What makes a screen's page sit in this one as its own: this page's background shows through, and on a computer
  /// the Mac app's size of text. It's light or dark as `appearance` says ("automatic", "light" or "dark"), and as a
  /// `{ oneScheme }` posted to its frame says after. In a sheet (`raised`), its cards are a step lighter, as on a phone's.
  function look(html, appearance, raised = false) {
    // With the nonce of a page that runs only its own scripts (the client's own screens), as one of them.
    const nonce = html.match(/^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="script-src 'nonce-([0-9a-f]+)'/)?.[1];
    const looks = `<script${nonce ? ` nonce="${nonce}"` : ""}>(() => { ${raised ? 'document.documentElement.setAttribute("data-raised", "");' : ""} const scheme = (${scheme})(); let now = ${JSON.stringify(appearance)}; const apply = () => scheme(document, now);
      addEventListener("DOMContentLoaded", apply); addEventListener("load", apply);
      addEventListener("message", (e) => { if (e.source === parent && e.data?.oneScheme) apply(now = e.data.oneScheme); }); })()<\/script>`;
    const style = `<style>html, body { background: transparent !important; }
      @media (pointer: fine) { html { zoom: calc(13 / 17); } :root:root { --bg: transparent; } }</style>`;
    return html.includes("</head>") ? html.replace("</head>", `${style}${looks}</head>`) : style + looks + html;
  }

  /// The bridge's messages (src/screen/bridge.js) from a screen, answered as every web page that holds screens
  /// answers them, the web client's and a chat's alike → what it replies; what it throws, it refuses with. `h` is what
  /// each page does its own way: `frame`, the screen's; `call(op, input)`, as its app; `open(m)`, `close(value)` and
  /// `title(text, m)`; `kept(op, input)`, what that call last answered if this page keeps it (`{ answer, at }`, or null); `vault(need)` → whether it's set; `link(url)`; `icons(names)`; `blob(id)`, a file the node
  /// keeps; `elsewhere()`, on a page that can't give files (a chat's), the screen opened where it can; and `own`, what
  /// only the client's own screens may do, by message, on a page that has them.
  async function answer(m, h) {
    switch (m.type) {
      case "call": return h.call(String(m.op || ""), m.input ?? {});
      case "kept": return h.kept ? h.kept(String(m.op || ""), m.input ?? {}) : null;
      case "open": return h.open(m);
      case "close": return h.close(m.value);
      case "title": return h.title(String(m.text ?? ""), m);
      case "menu": return menu(h.frame, m);
      case "alert": return alert(m);
      case "save": case "preview":
        if (h.elsewhere) return h.elsewhere();
        await file(m.type, m.file || {}, h.blob);
        return null;
      case "copy":
        await navigator.clipboard.writeText(String(m.text ?? ""));
        return null;
      case "link": return h.link(String(m.url ?? ""));
      case "vault": return h.vault(String(m.need || ""));
      case "icons": return h.icons(Array.isArray(m.names) ? m.names.map(String) : []);
      case "scan": throw new Error("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.");
      case "icon": case "haptic": return null;
      case "api": case "signOut": case "appearance": case "device": case "request": case "cache":
        if (!h.own) throw new Error("Only Settings can do that.");
        return h.own[m.type](m);
      default: throw new Error("Unknown message");
    }
  }

  return { answer, menu, alert, file, scheme, look };
})();
