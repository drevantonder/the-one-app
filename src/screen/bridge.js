// The bridge every app screen gets as `one`. Inside a shell, the shell answers it (WKWebView's
// `one` message handler, WebView2's `chrome.webview`); in a plain browser on the device it talks
// to the node directly. A screen never knows which.
(() => {
  const ctx = window.__one;
  const handlers = { changed: [] };
  const shell = window.webkit?.messageHandlers?.one;

  async function host(message) {
    if (shell) return shell.postMessage(message);
    if (message.type === "call") {
      const r = await fetch("/v1/call", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ space: ctx.space, app: message.app, op: message.op, input: message.input }),
      });
      const body = await r.json();
      if (body.error) throw new Error(body.error);
      return body.ok;
    }
    if (message.type === "open") {
      const q = new URLSearchParams({ space: ctx.space, params: JSON.stringify(message.params || {}) });
      location.href = `/v1/screens/${ctx.app}/${message.screen}?${q}`;
    }
    if (message.type === "title") document.title = message.text;
  }

  // Browser only: the node's events, to refresh when this app's data changes.
  if (!shell && window.EventSource) {
    new EventSource("/v1/events").onmessage = (m) => one.__event(JSON.parse(m.data));
  }

  const colors = ["#5e5ce6", "#ff9f0a", "#30b0c7", "#ff375f", "#34c759", "#af52de", "#0a84ff", "#a2845e"];

  window.one = {
    app: ctx.app, screen: ctx.screen, params: ctx.params, space: ctx.space, me: ctx.me,
    call: (op, input = {}) => host({ type: "call", app: ctx.app, op, input }),
    open: (screen, params = {}) => host({ type: "open", screen, params }),
    title: (text) => host({ type: "title", text }),
    on: (event, fn) => (handlers[event] ||= []).push(fn),
    // The shell calls this with node events.
    __event(e) {
      if (e.type === "changed" && e.space === ctx.space && (e.apps || []).includes(ctx.app)) handlers.changed.forEach((f) => f(e));
    },
    esc: (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]),
    ago(ms, now = Date.now()) {
      const s = Math.max(0, (now - ms) / 1000);
      if (s < 60) return "now";
      if (s < 3600) return `${Math.floor(s / 60)} min`;
      if (s < 86400) return `${Math.floor(s / 3600)} h`;
      const d = new Date(ms);
      return s < 7 * 86400 ? d.toLocaleDateString(undefined, { weekday: "short" }) : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
    },
    // A steady colour and initials for someone, from their id and name.
    avatar(id, name) {
      let h = 0;
      for (const c of String(id)) h = (h * 31 + c.charCodeAt(0)) >>> 0;
      const initials = String(name || "?").split(/\s+/).map((w) => w[0]).join("").slice(0, 2).toUpperCase();
      return `<div class="avatar" style="background:${colors[h % colors.length]}">${one.esc(initials)}</div>`;
    },
  };
})();
