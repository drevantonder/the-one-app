// Widgets drawn from their trees (`GET /v1/widgets`), in a web page: the web client's pages and `one mcp`'s view in
// chats. The Apple shells draw the same trees natively (apple/Shared/Views/WidgetTree.swift). Nodes it doesn't know
// draw nothing. Its styles come with it. `host` is what buttons, toggles and links act through: `run(app, op, input)` → the error's text if it
// failed, and `follow(link, widget)` for a web page's address or `{ screen, params }`.
window.oneWidgets = (() => {
  const css = `
  .w-card { display: flex; flex-direction: column; gap: 6px; padding: 16px; border-radius: 22px; background: var(--w-card, rgba(128, 128, 128, 0.12)); overflow: hidden; min-width: 0; font-size: 12px; line-height: 1.3; }
  .w-card > .w-title { font-size: 11px; font-weight: 500; color: var(--w-muted, #8e8e93); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .w-card .w-body { flex: 1; min-height: 0; display: flex; flex-direction: column; }
  .w-col { display: flex; flex-direction: column; gap: var(--gap, 4px); min-width: 0; }
  .w-row { display: flex; align-items: center; gap: var(--gap, 6px); min-width: 0; }
  .w-row > * { min-width: 0; }
  .w-row > .w-text { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .w-xl { font-size: 22px; font-weight: 600; } .w-lg { font-size: 15px; font-weight: 600; } .w-sm { font-size: 11px; } .w-xs { font-size: 10px; }
  .w-medium { font-weight: 600; } .w-bold { font-weight: 700; }
  .w-muted { color: var(--w-muted, #8e8e93); } .w-accent { color: var(--accent); } .w-good { color: #34c759; } .w-warn { color: #ff9f0a; } .w-bad { color: #ff3b30; } .w-blue { color: #0a9edc; } .w-purple { color: #a550d9; }
  .w-stat b { font-size: 26px; font-weight: 600; font-variant-numeric: tabular-nums; } .w-stat small { color: var(--w-muted, #8e8e93); font-size: 11px; display: block; }
  .w-bar { height: 6px; border-radius: 3px; background: rgba(128, 128, 128, 0.22); overflow: hidden; } .w-bar > i { display: block; height: 100%; background: var(--tone, var(--accent)); }
  .w-list > .w-item { display: flex; align-items: center; gap: 8px; padding: 5px 0; border-top: 0.5px solid rgba(128, 128, 128, 0.25); } .w-list > .w-item:first-child { border-top: 0; }
  .w-item > div { min-width: 0; } .w-item .w-name { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .w-item .w-sub { font-size: 11px; color: var(--w-muted, #8e8e93); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; } .w-item .w-end { margin-left: auto; flex: none; color: var(--w-muted, #8e8e93); font-variant-numeric: tabular-nums; }
  .w-dot { width: 8px; height: 8px; border-radius: 50%; background: var(--tone, #34c759); flex: none; }
  .w-card hr { border: 0; border-top: 0.5px solid rgba(128, 128, 128, 0.3); margin: 4px 0; width: 100%; }
  .w-link { color: var(--accent); cursor: pointer; text-decoration: none; }
  .w-button { font: inherit; border: 0; border-radius: 999px; padding: 4px 11px; background: rgba(128, 128, 128, 0.18); color: var(--tone, var(--accent)); cursor: pointer; }
  .w-button:disabled { opacity: .5; }
  label.w-toggle { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex: 1; }
  .w-linked { cursor: pointer; }
  .w-broken { color: #ff9f0a; font-weight: 500; font-size: 13px; } .w-why { color: var(--w-muted, #8e8e93); font-size: 11px; overflow-wrap: anywhere; user-select: text; }`;

  const tones = { muted: "var(--w-muted, #8e8e93)", accent: "var(--accent)", good: "#34c759", warn: "#ff9f0a", bad: "#ff3b30", blue: "#0a9edc", purple: "#a550d9" };
  const fmt = (v) => typeof v === "number" ? v.toLocaleString(undefined, { maximumFractionDigits: 2 }) : String(v ?? "–");

  function node(tag, cls, text) {
    const el = document.createElement(tag);
    if (cls) el.className = cls;
    if (text !== undefined) el.textContent = text;
    return el;
  }

  function svg(attrs) {
    const el = document.createElementNS("http://www.w3.org/2000/svg", "svg");
    for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
    return el;
  }

  function time(n) {
    const d = new Date(n.at || 0), s = (Date.now() - d) / 1000;
    if (n.style === "date") return d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
    if (n.style === "clock") return d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
    if (n.style === "timer") { const a = Math.abs(Math.round(s)); return `${Math.floor(a / 60)}:${String(a % 60).padStart(2, "0")}`; }
    if (n.style === "relative") return new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" }).format(...(Math.abs(s) < 3600 ? [-Math.round(s / 60), "minute"] : Math.abs(s) < 86400 ? [-Math.round(s / 3600), "hour"] : [-Math.round(s / 86400), "day"]));
    if (s < 60) return "now";
    if (s < 3600) return `${Math.floor(s / 60)} min`;
    if (s < 86400) return `${Math.floor(s / 3600)} h`;
    return d.toLocaleDateString(undefined, s < 7 * 86400 ? { weekday: "short" } : { day: "numeric", month: "short" });
  }

  // A widget, as a card: its tree under its title (unless the tree labels itself); a broken one says so, and why.
  function card(w, host, tree = w.root) {
    const el = node("div", "w-card");
    if (tree?.header !== false || w.error) el.append(node("div", "w-title", w.title || w.widget));
    if (w.error) {
      el.append(node("div", "w-broken", "This widget stopped working."), node("div", "w-why", w.error));
      return el;
    }
    const body = node("div", "w-body");
    body.append(draw(tree, w, host));
    el.append(body);
    if (tree?.link) { el.classList.add("w-linked"); el.addEventListener("click", (e) => { if (!e.defaultPrevented) host.follow(tree.link, w); }); }
    return el;
  }

  // A node of a widget's tree.
  function draw(n, w, host) {
    if (!n || typeof n !== "object") return document.createTextNode("");
    const tone = (el) => { if (n.tone) el.classList.add("w-" + n.tone); if (tones[n.tone]) el.style.setProperty("--tone", tones[n.tone]); return el; };
    const kids = (el) => { el.append(...(n.c || []).map((c) => draw(c, w, host))); if (n.gap != null) el.style.setProperty("--gap", n.gap + "px"); return el; };
    const cls = (...names) => names.filter(Boolean).map((c) => "w-" + c).join(" ");
    let el;
    switch (n.t) {
      case "col": case "row": {
        el = kids(node("div", cls(n.t)));
        if (n.w) el.style.cssText += `width:${n.w}px;flex:none;`;
        if (n.grow) el.style.flex = "1";
        if (n.align === "center") el.style.alignItems = "center";
        if (n.dim) el.style.opacity = ".5";
        break;
      }
      case "text":
        el = tone(node("span", cls("text", n.size, n.weight), n.text ?? ""));
        if (n.lines) el.style.cssText += `display:-webkit-box;-webkit-box-orient:vertical;-webkit-line-clamp:${n.lines};overflow:hidden;`;
        break;
      case "time": el = tone(node("span", cls("text", n.size), time(n))); break;
      case "stat": {
        el = tone(node("div", "w-stat"));
        el.append(node("b", "", fmt(n.value) + (n.unit ? " " + n.unit : "")));
        if (n.label) el.append(node("small", "", n.label));
        break;
      }
      case "gauge": {
        const f = Math.max(0, Math.min(1, (Number(n.value) || 0) / (n.max || 1))), r = 18, c = 2 * Math.PI * r;
        el = svg({ viewBox: "0 0 46 46", width: n.w || 46, height: n.w || 46 });
        el.innerHTML = `<circle cx="23" cy="23" r="${r}" fill="none" stroke="rgba(128,128,128,.22)" stroke-width="5"/>
          <circle cx="23" cy="23" r="${r}" fill="none" stroke="${tones[n.tone] || "var(--accent)"}" stroke-width="5" stroke-linecap="round"
            stroke-dasharray="${c * f} ${c}" transform="rotate(-90 23 23)"/>
          <text x="23" y="27" text-anchor="middle" font-size="10" font-weight="600" fill="currentColor"></text>`;
        el.querySelector("text").textContent = n.label ?? Math.round(f * 100) + "%";
        break;
      }
      case "progress": {
        el = node("div", "w-col");
        if (n.label) el.append(node("span", "w-sm w-muted", n.label));
        if (n.w) el.style.width = n.w + "px";
        const bar = tone(node("div", "w-bar")), fill = node("i");
        fill.style.width = Math.max(0, Math.min(1, (Number(n.value) || 0) / (n.max || 1))) * 100 + "%";
        bar.append(fill);
        el.append(bar);
        break;
      }
      case "spark": case "chart": {
        const v = (n.values || []).map(Number), max = n.max || Math.max(...v, 1), h = n.h || (n.t === "spark" ? 28 : 70), wd = n.t === "spark" && n.w ? n.w : 240;
        el = svg({ viewBox: `0 0 ${wd} ${h + (n.labels ? 14 : 0)}`, width: n.t === "spark" && n.w ? n.w : "100%" });
        if (n.t === "spark") el.setAttribute("preserveAspectRatio", "none");
        const color = tones[n.tone] || "var(--accent)", step = wd / Math.max(v.length, 1);
        if (n.t === "spark" && !v.length) {
          el.innerHTML = `<line x1="2" y1="${h / 2}" x2="${wd - 2}" y2="${h / 2}" stroke="rgba(128,128,128,.4)" stroke-width="1.5" stroke-dasharray="2 3"/>`;
        } else if (n.t === "spark") {
          const pts = v.map((y, i) => [v.length > 1 ? (i / (v.length - 1)) * wd : wd / 2, h - 1 - (Math.min(y, max) / max) * (h - 2)]);
          const line = pts.map((p) => p.join(",")).join(" ");
          el.innerHTML = `<polygon points="${line} ${wd},${h} 0,${h}" fill="${color}" opacity=".2"/><polyline points="${line}" fill="none" stroke="${color}" stroke-width="1.4" stroke-linejoin="round"/>`;
        } else if (n.kind === "line") {
          const pts = v.map((y, i) => `${(i + 0.5) * step},${h - (y / max) * (h - 4) - 2}`).join(" ");
          el.innerHTML = `<polyline points="${pts}" fill="none" stroke="${color}" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/>`;
        } else {
          el.innerHTML = v.map((y, i) => `<rect x="${i * step + step * 0.15}" y="${h - (y / max) * h}" width="${step * 0.7}" height="${(y / max) * h}" rx="3" fill="${color}"/>`).join("");
        }
        (n.labels || []).forEach((l, i) => {
          const t = document.createElementNS("http://www.w3.org/2000/svg", "text");
          for (const [k, val] of Object.entries({ x: (i + 0.5) * step, y: h + 12, "text-anchor": "middle", "font-size": "10", fill: "var(--w-muted, #8e8e93)" })) t.setAttribute(k, val);
          t.textContent = l;
          el.append(t);
        });
        break;
      }
      case "list":
        el = node("div", "w-list");
        if (!(n.c || []).length) el.append(node("div", "w-muted", n.empty || "Nothing here"));
        el.append(...(n.c || []).map((c) => draw(c, w, host)));
        break;
      case "item": {
        el = tone(node("div", "w-item"));
        if (n.icon) el.append(tone(node("span", "w-dot")));
        const mid = node("div");
        mid.append(node("div", "w-name", n.title || ""));
        if (n.subtitle) mid.append(node("div", "w-sub", n.subtitle));
        el.append(mid);
        if (n.at || n.trailing) el.append(node("span", "w-end", n.at ? time({ at: n.at }) : n.trailing));
        break;
      }
      case "image":
        el = node("img");
        el.src = /^(https:|data:image\/)/.test(n.url || "") ? n.url : "";
        el.alt = "";
        el.style.cssText = `width:${n.w ? n.w + "px" : "100%"};height:${n.h ? n.h + "px" : "auto"};object-fit:cover;border-radius:${n.round ? "50%" : "8px"};flex:none`;
        break;
      case "dot": el = tone(node("span", "w-dot")); break;
      case "icon": {
        // Named icons are the Apple shells' symbols; here only a path and a badge (SVG paths) can be drawn.
        const size = n.w || 16;
        el = svg({ viewBox: "0 0 24 24", width: size, height: size });
        el.style.flex = "none";
        el.style.color = tones[n.tone] || "currentColor";
        const path = (d, attrs) => { const p = document.createElementNS("http://www.w3.org/2000/svg", "path"); p.setAttribute("d", d); for (const [k, v] of Object.entries(attrs)) p.setAttribute(k, v); el.append(p); };
        if (n.path) path(n.path, { fill: "none", stroke: "currentColor", "stroke-width": "1.5", "stroke-linejoin": "round" });
        if (n.badge) { path(n.badge, { fill: "currentColor", transform: n.path || n.name ? "translate(15.5 15.5) scale(.458)" : "" }); el.style.overflow = "visible"; }
        break;
      }
      case "link": el = node("a", "w-link", n.text || ""); break;
      case "divider": el = node("hr"); break;
      case "button": {
        el = tone(node("button", "w-button", n.label || n.text || ""));
        el.addEventListener("click", (e) => { e.preventDefault(); e.stopPropagation(); act(el, n.op, n.input || {}); });
        break;
      }
      case "toggle": {
        el = node("label", "w-toggle");
        const box = Object.assign(document.createElement("input"), { type: "checkbox", checked: !!n.on });
        box.addEventListener("click", (e) => e.stopPropagation());
        box.onchange = () => act(box, n.op, { ...(n.input || {}), on: box.checked });
        el.append(node("span", "", n.label || ""), box);
        el.addEventListener("click", (e) => e.stopPropagation());
        break;
      }
      default: return document.createTextNode("");
    }
    if (n.link) { el.classList?.add("w-linked"); el.addEventListener("click", (e) => { e.preventDefault(); e.stopPropagation(); host.follow(n.link, w); }); }
    return el;

    // A button or toggle: runs the app's operation as the person; the host draws the widgets again.
    async function act(target, op, input) {
      target.disabled = true;
      const error = await host.run(w.app, op, input);
      target.disabled = false;
      target.title = error || "";
    }
  }

  document.head.append(Object.assign(document.createElement("style"), { textContent: css }));
  return { card, draw };
})();
