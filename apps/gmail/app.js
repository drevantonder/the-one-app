// Gmail: the user's own Gmail accounts, as a pure app: Google's API and the Vault, no service. Each account's
// refresh token is a secret field of its Google account item, which the node fills in only for Google's token
// address, so no app or agent ever reads it; each call gets a fresh access token with it. Signing in is the same in
// every Google app: see google/google.js.

import settings from "./dist/settings.html" with { type: "text" };
import { google } from "./google/google.js";

const SCOPES = "openid email https://www.googleapis.com/auth/gmail.modify";
const API = "https://gmail.googleapis.com/gmail/v1/users/me/";
const UPLOAD = "https://gmail.googleapis.com/upload/gmail/v1/users/me/";
const APP = "gmail";
const TITLE = "Gmail";
const { account, api, str, withAccount, vault, schema, tools: signIn } = google({ app: APP, title: TITLE, scopes: SCOPES, base: API });
const LONGEST = 20000;
const MOST_FILES = 10;
const MOST_BYTES = 25 << 20;

const utf8 = new TextEncoder();

function toB64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
function fromB64(s) {
  const bin = atob(String(s).replace(/-/g, "+").replace(/_/g, "/").replace(/\s+/g, ""));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
// Base64 in lines of 76, as MIME wants it.
const wrapped = (bytes) => toB64(bytes).replace(/.{76}(?=.)/g, "$&\r\n");

const header = (p, name) => (p?.headers || []).find((h) => h.name.toLowerCase() === name.toLowerCase())?.value || "";

const when = (m) => `${new Date(Number(m.internalDate)).toISOString().slice(0, 16)}Z`;

function decode(data, charset) {
  const bytes = fromB64(data || "");
  try { return new TextDecoder(charset || "utf-8").decode(bytes); } catch { return new TextDecoder().decode(bytes); }
}
const charsetOf = (p) => /charset="?([^";\s]+)/i.exec(header(p, "Content-Type"))?.[1];

const FILLER = /[\u200b-\u200d\u2060\ufeff\u034f\u00ad]/g; // invisible padding marketing emails use
const ENTITIES = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: "\u00a0" };
const unescapeHtml = (s) => s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (all, e) =>
  e[0] === "#" ? String.fromCodePoint(e[1].toLowerCase() === "x" ? parseInt(e.slice(2), 16) : Number(e.slice(1))) : ENTITIES[e.toLowerCase()] ?? all);

function htmlText(h) {
  h = h.replace(/<(head|style|script|title)\b[\s\S]*?<\/\1\s*>/gi, "")
    .replace(/<br\s*\/?>|<\/(p|div|tr|h[1-6]|table|ul|ol|blockquote)\s*>/gi, "\n")
    .replace(/<li\b[^>]*>/gi, "\n- ")
    .replace(/<[^>]*>/g, "");
  h = unescapeHtml(h).replace(FILLER, "").replace(/[ \t\u00a0]+/g, " ");
  return h.split("\n").map((l) => l.trim()).join("\n").replace(/\n\s*\n\s*\n+/g, "\n\n").trim();
}

function body(p) {
  let plain = "", html = "";
  const walk = (p) => {
    if (p.filename) return;
    if (p.mimeType?.startsWith("multipart/")) for (const c of p.parts || []) walk(c);
    else if (p.mimeType === "text/plain" && !plain) plain = decode(p.body?.data, charsetOf(p));
    else if (p.mimeType === "text/html" && !html) html = decode(p.body?.data, charsetOf(p));
  };
  walk(p);
  if (plain.trim()) return plain.replace(/\r\n/g, "\n").replace(FILLER, "").replace(/[ \t\u00a0]+\n/g, "\n").trim();
  return htmlText(html);
}

function attachments(p, out = []) {
  if (p.filename) out.push({ name: p.filename, type: p.mimeType, size: p.body?.size || 0, id: p.body?.attachmentId, data: p.body?.data });
  for (const c of p.parts || []) attachments(c, out);
  return out;
}

function size(n) {
  if (n >= 1 << 20) return `${(n / (1 << 20)).toFixed(1)} MB`;
  if (n >= 1 << 10) return `${n >> 10} KB`;
  return `${n} bytes`;
}

