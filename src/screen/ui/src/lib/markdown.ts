// Markdown as GitHub reads it, as blocks for the Markdown component to draw: headings, paragraphs, lists and task lists,
// quotes and alerts, tables, code, rules. Raw HTML is never drawn, only its text.
//
// A block's text is inline HTML made here: every character of the text goes through `esc`, and the only tags are the
// ones this makes (a, img, code, strong, em, del, br), so it's safe to put in a page. A link goes to http(s), mailto, a
// place in the page or a path, never to javascript:, data: or vbscript:; an image is drawn only from http(s), else
// its alt text shows.

export type Block =
  | { type: "heading"; level: number; html: string; id: string }
  | { type: "paragraph"; html: string }
  | { type: "code"; text: string; lang: string }
  | { type: "rule" }
  | { type: "quote"; blocks: Block[] }
  | { type: "alert"; kind: string; blocks: Block[] }
  | { type: "table"; align: string[]; head: string[]; rows: string[][] }
  | { type: "list"; ordered: boolean; start: number; items: Item[] }

/// A list item: its blocks, whether the list is tight (its first paragraph then has no space around it), and for a
/// task, whether it's done.
export type Item = { blocks: Block[]; tight: boolean; task?: "open" | "done" }

const esc = (s: string) => s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!)

