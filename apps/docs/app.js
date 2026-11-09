// Docs: the user's own Google Docs, through Google's API. Signing in is the same in every
// Google app: see google/google.js.

import settings from "./dist/settings.html" with { type: "text" };
import { google } from "./google/google.js";

const APP = "docs";
const TITLE = "Google Docs";
const SCOPES = "openid email https://www.googleapis.com/auth/documents";
const API = "https://docs.googleapis.com/v1/";
const { account, api, str, withAccount, vault, schema, tools: signIn } = google({ app: APP, title: TITLE, scopes: SCOPES, base: API });

const docId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give documentId: the document's id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const docInput = str("the document's id, or its address, like https://docs.google.com/document/d/1AbC…/edit; drive_files_list finds documents (q mimeType = 'application/vnd.google-apps.document' and name contains '…')");
const noDoc = (v) => `no document ${docId(v)}: give its id or its address (docs.google.com/document/d/…); drive_files_list finds documents`;

const ORDERED = /DECIMAL|ALPHA|ROMAN|ZERO/;

function runs(p, md) {
  let out = "";
  for (const el of p.elements || []) {
    const t = el.textRun;
    if (!t) {
      if (el.person) out += el.person.personProperties?.name || el.person.personProperties?.email || "";
      else if (el.richLink) out += md ? `[${el.richLink.richLinkProperties?.title}](${el.richLink.richLinkProperties?.uri})` : el.richLink.richLinkProperties?.uri || "";
      continue;
    }
    let s = t.content.replace(/\n$/, "").replace(/\u000b/g, "\n");
    if (md && s.trim()) {
      const st = t.textStyle || {};
      const [lead, core, trail] = [/^\s*/.exec(s)[0], s.trim(), /\s*$/.exec(s)[0]];
      let c = core;
      if (st.weightedFontFamily?.fontFamily && /mono|courier|consol/i.test(st.weightedFontFamily.fontFamily)) c = `\`${c}\``;
      else {
        if (st.bold) c = `**${c}**`;
        if (st.italic) c = `*${c}*`;
        if (st.strikethrough) c = `~~${c}~~`;
      }
      if (st.link?.url) c = `[${c}](${st.link.url})`;
      s = lead + c + trail;
    }
    out += s;
  }
  return out;
}