const labelText = (ids, names) => (ids || []).filter((id) => !id.startsWith("CATEGORY_")).map((id) => names[id] || id).join(", ");

async function labelNames(ctx, email) {
  const { labels = [] } = await api(ctx, email, "GET", "labels");
  return Object.fromEntries(labels.map((l) => [l.id, l.name]));
}

function show(m, names) {
  const lines = [];
  for (const h of ["Subject", "From", "To", "Cc", "Reply-To"]) {
    const v = header(m.payload, h);
    if (v) lines.push(`${h}: ${v}`);
  }
  lines.push(`Date: ${when(m)}`, `Id: ${m.id} (thread ${m.threadId})`);
  const l = labelText(m.labelIds, names);
  if (l) lines.push(`Labels: ${l}`);
  attachments(m.payload).forEach((a, i) => lines.push(`Attachment ${i + 1}: ${a.name} (${a.type}, ${size(a.size)})`));
  let text = body(m.payload);
  if (text.length > LONGEST) text = text.slice(0, LONGEST) + "\n\n[cut: the message goes on]";
  return lines.join("\n") + "\n\n" + text;
}

const snippet = (s) => unescapeHtml(s || "").replace(FILLER, "").split(/\s+/).filter(Boolean).join(" ");

async function metadata(ctx, email, ms) {
  const out = new Array(ms.length);
  for (let i = 0; i < ms.length; i += 10) {
    await Promise.all(ms.slice(i, i + 10).map(async (m, j) => {
      out[i + j] = await api(ctx, email, "GET", `messages/${encodeURIComponent(m.id)}`,
        { query: { format: "metadata", metadataHeaders: ["From", "To", "Subject", "Date"] } });
    }));
  }
  return out;
}

const ASCII = /^[\x20-\x7e]*$/;

/// An RFC 2047 encoded word (or several, folded) for text that isn't plain ASCII, or would look like a placeholder.
function word(s) {
  if (ASCII.test(s) && !s.includes("{{") && !/=\?/.test(s)) return s;
  const words = [];
  let chunk = "";
  for (const ch of s) {
    if (utf8.encode(chunk + ch).length > 45) { words.push(chunk); chunk = ""; }
    chunk += ch;
  }
  if (chunk) words.push(chunk);
  return words.map((w) => `=?UTF-8?B?${toB64(utf8.encode(w))}?=`).join("\r\n ");
}

function splitAddresses(s) {
  const out = [];
  let cur = "", quoted = false, angle = false;
  for (const ch of s) {
    if (ch === '"') quoted = !quoted;
    else if (ch === "<" && !quoted) angle = true;
    else if (ch === ">" && !quoted) angle = false;
    if ((ch === "," || ch === ";") && !quoted && !angle) { out.push(cur); cur = ""; } else cur += ch;
  }
  out.push(cur);
  return out.map((a) => a.trim()).filter(Boolean);
}