const FENCE = /^ {0,3}(`{3,}|~{3,})\s*([^\s`]*)[^`]*$/
const HEADING = /^ {0,3}(#{1,6})(?:\s+(.*?))?\s*#*\s*$/
const RULE = /^ {0,3}([-*_])(?:\s*\1){2,}\s*$/
const QUOTE = /^ {0,3}> ?/
const ITEM = /^( {0,3})([-*+]|\d{1,9}[.)])(\s+|$)/
const HTML = /^ {0,3}<\/?[a-zA-Z!?]/
const DIVIDER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/
const starts = (l: string) => FENCE.test(l) || HEADING.test(l) || RULE.test(l) || QUOTE.test(l) || ITEM.test(l) || HTML.test(l)
const slug = (s: string) =>
  s.toLowerCase().replace(/<[^>]+>/g, "").replace(/&[a-z#0-9]+;/g, "").replace(/[^\w\- ]+/g, "").trim().replace(/\s/g, "-")

function safeHref(href: string): string | null {
  const h = href.trim().replace(/&amp;/g, "&")
  return /^(https?:|mailto:|#|\/|\.|[\w-]+(\/|\.|$))/i.test(h) && !/^\s*(javascript|data|vbscript):/i.test(h) ? h : null
}

function inline(text: string): string {
  const held: string[] = []
  const hold = (html: string) => `\u0000${held.push(html) - 1}\u0000`
  let s = text
    .replace(/(`+)([^`]|[^`][\s\S]*?[^`])\1(?!`)/g, (_, __, code: string) => hold(`<code>${esc(code.replace(/^ (.*) $/, "$1"))}</code>`))
    .replace(/\\([\\`*_{}[\]()#+\-.!|~<>])/g, (_, c: string) => hold(esc(c)))
    .replace(/<br\s*\/?>/gi, () => hold("<br>"))
    .replace(/<((?:https?|mailto):[^\s>]+)>/g, (_, u: string) => hold(`<a href="${esc(u)}">${esc(u)}</a>`))
    .replace(/<\/?[a-zA-Z][^>]*>/g, "")
  s = esc(s)
  s = s.replace(/!\[([^\]]*)\]\(\s*([^)\s]+)(?:\s+&quot;[^&]*&quot;)?\s*\)/g, (_, alt: string, src: string) =>
    hold(/^(https?:)?\/\//i.test(src) ? `<img src="${esc(src)}" alt="${alt}" loading="lazy">` : alt ? `<span class="text-muted-foreground">${alt}</span>` : ""),
  )
  s = s.replace(/\[((?:[^[\]]|\[[^\]]*\])*)\]\(\s*([^)\s]+)(?:\s+&quot;[^&]*&quot;)?\s*\)/g, (_, label: string, href: string) => {
    const h = safeHref(href)
    if (!h) return label
    // A link to a path, not to a page or an address: the Markdown component tells where it goes.
    const local = !/^(https?:|mailto:|#)/i.test(h)
    return hold(`<a href="${esc(h)}"${local ? " data-local" : ""}>`) + label + hold("</a>")
  })
  s = s.replace(/(^|[\s(])(https?:\/\/[^\s<]*[^\s<.,:;"')\]])/g, (_, pre: string, u: string) => pre + hold(`<a href="${u}">${u}</a>`))
  s = s
    .replace(/(\*\*|__)(?=\S)([\s\S]*?\S)\1/g, "<strong>$2</strong>")
    .replace(/(^|[^\w*])\*(?=\S)([^*]*?\S)\*(?!\*)/g, "$1<em>$2</em>")
    .replace(/(^|[^\w])_(?=\S)([^_]*?\S)_(?!\w)/g, "$1<em>$2</em>")
    .replace(/~~(?=\S)([\s\S]*?\S)~~/g, "<del>$1</del>")
    .replace(/( {2,}|\\)\n/g, "<br>")
  const back = (_: string, i: string) => held[Number(i)]
  return s.replace(/\u0000(\d+)\u0000/g, back).replace(/\u0000(\d+)\u0000/g, back)
}

const cells = (row: string) => row.trim().replace(/^\||\|$/g, "").split(/(?<!\\)\|/).map((c) => c.trim())

function blocks(lines: string[]): Block[] {
  const out: Block[] = []
  for (let i = 0; i < lines.length; ) {
    const line = lines[i]
    let m: RegExpExecArray | null
    if (!line.trim()) {
      i++
    } else if ((m = FENCE.exec(line))) {
      const fence = m[1]
      const body: string[] = []
      const indent = line.match(/^ */)![0].length
      const close = new RegExp(`^ {0,3}${fence[0]}{${fence.length},}\\s*$`)
      for (i++; i < lines.length && !close.test(lines[i]); i++) body.push(lines[i].replace(new RegExp(`^ {0,${indent}}`), ""))
      i++
      out.push({ type: "code", text: body.join("\n"), lang: m[2].toLowerCase() })
    } else if ((m = HEADING.exec(line))) {
      const html = inline(m[2] || "")
      out.push({ type: "heading", level: m[1].length, html, id: slug(html) })
      i++
    } else if (RULE.test(line)) {
      out.push({ type: "rule" })
      i++
    } else if (QUOTE.test(line)) {
      const body: string[] = []
      for (; i < lines.length && lines[i].trim() && (QUOTE.test(lines[i]) || !starts(lines[i])); i++) body.push(lines[i].replace(QUOTE, ""))
      const alert = /^\s*\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*$/i.exec(body[0] || "")
      if (alert) out.push({ type: "alert", kind: alert[1].toLowerCase(), blocks: blocks(body.slice(1)) })
      else out.push({ type: "quote", blocks: blocks(body) })
    } else if (line.includes("|") && DIVIDER.test(lines[i + 1] || "") && lines[i + 1].includes("-")) {
      const align = cells(lines[i + 1]).map((c) => (c.endsWith(":") ? (c.startsWith(":") ? "center" : "right") : c.startsWith(":") ? "left" : ""))
      const head = cells(line).map(inline)
      const rows: string[][] = []
      for (i += 2; i < lines.length && lines[i].trim() && lines[i].includes("|"); i++) rows.push(cells(lines[i]).map(inline))
      out.push({ type: "table", align, head, rows })
    } else if ((m = ITEM.exec(line))) {
      const ordered = /\d/.test(m[2])
      const start = parseInt(m[2], 10)
      const items: string[][] = []
      let loose = false
      while (i < lines.length && (m = ITEM.exec(lines[i])) && /\d/.test(m[2]) === ordered) {
        const width = m[0].length === m[1].length + m[2].length ? m[0].length + 1 : m[0].length
        const body = [lines[i].slice(m[0].length)]
        for (i++; i < lines.length; i++) {
          const l = lines[i]
          if (!l.trim()) {
            const next = lines.slice(i + 1).find((x) => x.trim())
            if (next !== undefined && next.match(/^ */)![0].length >= width) {
              body.push("")
              loose = true
              continue
            }
            break
          }
          const indent = l.match(/^ */)![0].length
          if (indent >= width) body.push(l.slice(width))
          else if (!starts(l) && body[body.length - 1].trim()) body.push(l.trim())
          else break
        }
        items.push(body)
        // A blank line before another item of this list makes the list loose; one before a list of the other kind doesn't.
        const next = ITEM.exec(lines.slice(i).find((x) => x.trim()) || "")
        if (i < lines.length && !lines[i].trim() && next && /\d/.test(next[2]) === ordered) {
          loose = true
          while (i < lines.length && !lines[i].trim()) i++
        }
      }
      out.push({
        type: "list",
        ordered,
        start,
        items: items.map((body) => {
          const task = /^\[([ xX])\]\s+/.exec(body[0])
          if (task) body[0] = body[0].slice(task[0].length)
          return { blocks: blocks(body), tight: !loose, task: task ? (task[1] === " " ? "open" : "done") : undefined }
        }),
      })
    } else if (/^( {4}|\t)/.test(line)) {
      const body: string[] = []
      for (; i < lines.length && (/^( {4}|\t)/.test(lines[i]) || !lines[i].trim()); i++) body.push(lines[i].replace(/^( {4}|\t)/, ""))
      out.push({ type: "code", text: body.join("\n").replace(/\n+$/, ""), lang: "" })
    } else if (HTML.test(line)) {
      const body: string[] = []
      for (; i < lines.length && lines[i].trim(); i++) body.push(lines[i])
      const text = body.join("\n").replace(/<!--[\s\S]*?-->/g, "")
      if (text.replace(/<[^>]*>/g, "").trim()) out.push({ type: "paragraph", html: inline(text.replace(/<\/(p|div|h\d)>/gi, "<br>")).replace(/^(\s*<br>)+|(<br>\s*)+$/g, "") })
    } else {
      const body = [line]
      for (i++; i < lines.length && lines[i].trim() && !starts(lines[i]) && !(lines[i].includes("|") && DIVIDER.test(lines[i + 1] || "")); i++) {
        if (/^ {0,3}(=+|-+)\s*$/.test(lines[i])) break
        body.push(lines[i])
      }
      const setext = /^ {0,3}(=+|-+)\s*$/.exec(lines[i] || "")
      if (setext && lines[i].trim()) {
        const html = inline(body.join("\n"))
        out.push({ type: "heading", level: setext[1][0] === "=" ? 1 : 2, html, id: slug(html) })
        i++
      } else out.push({ type: "paragraph", html: inline(body.join("\n").replace(/^\s+/gm, "")) })
    }
  }
  return out
}

/// `text` as blocks.
export function parseMarkdown(text: string): Block[] {
  return blocks(String(text ?? "").replace(/\r\n?/g, "\n").replace(/\t/g, "    ").split("\n"))
}
