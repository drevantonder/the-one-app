// one.markdown(text, opts): Markdown as HTML the way GitHub draws it (headings, lists and task lists, quotes and
// alerts, tables, code with highlighting and a copy button, links), in a `.md` block. Raw HTML is never drawn, only
// its text. ```mermaid blocks become diagrams once the page shows them, or stay code if Mermaid can't draw them.
//
// opts.link(href) may return a script to run instead of following a relative link; opts.image(src) a URL for a
// relative image (otherwise its alt text shows).
(() => {
  const esc = one.esc;
  const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/;
  const HEADING = /^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$/;
  const RULE = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/;
  const QUOTE = /^ {0,3}> ?/;
  const ITEM = /^( {0,3})([-*+]|\d{1,9}[.)])(\s+|$)/;
  const HTML = /^ {0,3}<\/?[a-zA-Z!?]/;
  const DIVIDER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/;
  const starts = (l) => FENCE.test(l) || HEADING.test(l) || RULE.test(l) || QUOTE.test(l) || ITEM.test(l) || HTML.test(l);
  const slug = (s) => s.toLowerCase().replace(/<[^>]+>/g, "").replace(/&[a-z#0-9]+;/g, "").replace(/[^\w\- ]+/g, "").trim().replace(/\s/g, "-");

  function safeHref(href) {
    const h = href.trim().replace(/&amp;/g, "&");
    return /^(https?:|mailto:|#|\/|\.|[\w-]+(\/|\.|$))/i.test(h) && !/^\s*(javascript|data|vbscript):/i.test(h) ? h : null;
  }

  function inline(text, o) {
    const held = [];
    const hold = (html) => `\u0000${held.push(html) - 1}\u0000`;
    let s = text
      .replace(/(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/g, (_, __, code) => hold(`<code>${esc(code.replace(/^ (.*) $/, "$1"))}</code>`))
      .replace(/\\([\\`*_{}\[\]()#+\-.!|~<>])/g, (_, c) => hold(esc(c)))
      .replace(/<br\s*\/?>/gi, () => hold("<br>"))
      .replace(/<((?:https?|mailto):[^\s>]+)>/g, (_, u) => hold(`<a href="${esc(u)}">${esc(u)}</a>`))
      .replace(/<\/?[a-zA-Z][^>]*>/g, "");
    s = esc(s);
    s = s.replace(/!\[([^\]]*)\]\(\s*([^)\s]+)(?:\s+&quot;[^&]*&quot;)?\s*\)/g, (_, alt, src) => {
      const url = /^(https?:)?\/\//i.test(src) ? src : o.image?.(src.replace(/&amp;/g, "&"));
      return hold(url ? `<img src="${esc(url)}" alt="${alt}" loading="lazy">` : alt ? `<span class="muted">${alt}</span>` : "");
    });
    s = s.replace(/\[((?:[^\[\]]|\[[^\]]*\])*)\]\(\s*([^)\s]+)(?:\s+&quot;[^&]*&quot;)?\s*\)/g, (whole, label, href) => {
      const h = safeHref(href);
      if (!h) return label;
      const run = !/^(https?:|mailto:|#)/i.test(h) && o.link?.(h);
      return hold(run ? `<a href="#" onclick="${esc(run)};return false">` : `<a href="${esc(h)}">`) + label + hold("</a>");
    });
    s = s.replace(/(^|[\s(])(https?:\/\/[^\s<]*[^\s<.,:;"')\]])/g, (_, pre, u) => pre + hold(`<a href="${u}">${u}</a>`));
    s = s
      .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, "<strong>$2</strong>")
      .replace(/(^|[^\w*])\*(?=\S)([^*]*?\S)\*(?!\*)/g, "$1<em>$2</em>")
      .replace(/(^|[^\w])_(?=\S)([^_]*?\S)_(?!\w)/g, "$1<em>$2</em>")
      .replace(/~~(?=\S)([\s\S]*?\S)~~/g, "<del>$1</del>")
      .replace(/( {2,}|\\)\n/g, "<br>");
    return s.replace(/\u0000(\d+)\u0000/g, (_, i) => held[i]).replace(/\u0000(\d+)\u0000/g, (_, i) => held[i]);
  }

  function code(text, lang) {
    if (lang === "mermaid") return `<div class="mermaid" data-mermaid="${esc(text)}"><pre><code>${esc(text)}</code></pre></div>`;
    const body = one.highlight ? one.highlight(text, lang).join("\n") : esc(text);
    return `<div class="md-code">${one.copyButton ? one.copyButton(text, { label: "", cls: "md-copy" }) : ""}<pre><code>${body}</code></pre></div>`;
  }

  function cells(row) {
    return row.trim().replace(/^\||\|$/g, "").split(/(?<!\\)\|/).map((c) => c.trim());
  }

  function blocks(lines, o) {
    let out = "";
    for (let i = 0; i < lines.length; ) {
      const line = lines[i];
      let m;
      if (!line.trim()) { i++; continue; }
      if ((m = FENCE.exec(line))) {
        const fence = m[1], body = [];
        const indent = line.match(/^ */)[0].length;
        for (i++; i < lines.length && !new RegExp(`^ {0,3}${fence[0]}{${fence.length},}\\s*$`).test(lines[i]); i++) body.push(lines[i].replace(new RegExp(`^ {0,${indent}}`), ""));
        i++;
        out += code(body.join("\n"), m[2].toLowerCase());
      } else if ((m = HEADING.exec(line))) {
        const n = m[1].length, html = inline(m[2] || "", o);
        out += `<h${n} id="${slug(html)}">${html}</h${n}>`;
        i++;
      } else if (RULE.test(line)) {
        out += "<hr>";
        i++;
      } else if (QUOTE.test(line)) {
        const body = [];
        for (; i < lines.length && lines[i].trim() && (QUOTE.test(lines[i]) || !starts(lines[i])); i++) body.push(lines[i].replace(QUOTE, ""));
        const alert = /^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*$/i.exec(body[0] || "");
        if (alert) {
          const kind = alert[1].toLowerCase();
          out += `<div class="md-alert ${kind}"><p class="md-alert-title">${kind[0].toUpperCase() + kind.slice(1)}</p>${blocks(body.slice(1), o)}</div>`;
        } else out += `<blockquote>${blocks(body, o)}</blockquote>`;
      } else if (line.includes("|") && DIVIDER.test(lines[i + 1] || "") && lines[i + 1].includes("-")) {
        const align = cells(lines[i + 1]).map((c) => (c.endsWith(":") ? (c.startsWith(":") ? "center" : "right") : c.startsWith(":") ? "left" : ""));
        const row = (r, tag) => `<tr>${cells(r).map((c, k) => `<${tag}${align[k] ? ` style="text-align:${align[k]}"` : ""}>${inline(c, o)}</${tag}>`).join("")}</tr>`;
        let html = `<thead>${row(line, "th")}</thead><tbody>`;
        for (i += 2; i < lines.length && lines[i].trim() && lines[i].includes("|"); i++) html += row(lines[i], "td");
        out += `<div class="md-table"><table>${html}</tbody></table></div>`;
      } else if ((m = ITEM.exec(line))) {
        const ordered = /\d/.test(m[2]), first = parseInt(m[2], 10);
        const items = [];
        let loose = false;
        while (i < lines.length && (m = ITEM.exec(lines[i])) && /\d/.test(m[2]) === ordered) {
          const width = m[0].length === m[1].length + m[2].length ? m[0].length + 1 : m[0].length;
          const body = [lines[i].slice(m[0].length)];
          for (i++; i < lines.length; i++) {
            const l = lines[i];
            if (!l.trim()) {
              const next = lines.slice(i + 1).find((x) => x.trim());
              if (next !== undefined && next.match(/^ */)[0].length >= width) { body.push(""); loose = true; continue; }
              break;
            }
            const indent = l.match(/^ */)[0].length;
            if (indent >= width) body.push(l.slice(width));
            else if (!starts(l) && body[body.length - 1].trim()) body.push(l.trim());
            else break;
          }
          items.push(body);
          if (i < lines.length && !lines[i].trim() && ITEM.test(lines.slice(i).find((x) => x.trim()) || "")) {
            loose = true;
            while (i < lines.length && !lines[i].trim()) i++;
          }
        }
        const li = items.map((body) => {
          const task = /^\[([ xX])\]\s+/.exec(body[0]);
          if (task) body[0] = body[0].slice(task[0].length);
          let html = blocks(body, o);
          if (!loose) html = html.replace(/^<p>([\s\S]*?)<\/p>/, "$1");
          return task ? `<li class="task"><input type="checkbox" disabled${task[1] === " " ? "" : " checked"}> ${html}</li>` : `<li>${html}</li>`;
        });
        out += ordered ? `<ol${first !== 1 ? ` start="${first}"` : ""}>${li.join("")}</ol>` : `<ul>${li.join("")}</ul>`;
      } else if (/^( {4}|\t)/.test(line)) {
        const body = [];
        for (; i < lines.length && (/^( {4}|\t)/.test(lines[i]) || !lines[i].trim()); i++) body.push(lines[i].replace(/^( {4}|\t)/, ""));
        out += code(body.join("\n").replace(/\n+$/, ""), "");
      } else if (HTML.test(line)) {
        const body = [];
        for (; i < lines.length && lines[i].trim(); i++) body.push(lines[i]);
        const text = body.join("\n").replace(/<!--[\s\S]*?-->/g, "");
        if (text.replace(/<[^>]*>/g, "").trim()) out += `<p>${inline(text.replace(/<\/(p|div|h\d)>/gi, "<br>"), o).replace(/^(\s*<br>)+|(<br>\s*)+$/g, "")}</p>`;
      } else {
        const body = [line];
        for (i++; i < lines.length && lines[i].trim() && !starts(lines[i]) && !(lines[i].includes("|") && DIVIDER.test(lines[i + 1] || "")); i++) {
          if (/^ {0,3}(=+|-+)\s*$/.test(lines[i])) break;
          body.push(lines[i]);
        }
        const setext = /^ {0,3}(=+|-+)\s*$/.exec(lines[i] || "");
        if (setext && lines[i].trim()) {
          const n = setext[1][0] === "=" ? 1 : 2, html = inline(body.join("\n"), o);
          out += `<h${n} id="${slug(html)}">${html}</h${n}>`;
          i++;
        } else out += `<p>${inline(body.join("\n").replace(/^\s+/gm, ""), o)}</p>`;
      }
    }
    return out;
  }

  one.markdown = (text, o = {}) => {
    watch();
    return `<div class="md">${blocks(String(text ?? "").replace(/\r\n?/g, "\n").replace(/\t/g, "    ").split("\n"), o)}</div>`;
  };

  // Mermaid, pinned and checked, from a CDN, only once a page has a diagram to draw.
  const MERMAID = {
    src: "https://cdn.jsdelivr.net/npm/mermaid@11.12.1/dist/mermaid.min.js",
    integrity: "sha384-LlKSgo4Eo5GuF/ZrstLti44dE+GC5XAJ7TSu0Nw9Q3vIZF2QMnkRcK7BUoLabYLF",
  };
  let mermaid, watching = false;
  function load() {
    mermaid ||= new Promise((ok, fail) => {
      const s = Object.assign(document.createElement("script"), { src: MERMAID.src, integrity: MERMAID.integrity, crossOrigin: "anonymous" });
      s.onload = () => (window.mermaid ? ok(window.mermaid) : fail(new Error("no mermaid")));
      s.onerror = fail;
      document.head.append(s);
    }).then((m) => {
      const css = getComputedStyle(document.documentElement);
      const v = (name) => css.getPropertyValue(name).trim();
      const dark = matchMedia("(prefers-color-scheme: dark)").matches;
      m.initialize({
        startOnLoad: false,
        securityLevel: "strict",
        theme: "base",
        fontFamily: getComputedStyle(document.body).fontFamily,
        themeVariables: {
          darkMode: dark, fontSize: "14px", background: "transparent",
          primaryColor: v("--card") || (dark ? "#1c1c1e" : "#fff"), primaryTextColor: v("--text"), primaryBorderColor: v("--tint"),
          secondaryColor: v("--bg"), tertiaryColor: v("--bg"), lineColor: v("--muted"), textColor: v("--text"),
          noteBkgColor: v("--bg"), noteTextColor: v("--text"), noteBorderColor: v("--line"),
        },
        flowchart: { curve: "basis", htmlLabels: false },
      });
      return m;
    });
    return mermaid;
  }
  let n = 0;
  async function draw() {
    const todo = [...document.querySelectorAll(".mermaid[data-mermaid]:not([data-drawn])")];
    if (!todo.length) return;
    todo.forEach((b) => (b.dataset.drawn = "…"));
    let m;
    try {
      m = await load();
    } catch {
      todo.forEach((b) => b.classList.add("failed"));
      return;
    }
    for (const b of todo) {
      try {
        const { svg } = await m.render(`mermaid-${++n}`, b.dataset.mermaid);
        b.innerHTML = svg;
        b.dataset.drawn = "1";
        const el = b.querySelector("svg");
        el.removeAttribute("height");
        el.style.maxWidth = "100%";
        el.querySelectorAll("rect, polygon").forEach((r) => r.tagName === "rect" && !r.getAttribute("rx") && r.setAttribute("rx", "6"));
      } catch {
        b.classList.add("failed");
        document.getElementById(`dmermaid-${n}`)?.remove();
      }
    }
  }
  function watch() {
    if (watching) return;
    watching = true;
    new MutationObserver(() => draw()).observe(document.body || document.documentElement, { childList: true, subtree: true });
  }
})();