function address(a) {
  let name = "", email = "";
  if (typeof a === "object" && a) {
    name = a.name || "";
    email = a.email || a.address || "";
  } else {
    const m = /^(.*?)\s*<([^<>]+)>$/.exec(String(a).trim());
    if (m) [name, email] = [m[1].trim().replace(/^"(.*)"$/, "$1").replace(/\\(.)/g, "$1"), m[2].trim()];
    else email = String(a).trim();
  }
  email = email.replace(/[\r\n]/g, "");
  if (!/^[^\s@<>,]+@[^\s@<>,]+$/.test(email)) throw new Error(`"${email || a}" isn't an email address`);
  name = name.replace(/[\r\n]+/g, " ").trim();
  if (!name) return email;
  if (!ASCII.test(name) || name.includes("{{")) return `${word(name)} <${email}>`;
  if (/[()<>\[\]:;@\\,."]/.test(name)) return `"${name.replace(/(["\\])/g, "\\$1")}" <${email}>`;
  return `${name} <${email}>`;
}

const recipients = (v) => (v == null || v === "" ? [] : (Array.isArray(v) ? v : [v]).flatMap((a) => (typeof a === "string" ? splitAddresses(a) : [a])).map(address));

const TYPES = {
  pdf: "application/pdf", png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", heic: "image/heic",
  txt: "text/plain", csv: "text/csv", html: "text/html", htm: "text/html", md: "text/markdown", json: "application/json", zip: "application/zip",
  doc: "application/msword", docx: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  xls: "application/vnd.ms-excel", xlsx: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
  ppt: "application/vnd.ms-powerpoint", pptx: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
  ics: "text/calendar", mp3: "audio/mpeg", m4a: "audio/mp4", mp4: "video/mp4", mov: "video/quicktime",
};

/// The files to attach, each { name, type?, base64 | data (base64) | blob or file (a blob's hash) }.
async function files(ctx, given) {
  const list = given || [];
  if (list.length > MOST_FILES) throw new Error(`at most ${MOST_FILES} attachments`);
  let total = 0;
  const out = [];
  for (const f of list) {
    if (!f?.name) throw new Error("each attachment needs a name");
    let bytes;
    if (f.blob || f.file) bytes = await ctx.blob.get(f.blob || f.file);
    else if ((f.base64 ?? f.data) != null) {
      try { bytes = fromB64(f.base64 ?? f.data); } catch { throw new Error(`attachment ${f.name} isn't base64`); }
    } else throw new Error(`attachment ${f.name} needs base64 or blob`);
    total += bytes.length;
    if (total > MOST_BYTES) throw new Error("attachments may be 25 MB in all");
    out.push({ name: String(f.name).replace(/[\r\n"]/g, ""), type: f.type || TYPES[f.name.split(".").pop().toLowerCase()] || "application/octet-stream", bytes });
  }
  return out;
}

let boundaries = 0;
const boundary = (kind) => `one-${kind}-${Date.now().toString(36)}-${(boundaries++).toString(36)}-${Math.random().toString(36).slice(2, 10)}`;

/// A part of text in UTF-8, base64 so that no line is too long and nothing in it looks like a placeholder.
const textPart = (type, text) => `Content-Type: ${type}; charset=UTF-8\r\nContent-Transfer-Encoding: base64\r\n\r\n${wrapped(utf8.encode(text.replace(/\r?\n/g, "\r\n")))}\r\n`;

/// The email as RFC 5322 text, all ASCII: text, or text and HTML as multipart/alternative, and files after them in
/// multipart/mixed. Gmail adds From, Date and Message-ID.
function rfc822(d) {
  const head = [];
  const fold = (list) => list.join(",\r\n ");
  if (d.to.length) head.push(`To: ${fold(d.to)}`);
  if (d.cc.length) head.push(`Cc: ${fold(d.cc)}`);
  if (d.bcc.length) head.push(`Bcc: ${fold(d.bcc)}`);
  head.push(`Subject: ${word(d.subject.replace(/[\r\n]+/g, " "))}`);
  if (d.inReplyTo) head.push(`In-Reply-To: ${d.inReplyTo}`);
  if (d.references) head.push(`References: ${d.references}`);
  head.push("MIME-Version: 1.0");
  let content;
  if (d.html) {
    const b = boundary("alt");
    content = `Content-Type: multipart/alternative; boundary="${b}"\r\n\r\n--${b}\r\n${textPart("text/plain", d.text ?? htmlText(d.html))}--${b}\r\n${textPart("text/html", d.html)}--${b}--\r\n`;
  } else content = textPart("text/plain", d.text || "");
  if (d.files.length) {
    const b = boundary("mixed");
    let out = `Content-Type: multipart/mixed; boundary="${b}"\r\n\r\n--${b}\r\n${content}`;
    for (const f of d.files) {
      const ascii = ASCII.test(f.name);
      const name = ascii ? f.name : word(f.name);
      const star = ascii ? "" : `; filename*=UTF-8''${encodeURIComponent(f.name).replace(/['()*]/g, (c) => `%${c.charCodeAt(0).toString(16).toUpperCase()}`)}`;
      out += `--${b}\r\nContent-Type: ${f.type}; name="${name}"\r\nContent-Disposition: attachment; filename="${name}"${star}\r\nContent-Transfer-Encoding: base64\r\n\r\n${wrapped(f.bytes)}\r\n`;
    }
    content = out + `--${b}--\r\n`;
  }
  return head.join("\r\n") + "\r\n" + content;
}

async function compose(ctx, email, input) {
  const d = {
    to: recipients(input.to), cc: recipients(input.cc), bcc: recipients(input.bcc),
    subject: input.subject || "", text: input.body ?? input.text, html: input.html || "", files: await files(ctx, input.attachments), thread: "",
  };
  if (input.reply_to) {
    const m = await api(ctx, email, "GET", `messages/${encodeURIComponent(String(input.reply_to))}`,
      { query: { format: "metadata", metadataHeaders: ["From", "Reply-To", "Subject", "Message-ID", "References"] }, missing: noMessage(input.reply_to) });
    d.thread = m.threadId;
    const id = header(m.payload, "Message-ID");
    d.inReplyTo = id;
    d.references = `${header(m.payload, "References")} ${id}`.trim();
    if (!d.to.length) d.to = recipients(header(m.payload, "Reply-To") || header(m.payload, "From"));
    if (!d.subject) d.subject = /^re:/i.test(header(m.payload, "Subject")) ? header(m.payload, "Subject") : `Re: ${header(m.payload, "Subject")}`;
  }
  return d;
}

/// Sends or saves the message through Gmail's upload endpoint, which takes messages up to 35 MB, with the thread
/// (and draft) it belongs to as metadata.
async function upload(ctx, email, method, path, d, meta, missing) {
  const b = boundary("upload");
  const raw = `--${b}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n${JSON.stringify(meta)}\r\n--${b}\r\nContent-Type: message/rfc822\r\n\r\n${rfc822(d)}\r\n--${b}--\r\n`;
  return api(ctx, email, method, UPLOAD + path, { query: { uploadType: "multipart" }, raw, type: `multipart/related; boundary=${b}`, missing });
}

const ids = (description) => ({ type: "array", items: { type: "string" }, description });
const people = (what) => ({
  type: "array", items: { type: "string" },
  description: `${what}: addresses, each "a@example.org" or "Name <a@example.org>"`,
});
const maxResults = (usual, most) => ({ type: "integer", description: `at most this many; default ${usual}, at most ${most}` });
const pageToken = (tool) => str(`the next page: the token the last ${tool} ended with ("More: give pageToken …"), with the same other inputs`);
const writing = {
  to: people("who it's to; a reply goes to the sender unless given"), cc: people("copied to"), bcc: people("copied to, unseen"),
  subject: str("the subject; a reply keeps the original's (with Re:) unless given"),
  body: str("the email's text, as plain text"),
  html: str("the email as HTML; sent with body (or, without it, the HTML made plain) as its text, so mail apps show the one they prefer"),
  reply_to: str("a message's id, from gmail_search or gmail_read, to reply to it: the reply goes in its thread"),
  attachments: {
    type: "array", description: `files to attach, up to ${MOST_FILES} and 25 MB in all`,
    items: { type: "object", format: "file", required: ["name"], properties: { name: str("the file's name, like receipt.pdf"), type: str("its MIME type, like application/pdf; guessed from the name if left out"), base64: str("its bytes, in base64"), blob: str("or a kept file's hash, as file put prints it") } },
  },
};
const list = (v) => (v == null ? [] : (Array.isArray(v) ? v : [v]).flatMap((s) => String(s).split(",")).map((s) => s.trim()).filter(Boolean));
const need = (v, what) => { if (!String(v ?? "").trim()) throw new Error(`give ${what}`); return String(v).trim(); };
const noMessage = (id) => `no message ${id}: message ids come from gmail_search`;
const noDraft = (id) => `no draft ${id}: draft ids come from gmail_drafts`;
const more = (token) => (token ? `\n\nMore: give pageToken ${token}` : "");

export default {
  name: "gmail",
  title: "Gmail",
  tagline: "Your email",
  icon: "envelope",
  color: "#ea4335",
  description: "Your Gmail accounts, for you and your agents: search, read, label, draft and send",
  instructions: "Gmail is the user's Gmail accounts. gmail_search finds messages with Gmail's search syntax " +
    "(q: from:, is:unread, in:inbox, newer_than:2d, ...); gmail_read reads one and gmail_thread a whole conversation; gmail_attachment gets one of a message's files. " +
    "gmail_label archives (remove INBOX), marks read (remove UNREAD), stars or labels messages. Write with gmail_draft and let the user send it, unless they asked you to send: " +
    "gmail_send and gmail_send_draft send real email to real people. With several accounts, each tool takes account; gmail_accounts lists them.",
  vault,
  schema,

  tools: {
    ...signIn,
    profile: {
      reads: true,
      description: "The account's address and how many messages and threads it has.",
      input: withAccount({}),
      async run(ctx, { account: a }) {
        const p = await api(ctx, await account(ctx, a), "GET", "profile");
        return `${p.emailAddress}: ${p.messagesTotal} messages in ${p.threadsTotal} threads.`;
      },
    },
    search: {
      reads: true,
      description: "Find messages (Gmail's messages.list), newest first, as text: each message's id, when it came (ISO 8601, UTC), sender, subject, label names and a snippet; with no q, all mail but spam and trash. Read one with gmail_read, its conversation with gmail_thread. A long list ends with More: give pageToken <token>.",
      input: withAccount({
        q: str("a search as in Gmail's search box, like in:inbox is:unread from:someone@example.com newer_than:7d"),
        labelIds: ids("only messages with all these labels: label ids, as gmail_labels lists them, like INBOX or Label_12"),
        maxResults: maxResults(20, 100), pageToken: pageToken("search"),
      }),
      async run(ctx, { account: a, q, labelIds, maxResults, pageToken }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "GET", "messages", { query: { maxResults: Math.min(Number(maxResults) || 20, 100), q, labelIds: list(labelIds), pageToken } });
        if (!r.messages?.length) return pageToken ? "No more." : "Nothing found.";
        const [names, ms] = await Promise.all([labelNames(ctx, email), metadata(ctx, email, r.messages)]);
        const out = ms.map((m) => {
          let s = `${m.id}  ${when(m)}  ${header(m.payload, "From")}\n  ${header(m.payload, "Subject")}`;
          const l = labelText(m.labelIds, names);
          if (l) s += `\n  [${l}]`;
          if (m.snippet) s += `\n  ${snippet(m.snippet)}`;
          return s;
        });
        return out.join("\n") + more(r.nextPageToken);
      },
    },
    read: {
      reads: true,
      description: "One message in full: its headers, when it came, labels, attachments (numbered, for gmail_attachment) and its text. For the replies around it, gmail_thread gives the whole conversation.",
      input: withAccount({ id: str("the message's id, from gmail_search") }, ["id"]),
      async run(ctx, { account: a, id }) {
        const email = await account(ctx, a);
        id = need(id, "id: a message's id, from gmail_search");
        const [m, names] = await Promise.all([api(ctx, email, "GET", `messages/${encodeURIComponent(id)}`, { query: { format: "full" }, missing: noMessage(id) }), labelNames(ctx, email)]);
        return show(m, names);
      },
    },
    thread: {
      reads: true,
      description: "A whole conversation, oldest first, each message as gmail_read gives it. Use it to see a message with its replies; gmail_read gives just the one.",
      input: withAccount({ id: str("the thread's id, or the id of any message in it, from gmail_search") }, ["id"]),
      async run(ctx, { account: a, id }) {
        const email = await account(ctx, a);
        id = need(id, "id: a message's id, from gmail_search");
        let t;
        try {
          t = await api(ctx, email, "GET", `threads/${encodeURIComponent(id)}`, { query: { format: "full" }, missing: noMessage(id) });
        } catch (err) { // or a message's id, whose thread this is
          const m = await api(ctx, email, "GET", `messages/${encodeURIComponent(id)}`, { query: { format: "minimal" } }).catch(() => null);
          if (!m || m.threadId === id) throw err;
          t = await api(ctx, email, "GET", `threads/${encodeURIComponent(m.threadId)}`, { query: { format: "full" } });
        }
        const names = await labelNames(ctx, email);
        return t.messages.map((m) => show(m, names)).join("\n\n----------\n\n");
      },
    },
    attachment: {
      reads: true,
      description: "One of a message's attachments, as a file: { name, type, size, base64 }. MCP gives it back as a file id other tools take; the CLI saves it with --out.",
      input: withAccount({
        id: str("the message's id, from gmail_search"), name: str("the attachment's file name, or a part of it, as gmail_read lists them; or give number"),
        number: { type: "integer", description: "which attachment, as gmail_read numbers them (1 is the first); the first when neither is given" },
      }, ["id"]),
      async run(ctx, { account: a, id, name, number }) {
        const email = await account(ctx, a);
        id = need(id, "id: a message's id, from gmail_search");
        const m = await api(ctx, email, "GET", `messages/${encodeURIComponent(id)}`, { query: { format: "full" }, missing: noMessage(id) });
        const all = attachments(m.payload);
        if (!all.length) throw new Error(`message ${id} has no attachments`);
        const f = name ? all.find((x) => x.name === name) || all.find((x) => x.name.toLowerCase().includes(String(name).toLowerCase())) : all[(Number(number) || 1) - 1];
        if (!f) throw new Error(`no attachment ${name ? `"${name}"` : number} on message ${id}; it has: ${all.map((x, i) => `${i + 1}. ${x.name}`).join(", ")}`);
        const data = f.id ? (await api(ctx, email, "GET", `messages/${encodeURIComponent(m.id)}/attachments/${encodeURIComponent(f.id)}`)).data : f.data;
        const bytes = fromB64(data || "");
        return { name: f.name, type: f.type, size: bytes.length, base64: toB64(bytes) };
      },
    },
    labels: {
      reads: true,
      description: "The account's labels, Gmail's own (INBOX, UNREAD, STARRED, ...) first, then the user's: each its name, and its id where that differs. gmail_label and gmail_search's labelIds take either.",
      input: withAccount({}),
      async run(ctx, { account: a }) {
        const { labels = [] } = await api(ctx, await account(ctx, a), "GET", "labels");
        labels.sort((x, y) => x.type.localeCompare(y.type) || x.name.toLowerCase().localeCompare(y.name.toLowerCase()));
        return labels.map((l) => (l.id !== l.name ? `${l.name}  (id ${l.id})` : l.name)).join("\n");
      },
    },
    label: {
      reads: false,
      description: "Add labels to and remove labels from messages (Gmail's messages.batchModify). Archive: remove INBOX. Mark read: remove UNREAD. Star: add STARRED. Says what changed.",
      input: withAccount({
        ids: ids("the messages' ids, from gmail_search"),
        addLabelIds: ids("labels to add: ids or names, as gmail_labels lists them, like STARRED or Receipts"),
        removeLabelIds: ids("labels to remove: ids or names, like INBOX or UNREAD"),
      }, ["ids"]),
      async run(ctx, { account: a, ids: given, addLabelIds: add, removeLabelIds: remove }) {
        const email = await account(ctx, a);
        const which = list(given);
        if (!which.length) throw new Error("give ids: messages' ids, from gmail_search");
        if (list(add).length + list(remove).length === 0) throw new Error("give addLabelIds or removeLabelIds: label ids or names, as gmail_labels lists them");
        const { labels = [] } = await api(ctx, email, "GET", "labels");
        const resolve = (names) => list(names).map((g) => {
          const l = labels.find((l) => l.id === g || l.name.toLowerCase() === g.toLowerCase());
          if (!l) throw new Error(`no label "${g}": gmail_labels lists them`);
          return l;
        });
        const [adding, removing] = [resolve(add), resolve(remove)];
        await api(ctx, email, "POST", "messages/batchModify", {
          json: { ids: which, addLabelIds: adding.map((l) => l.id), removeLabelIds: removing.map((l) => l.id) },
          missing: `no message among ${which.join(", ")}: message ids come from gmail_search`,
        });
        const did = [adding.length && `added ${adding.map((l) => l.name).join(", ")}`, removing.length && `removed ${removing.map((l) => l.name).join(", ")}`].filter(Boolean).join(" and ");
        return `${did[0].toUpperCase()}${did.slice(1)} on ${which.length === 1 ? "1 message" : `${which.length} messages`}.`;
      },
    },
    trash: {
      reads: false,
      description: "Move messages to the trash. They stay there 30 days, where the user can take them out again in Gmail.",
      input: withAccount({ ids: ids("the messages' ids, from gmail_search") }, ["ids"]),
      async run(ctx, { account: a, ids: given }) {
        const email = await account(ctx, a);
        const which = list(given);
        if (!which.length) throw new Error("give ids: messages' ids, from gmail_search");
        for (const [i, one] of which.entries()) {
          try { await api(ctx, email, "POST", `messages/${encodeURIComponent(one)}/trash`, { missing: noMessage(one) }); } catch (e) { throw new Error(i ? `trashed ${i} of ${which.length}, then ${e.message}` : e.message); }
        }
        return `Moved ${which.length === 1 ? "1 message" : `${which.length} messages`} to the trash.`;
      },
    },
    drafts: {
      reads: true,
      description: "The account's drafts (Gmail's drafts.list), newest first, as text: each draft's id, when it was saved (ISO 8601, UTC), who it's to and its subject. A long list ends with More: give pageToken <token>.",
      input: withAccount({ maxResults: maxResults(20, 100), pageToken: pageToken("drafts") }),
      async run(ctx, { account: a, maxResults, pageToken }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "GET", "drafts", { query: { maxResults: Math.min(Number(maxResults) || 20, 100), pageToken } });
        if (!r.drafts?.length) return pageToken ? "No more." : "No drafts.";
        const ms = await metadata(ctx, email, r.drafts.map((d) => d.message));
        const out = ms.map((m, i) => `${r.drafts[i].id}  ${when(m)}  to ${header(m.payload, "To") || "nobody yet"}\n  ${header(m.payload, "Subject") || "(no subject)"}`);
        return out.join("\n") + more(r.nextPageToken);
      },
    },
    draft: {
      reads: false,
      description: "Save an email as a draft (text, HTML or both, with files if given) for the user to check and send, or replace a draft. Use it rather than gmail_send unless the user asked you to send. Gives the draft's id.",
      input: withAccount({ ...writing, id: str("a draft's id, from gmail_drafts, to replace that draft") }),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const d = await compose(ctx, email, input);
        const message = d.thread ? { threadId: d.thread } : {};
        if (input.id) {
          const id = String(input.id);
          const saved = await upload(ctx, email, "PUT", `drafts/${encodeURIComponent(id)}`, d, { id, message }, noDraft(id));
          return `Replaced draft ${saved.id} (message ${saved.message?.id}).`;
        }
        const saved = await upload(ctx, email, "POST", "drafts", d, { message });
        return `Saved draft ${saved.id} (message ${saved.message?.id}).`;
      },
    },
    send_draft: {
      reads: false,
      confirm: true,
      description: "Send a draft as it is. This sends real email to real people: only when the user asked to send this one.",
      input: withAccount({ id: str("the draft's id, from gmail_drafts or gmail_draft") }, ["id"]),
      async run(ctx, { account: a, id }) {
        id = need(id, "id: a draft's id, from gmail_drafts");
        const sent = await api(ctx, await account(ctx, a), "POST", "drafts/send", { json: { id }, missing: noDraft(id) });
        return `Sent, as message ${sent.id}.`;
      },
    },
    delete_draft: {
      reads: false,
      confirm: true,
      description: "Delete a draft for good: it doesn't go to the trash.",
      input: withAccount({ id: str("the draft's id, from gmail_drafts") }, ["id"]),
      async run(ctx, { account: a, id }) {
        id = need(id, "id: a draft's id, from gmail_drafts");
        await api(ctx, await account(ctx, a), "DELETE", `drafts/${encodeURIComponent(id)}`, { missing: noDraft(id) });
        return `Deleted draft ${id}.`;
      },
    },
    send: {
      reads: false,
      confirm: true,
      description: "Send an email (text, HTML or both, with files if given), or a reply. This sends real email to real people: only what the user asked to send; otherwise gmail_draft. Gives the sent message's id.",
      input: withAccount(writing),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const d = await compose(ctx, email, input);
        if (!d.to.length && !d.cc.length && !d.bcc.length) throw new Error("give to: who it's to, or reply_to a message");
        const sent = await upload(ctx, email, "POST", "messages/send", d, d.thread ? { threadId: d.thread } : {});
        return `Sent, as message ${sent.id}.`;
      },
    },
  },

  screens: {
    settings: { title: "Gmail", html: settings },
  },
};