function render(doc, md) {
  const lines = [];
  const counters = {};
  const walk = (content, depth = 0) => {
    for (const el of content || []) {
      if (el.paragraph) {
        const p = el.paragraph;
        const text = runs(p, md);
        const style = p.paragraphStyle?.namedStyleType || "";
        if (p.bullet) {
          const level = p.bullet.nestingLevel || 0;
          const props = doc.lists?.[p.bullet.listId]?.listProperties?.nestingLevels?.[level] || {};
          const key = `${p.bullet.listId}/${level}`;
          counters[key] = (counters[key] || 0) + 1;
          const mark = ORDERED.test(props.glyphType || "") ? `${counters[key]}.` : "-";
          lines.push(`${"  ".repeat(level)}${mark} ${text}`);
          continue;
        }
        for (const k of Object.keys(counters)) delete counters[k];
        const h = /^HEADING_(\d)$/.exec(style)?.[1] || (style === "TITLE" ? "1" : style === "SUBTITLE" ? "2" : "");
        lines.push(md && h && text.trim() ? `${"#".repeat(Number(h))} ${text}` : text);
      } else if (el.table) {
        const rows = el.table.tableRows.map((r) => r.tableCells.map((c) => {
          const cell = [];
          for (const x of c.content || []) if (x.paragraph) cell.push(runs(x.paragraph, md));
          return cell.join(md ? "<br>" : " ").replace(/\|/g, md ? "\\|" : "|").trim();
        }));
        if (md && rows.length) {
          lines.push("", `| ${rows[0].join(" | ")} |`, `| ${rows[0].map(() => "---").join(" | ")} |`, ...rows.slice(1).map((r) => `| ${r.join(" | ")} |`), "");
        } else for (const r of rows) lines.push(r.join("\t"));
      } else if (el.tableOfContents) {
        if (depth === 0) walk(el.tableOfContents.content, depth + 1);
      }
    }
  };
  walk(doc.body?.content);
  return lines.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

export default {
  name: APP,
  title: "Google Docs",
  tagline: "Your Google Docs",
  icon: "file-text",
  color: "#4285f4",
  description: "Your Google Docs: read them as markdown or text, make them, and add or replace text",
  instructions: `Docs is the user's Google Docs. ${APP}_read reads a document as markdown (or text). ${APP}_create makes one, ${APP}_append adds text at its end, and ${APP}_replace replaces every match of some text: change only what the user asked. Documents are given as documentId, an id or address. With several accounts, each tool takes account.`,
  vault,
  schema,

  tools: {
    ...signIn,
    read: {
      title: "Read a document",
      reads: true,
      description: "A document (Docs' documents.get) as text: its title, address and body, as markdown (headings, lists, tables, bold, italic, links) or plain text.",
      input: withAccount({ documentId: docInput, format: { type: "string", enum: ["markdown", "text"], description: "markdown (the default) or text" } }, ["documentId"]),
      output: { type: "string", description: "the title, address and body of the document, as markdown or text" },
      async run(ctx, { account: a, documentId: document, format }) {
        const d = await api(ctx, await account(ctx, a), "GET", `documents/${encodeURIComponent(docId(document))}`, { missing: noDoc(document) });
        const text = render(d, format !== "text");
        return `${d.title}\nhttps://docs.google.com/document/d/${d.documentId}/edit\n\n${text}`;
      },
    },
    create: {
      title: "Make a document",
      reads: false,
      description: "Make a document in the user's Drive (in My Drive; Docs' documents.create), with text if given. Gives { id, title, url }.",
      input: withAccount({ title: str("its title, like Meeting notes 2026-11-03"), text: str("its first text, as plain text: markdown stays as typed") }, ["title"]),
      output: {
        type: "object",
        required: ["id", "title", "url"],
        properties: { id: { type: "string" }, title: { type: "string" }, url: { type: "string" } },
      },
      async run(ctx, { account: a, title, text }) {
        const email = await account(ctx, a);
        const d = await api(ctx, email, "POST", "documents", { json: { title } });
        if (text) await api(ctx, email, "POST", `documents/${d.documentId}:batchUpdate`, { json: { requests: [{ insertText: { text, location: { index: 1 } } }] } });
        return { id: d.documentId, title: d.title, url: `https://docs.google.com/document/d/${d.documentId}/edit` };
      },
    },
    append: {
      title: "Add text to the end of a document",
      reads: false,
      description: "Add text at the end of a document, as a new paragraph (a batchUpdate insertText). This changes the user's document: do it only when they asked.",
      input: withAccount({ documentId: docInput, text: str("the text, as plain text: markdown stays as typed") }, ["documentId", "text"]),
      output: { type: "string", description: "that it was added, with the document's address" },
      async run(ctx, { account: a, documentId: document, text }) {
        const id = encodeURIComponent(docId(document));
        await api(ctx, await account(ctx, a), "POST", `documents/${id}:batchUpdate`, { json: { requests: [{ insertText: { text: `\n${text}`, endOfSegmentLocation: {} } }] }, missing: noDoc(document) });
        return `Added it at the end of https://docs.google.com/document/d/${docId(document)}/edit`;
      },
    },
    replace: {
      title: "Replace text in a document",
      reads: false,
      confirm: true,
      description: "Replace every match of some text in a document (a batchUpdate replaceAllText); says how many changed. Do it only when the user asked.",
      input: withAccount({
        documentId: docInput, find: str("the text to find, exactly as docs_read gives it as text"), replace: str("what to put instead, as plain text (empty removes it)"),
        matchCase: { type: "boolean", description: "match capitals exactly (default false)" },
      }, ["documentId", "find"]),
      output: { type: "string", description: "how many matches changed, in words" },
      async run(ctx, { account: a, documentId: document, find, replace = "", matchCase }) {
        const id = encodeURIComponent(docId(document));
        const r = await api(ctx, await account(ctx, a), "POST", `documents/${id}:batchUpdate`,
          { json: { requests: [{ replaceAllText: { containsText: { text: find, matchCase: !!matchCase }, replaceText: replace } }] }, missing: noDoc(document) });
        const n = r.replies?.[0]?.replaceAllText?.occurrencesChanged || 0;
        return n ? `Replaced ${n === 1 ? "1 match" : `${n} matches`}.` : `Found no "${find}": nothing changed. docs_read with format text shows what's there.`;
      },
    },
  },

  screens: {
    settings: { title: TITLE, html: settings },
  },
};
