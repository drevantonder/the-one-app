// What a web page that holds screens does for them, the same in the web client (cloudflare/web/web.js) and in a chat's
// MCP view (src/mcp/view.html): their look in its frames, and the answer to each of the bridge's messages, with its
// menu, alert and files drawn in HTML where a native shell uses its own: @one/ui's. Its functions need @one/ui and the
// bridge on the page, though it may load before them.
window.oneHost = (() => {
  const ui = () => window.OneUI["@one/ui"];

  /// `one.menu` from the screen in `frame`: a small menu at its point in the frame, or beside the element it's for
  /// (`rect`) → the index chosen, or null once it's closed by a click elsewhere or Escape. Its `row` items lead, side by
  /// side as their icons; a `section` starts after a line.
  function menu(frame, m) {
    const box = frame.getBoundingClientRect();
    const r = Array.isArray(m.rect) && m.rect.length === 4 ? m.rect.map(Number) : [Number(m.x || 0), Number(m.y || 0), 0, 0];
    const items = (Array.isArray(m.items) ? m.items : []).map((e) => ({ title: String(e.title ?? ""), icon: e.icon ? String(e.icon) : undefined, danger: !!e.danger, checked: !!e.checked, row: !!e.row, section: !!e.section }));
    return ui().showMenu(items, [box.left + r[0], box.top + r[1], r[2], r[3]]);
  }

  /// `one.alert`: its title and text, then OK; or Cancel and its action → whether that was chosen.
  function alert(m) {
    return ui().showAlert({ title: String(m.title || ""), text: String(m.text || ""), action: m.action ? String(m.action) : undefined, danger: !!m.danger });
  }

  /// `one.save` and `one.preview`: a download, or a tab showing it. Only what can't run script shows in a tab, as
  /// this page's: anything else is saved. `blob(id)` gives a file the node keeps (`{ blob }`), where the page can reach
  /// it. `give(how, name, blob)`, where the page can't download (the native app), gives it instead.
  async function file(how, f, blob, give) {
    let data;
    if (f.base64) data = new Blob([Uint8Array.from(atob(f.base64), (c) => c.charCodeAt(0))]);
    else if (f.blob || f.file) data = await blob(String(f.blob || f.file));
    else throw new Error("No file to show.");
    const type = String(f.type || "");
    const viewable = /^(image\/(png|jpeg|gif|webp|avif)|application\/pdf|text\/plain|audio\/|video\/)/.test(type);
    if (give) return give(how === "preview" && viewable ? "preview" : "save", f.name || "file", new Blob([data], { type }));
    const url = URL.createObjectURL(new Blob([data], { type: viewable ? type : "application/octet-stream" }));
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    if (how === "preview" && viewable) return void window.open(url, "_blank", "noopener");
    const a = Object.assign(document.createElement("a"), { href: url, download: f.name || "file" });
    document.body.append(a);
    a.click();
    a.remove();
  }

  /// Light or dark whatever the system says: a document's rules for the other are switched off and its own on, and
  /// "automatic" puts them back. The browser's theme colours are the same rules, by their `media`. Its source also runs in
  /// screens' frames (`look`), so it keeps to itself.
  function scheme() {
    const kept = new Map();
    const chosen = (media, value) => value === "automatic" ? media
      : media.replace(/\(\s*prefers-color-scheme\s*:\s*(light|dark)\s*\)/g, (_, s) => (s === value ? "(min-width: 0px)" : "(max-width: 0px) and (min-width: 1px)"));
    return (doc, value) => {
      doc.documentElement.style.colorScheme = value === "automatic" ? "" : value;
      const walk = (rules) => {
        for (const rule of rules) {
          if (rule.media && /prefers-color-scheme/.test(kept.get(rule) ?? rule.media.mediaText)) {
            if (!kept.has(rule)) kept.set(rule, rule.media.mediaText);
            rule.media.mediaText = chosen(kept.get(rule), value);
          }
          if (rule.cssRules) walk(rule.cssRules);
        }
      };
      for (const sheet of doc.styleSheets) {
        try { walk(sheet.cssRules); } catch {}
      }
      for (const meta of doc.querySelectorAll("meta[name=theme-color][media]")) {
        if (!kept.has(meta)) kept.set(meta, meta.media);
        meta.media = chosen(kept.get(meta), value);
      }
    };
  }

  /// What a page in the app's window has that a browser's doesn't: no browser menu but on text being worked with, no
  /// reloading, nothing dragged out but what asks to be. Its source also runs in screens' frames (`look`), so it keeps to
  /// itself.
  function appBehaviour() {
    addEventListener("contextmenu", (e) => {
      const editing = e.target.closest?.("input, textarea, select, [contenteditable]");
      if (!editing && !String(getSelection())) e.preventDefault();
    }, true);
    addEventListener("keydown", (e) => {
      if (e.key === "F5" || ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "r")) e.preventDefault();
    }, true);
    addEventListener("dragstart", (e) => {
      if (e.target.closest?.("a, img") && !e.target.closest("[draggable=true]")) e.preventDefault();
    }, true);
  }

  /// `appBehaviour`'s rules: the page stays where it is and the size it is, and a long press on a phone is the screen's.
  const APP_RULES = `html, body { overscroll-behavior: none; touch-action: pan-x pan-y; } a, img { -webkit-touch-callout: none; }`;

  /// What makes a screen's page sit in this one as its own: this page's background shows through, and on a computer
  /// the Mac app's size of text. It's light or dark as `appearance` says ("automatic", "light" or "dark"), and as a
  /// `{ oneScheme }` posted to its frame says after. In a sheet (`raised`), its cards are a step lighter, as on a phone's.
  /// `shell` is what the page that holds it does beyond a browser, as its globals (`__oneApp`, `__oneCan`, `__oneLifts`).
  function look(html, appearance, raised = false, shell = {}) {
    // With the nonce of a page that runs only its own scripts (the client's own screens), as one of them.
    const nonce = html.match(/^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="script-src 'nonce-([0-9a-f]+)'/)?.[1];
    const looks = `<script${nonce ? ` nonce="${nonce}"` : ""}>(() => { Object.assign(window, ${JSON.stringify(shell).replace(/</g, "\\u003c")}); ${raised ? 'document.documentElement.setAttribute("data-raised", "");' : ""} ${shell.__oneApp ? `(${appBehaviour})();` : ""} const scheme = (${scheme})(); let now = ${JSON.stringify(appearance)}; const apply = () => scheme(document, now); if (now !== "automatic") document.documentElement.style.colorScheme = now;
      addEventListener("DOMContentLoaded", apply); addEventListener("load", apply);
      addEventListener("message", (e) => { if (e.source === parent && e.data?.oneScheme) apply(now = e.data.oneScheme); }); })()<\/script>`;
    const style = `<style>html, body { background: transparent !important; } html { touch-action: manipulation; }
      @media (pointer: fine) { html { zoom: calc(13 / 17); } :root:root { --bg: transparent; } }
      ${shell.__oneApp ? APP_RULES : ""}</style>`;
    return html.includes("</head>") ? html.replace("</head>", `${style}${looks}</head>`) : style + looks + html;
  }

  /// The bridge's messages (src/screen/bridge.js) from a screen, answered as every web page that holds screens
  /// answers them, the web client's and a chat's alike → what it replies; what it throws, it refuses with. `h` is what
  /// each page does its own way: `frame`, the screen's; `call(tool, input)`, as its app; `open(m)`, `opening(m)` (a press that will `open` has come down, where it can start early), `close(value)` and
  /// `title(text, m)`; `kept(tool, input)`, what that call last answered if this page keeps it (`{ answer, at }`, or null); `vault(need)` → whether it's set; `link(url)`; `icons(names)`; `blob(id)`, a file the node
  /// keeps; `give`, `copy`, `menu`, `scan` and `haptic`, where the page does those its own way (the native app); `elsewhere()`, on a page that can't give files (a chat's), the screen opened where it can; `traces()`, in
  /// developer mode, its latest requests' traces (else null); `thrown(m)`, what threw in the screen (its type and where), for a page that reports it; and `own`, what only the client's own screens may do, by
  /// message, on a page that has them.
  async function answer(m, h) {
    switch (m.type) {
      case "call": return h.call(String(m.tool || ""), m.input ?? {});
      case "kept": return h.kept ? h.kept(String(m.tool || ""), m.input ?? {}) : null;
      case "open": return h.open(m);
      case "opening": return h.opening ? h.opening(m) : null;
      case "close": return h.close(m.value);
      case "title": return h.title(String(m.text ?? ""), m);
      case "menu": return h.menu ? h.menu(m) : menu(h.frame, m);
      case "alert": return alert(m);
      case "save": case "preview":
        if (h.elsewhere) return h.elsewhere();
        await file(m.type, m.file || {}, h.blob, h.give);
        return null;
      case "copy":
        await (h.copy ? h.copy(String(m.text ?? "")) : navigator.clipboard.writeText(String(m.text ?? "")));
        return null;
      case "link": return h.link(String(m.url ?? ""));
      case "vault": return h.vault(String(m.need || ""));
      case "icons": return h.icons(Array.isArray(m.names) ? m.names.map(String) : []);
      case "scan":
        if (h.scan) return h.scan();
        throw new Error("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.");
      case "haptic": return h.haptic ? h.haptic(String(m.style || "selection")) : null;
      case "traces": return h.traces ? h.traces() : null;
      case "thrown": return h.thrown ? h.thrown(m) : null;
      case "api": case "signOut": case "appearance": case "device": case "notify": case "request": case "cache": case "developer":
        if (!h.own) throw new Error("Only Settings can do that.");
        return h.own[m.type](m);
      default: throw new Error("Unknown message");
    }
  }

  return { answer, menu, alert, file, scheme, look, appBehaviour, APP_RULES };
})();
