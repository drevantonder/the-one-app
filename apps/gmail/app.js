// Gmail: the person's own Gmail accounts, as a pure app: Google's API and the Vault, no service. Each account's
// refresh token is a secret field of its Google account item, which the node fills in only for Google's token
// address, so no app or agent ever reads it; each call gets a fresh access token with it. Signing in is the same in
// every Google app: see "accounts and tokens" below.

const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/gmail.modify";
const API = "https://gmail.googleapis.com/gmail/v1/users/me/";
const UPLOAD = "https://gmail.googleapis.com/upload/gmail/v1/users/me/";
const APP = "gmail";
const TITLE = "Gmail";
// Where Google sends the browser back: the install's address, which opens this app's settings screen to finish. The
// Google sign-in (a Web application client) lists it as an authorized redirect URI.
function redirect(ctx) {
  if (!ctx.address) throw new Error("This device is out of date: update it, then sign in again");
  return `${ctx.address}/back`;
}
// How much of a body read and thread give; the rest is cut.
const LONGEST = 20000;
const MOST_FILES = 10;
const MOST_BYTES = 25 << 20;

const utf8 = new TextEncoder();

function toB64(bytes) {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode.apply(null, bytes.subarray(i, i + 0x8000));
  return btoa(s);
}
const b64url = (bytes) => toB64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
function fromB64(s) {
  const bin = atob(String(s).replace(/-/g, "+").replace(/_/g, "/").replace(/\s+/g, ""));
  const out = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}
// Base64 in lines of 76, as MIME wants it.
const wrapped = (bytes) => toB64(bytes).replace(/.{76}(?=.)/g, "$&\r\n");

// ---- accounts and tokens ----

// The Vault keeps what signing in needs, the same for every Google app: one "Google sign-in" item, the OAuth client
// (its id, and its secret), and a "Google account" item for each account (its address, its refresh token and the
// scopes it allowed), sent only to Google's token address. Signing in to an account another Google app already uses
// adds this app's scopes to it (incremental consent). Google sends the browser back to the install's address,
// which opens this app's settings screen to finish; the verifier waits for it in this app's own data.

const VAULT = {
  client: {
    title: "Google sign-in", type: "Google sign-in", domain: "google.com", hosts: [OAUTH],
    why: "Make one in Google Cloud Console: APIs & Services › Credentials › Create OAuth client ID, type Web application. It's set once, for everyone's Google apps. Its authorized redirect URI: {address}/back",
    // Set once, by an admin in Everyone's Vault, for every app that declares a shared Google sign-in: anyone else is
    // told which admin to ask.
    shared: true,
    fields: { client_id: { label: "Client ID" }, client_secret: { label: "Client secret", secret: true } },
  },
  account: {
    title: "Google account", type: "Google account", domain: "google.com", hosts: [OAUTH], accounts: true,
    fields: { email: { label: "Email" }, refresh_token: { label: "Refresh token", secret: true } },
  },
};

/// The Google sign-in (OAuth client) this app is switched on for: yours first, else your group's or everyone's (the one
/// an admin set for everyone).
async function googleClient(ctx, id) {
  const all = (await ctx.vault.items()).filter((i) => i.fields.client_id?.value && i.fields.client_secret);
  return all.find((i) => i.fields.client_id.value === id) || all.find((i) => i.scope.startsWith("user:")) || all[0] || null;
}
const noClient = () => new Error(`No Google sign-in is set up yet: vault_request (app ${APP}, need client) asks for it. An admin sets it once, for everyone; for anyone else it says which admin to ask, which is all to tell them`);

/// The Google accounts this app is switched on for, the first signed in first.
async function accountItems(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === "Google account" && i.fields.email?.value && i.fields.refresh_token).sort((a, b) => a.time - b.time);
}

async function accounts(ctx) {
  if (!ctx.user) throw new Error("this device doesn't keep your data, so it can't reach your accounts: use one of your own devices");
  return (await accountItems(ctx)).map((i) => i.fields.email.value);
}

/// The account named, by its address or a part of it only it has; the first signed in when none is named.
async function account(ctx, name) {
  const all = await accounts(ctx);
  if (!all.length) throw new Error(`Sign in to a Google account first: ${APP}_sign_in signs one in`);
  if (!name) return all[0];
  const n = String(name).toLowerCase();
  const found = all.find((e) => e.toLowerCase() === n) || (all.filter((e) => e.toLowerCase().includes(n)).length === 1 && all.find((e) => e.toLowerCase().includes(n)));
  if (!found) throw new Error(`no single account matches "${name}"; they are: ${all.join(", ")}`);
  return found;
}

/// Whether an account allowed this app's scopes, or signed in before accounts kept them.
const allowed = (item) => !item.fields.scopes || SCOPES.split(" ").every((s) => s === "openid" || s === "email" || item.fields.scopes.value.split(" ").includes(s));

const tokens = new Map(); // `${user}/${email}` → { value, expires }, for this runtime only
async function token(ctx, email) {
  const item = (await accountItems(ctx)).find((i) => i.fields.email.value === email);
  if (!item) throw new Error(`Sign in to ${email} first: ${APP}_sign_in signs it in`);
  if (!allowed(item)) throw new Error(`Needs signing in again, to allow ${TITLE}: ${APP}_sign_in with account ${email} signs it in`);
  const key = `${ctx.user.id}/${email}`;
  const t = tokens.get(key);
  if (t && t.expires > Date.now() + 60_000) return t.value;
  const client = await googleClient(ctx, item.fields.client_id?.value);
  if (!client) throw noClient();
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=refresh_token&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&refresh_token={{${item.id}:refresh_token}}`,
  });
  const body = await r.json().catch(() => ({}));
  if (!r.ok || !body.access_token) throw new Error(`Needs signing in again (${email}: ${body.error_description || body.error || r.status}): ${APP}_sign_in signs it in again`);
  tokens.set(key, { value: body.access_token, expires: Date.now() + (body.expires_in || 3600) * 1000 });
  return body.access_token;
}

/// How long a sign-in waits for Google to send the browser back: past it, it's forgotten.
const WAITS = 30 * 60_000;

/// A link to sign in to Google with, `hint` the account to suggest. Each sign-in waits in this app's own data as a row
/// of its own, keyed by a random state Google passes back, so two at once (two accounts, devices or agents) each
/// finish with their own verifier.
async function startSignIn(ctx, hint) {
  const client = await googleClient(ctx);
  if (!client) throw noClient();
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
  const state = b64url(crypto.getRandomValues(new Uint8Array(16)));
  const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
  ctx.user.exec("delete from pending where at < ?", [Date.now() - WAITS]);
  ctx.user.exec("insert into pending values (?, ?, ?, ?)", [state, verifier, client.id ?? null, Date.now()]);
  const q = new URLSearchParams({
    client_id: client.fields.client_id.value, redirect_uri: redirect(ctx), response_type: "code", scope: SCOPES, access_type: "offline",
    // `<app>/<screen>/<state>`: the install's address opens this app's settings screen, which finishes this sign-in.
    prompt: "consent", include_granted_scopes: "true", state: `${APP}/settings/${state}`, code_challenge: challenge, code_challenge_method: "S256", ...(hint ? { login_hint: hint } : {}),
  });
  return `https://accounts.google.com/o/oauth2/v2/auth?${q}`;
}

/// A sign-in that's over without an account: it answers why rather than failing, so its verifier is forgotten, as a
/// failed call writes nothing.
function failed(ctx, state, why) {
  ctx.user.exec("delete from pending where state = ?", [state]);
  return { signed_in: false, why };
}

/// The address an id_token says signed in, from its base64url payload.
function emailOf(idToken) {
  try {
    const payload = String(idToken).split(".")[1].replace(/-/g, "+").replace(/_/g, "/");
    return JSON.parse(new TextDecoder().decode(Uint8Array.from(atob(payload), (c) => c.charCodeAt(0)))).email || null;
  } catch {
    return null;
  }
}

async function finishSignIn(ctx, given) {
  const s = String(given || "").trim();
  const params = s.includes("=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams : null;
  // The sign-in Google's state names; a bare code finishes the one sign-in waiting, if only one is.
  const state = params?.get("state")?.split("/")[2];
  const waiting = await ctx.user.query("select state, verifier, client from pending where at >= ?", [Date.now() - WAITS]);
  const p = state ? waiting.find((w) => w.state === state) : waiting.length === 1 ? waiting[0] : null;
  if (!p) {
    if (!state && waiting.length > 1) throw new Error("More than one sign-in is waiting: give the whole address the browser came back to, not just its code.");
    throw new Error(`Start signing in again: no sign-in is waiting for that address; ${APP}_sign_in starts one`);
  }
  const refused = params?.get("error");
  if (refused) return failed(ctx, p.state, `Google didn't sign you in (${refused}). Try again: ${APP}_sign_in starts again.`);
  const code = params ? params.get("code") : s;
  if (!code) throw new Error("That address has no code. Copy the whole address the browser ended up on.");
  const client = (await ctx.vault.items()).find((i) => i.id === p.client && i.fields.client_secret) || await googleClient(ctx);
  if (!client) throw noClient();
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=authorization_code&code=${encodeURIComponent(code)}&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&redirect_uri=${encodeURIComponent(redirect(ctx))}&code_verifier=${encodeURIComponent(p.verifier)}`,
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok) return failed(ctx, p.state, `Google didn't sign you in (${tok.error_description || tok.error || r.status}). Try again: ${APP}_sign_in starts again.`);
  if (!tok.refresh_token) return failed(ctx, p.state, "Google didn't sign you in. Try again: it gave no refresh_token, so the sign-in wouldn't last.");
  const email = emailOf(tok.id_token);
  if (!email) return failed(ctx, p.state, "Google didn't say which account signed in. Try again.");
  ctx.user.exec("delete from pending where state = ?", [p.state]);
  const id = await ctx.vault.create({
    title: email, type: "Google account", domain: "google.com", hosts: [OAUTH], same: "email",
    fields: { email: { value: email }, refresh_token: { value: tok.refresh_token, secret: true }, scopes: { value: tok.scope || SCOPES }, client_id: { value: client.fields.client_id.value } },
  });
  tokens.set(`${ctx.user.id}/${email}`, { value: tok.access_token, expires: Date.now() + (tok.expires_in || 3600) * 1000 });
  return { signed_in: true, account: email };
}

const signInSchema = { user: [
  "create table pending (verifier text not null, client text);",
  "drop table pending; create table pending (state text primary key, verifier text not null, client text, at integer not null);",
] };

const signInOps = {
  accounts: {
    reads: true,
    description: `The Google accounts ${TITLE} can use, the default first: [{ account, ok }], with why when ok is false (${APP}_sign_in with that account fixes it). Every other operation's account takes one of these.`,
    input: { type: "object", properties: {} },
    async run(ctx) {
      return Promise.all((await accounts(ctx)).map(async (account) => {
        try { await token(ctx, account); return { account, ok: true }; } catch (e) { return { account, ok: false, why: e.message }; }
      }));
    },
  },
  sign_in: {
    reads: false,
    description: `Start signing a Google account in to ${TITLE}, or allowing ${TITLE} on one already signed in to another Google app: gives { url, then }. The person opens url and signs in, and Google sends them back to finish.`,
    input: { type: "object", properties: {
      account: { type: "string", description: "the account's address, like someone@gmail.com, to suggest it to Google; for one that needs signing in again, as accounts lists it" },
    } },
    async run(ctx, { account }) {
      if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
      return {
        url: await startSignIn(ctx, account),
        then: `The person opens url and signs in, and Google sends them back to finish; ${APP}_accounts then lists the account. If it doesn't, give the address their browser came back to to ${APP}_finish_sign_in.`,
      };
    },
  },
  finish_sign_in: {
    reads: false,
    description: `Finish signing in with the address the browser came back to after ${APP}_sign_in's url, when it didn't finish by itself. Gives { signed_in: true, account }, or { signed_in: false, why } when Google didn't sign in.`,
    input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the whole address the browser came back to, like https://…/back?state=…&code=…, or just its code" } } },
    run: (ctx, { code }) => finishSignIn(ctx, code),
  },
  sign_out: {
    reads: false,
    description: `Sign a Google account out of ${TITLE}; it stays signed in to the other Google apps that use it, and ${APP}_sign_in signs it in again.`,
    input: { type: "object", required: ["account"], properties: { account: { type: "string", description: `the account's address, or a part of it only it has, as ${APP}_accounts lists them` } } },
    async run(ctx, { account: name }) {
      const email = await account(ctx, name);
      const item = (await accountItems(ctx)).find((i) => i.fields.email.value === email);
      ctx.vault.release(item.id);
      tokens.delete(`${ctx.user.id}/${email}`);
      return `Signed ${email} out of ${TITLE}.`;
    },
  },
};


/// Calls Gmail's API as `email`: a path under users/me, or a whole URL.
/// `missing` is what to say when what the path names isn't there, as Gmail answers 404 or "Invalid id value".
async function api(ctx, email, method, path, { query, json, raw, type, missing } = {}) {
  const q = query ? new URLSearchParams() : null;
  for (const [k, v] of Object.entries(query || {})) {
    for (const one of Array.isArray(v) ? v : [v]) if (one !== undefined && one !== null && one !== "") q.append(k, String(one));
  }
  const url = (path.startsWith("http") ? path : API + path) + (q && [...q].length ? `?${q}` : "");
  const r = await fetch(url, {
    method,
    headers: { authorization: `Bearer ${await token(ctx, email)}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
    body: json ? JSON.stringify(json) : raw,
  });
  const text = await r.text();
  if (r.status === 401) tokens.delete(`${ctx.user.id}/${email}`);
  if (!r.ok) {
    let message = text.slice(0, 300) || r.statusText;
    try { message = JSON.parse(text).error?.message || message; } catch {}
    if (missing && (r.status === 404 || /invalid id value/i.test(message))) throw new Error(missing);
    throw new Error(`Gmail: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}

// ---- reading ----

const header = (p, name) => (p?.headers || []).find((h) => h.name.toLowerCase() === name.toLowerCase())?.value || "";

/// When Gmail got it, in ISO 8601 to the minute: 2026-10-31T14:23Z.
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

/// HTML made readable as plain text: no tags, a line per block.
function htmlText(h) {
  h = h.replace(/<(head|style|script|title)\b[\s\S]*?<\/\1\s*>/gi, "")
    .replace(/<br\s*\/?>|<\/(p|div|tr|h[1-6]|table|ul|ol|blockquote)\s*>/gi, "\n")
    .replace(/<li\b[^>]*>/gi, "\n- ")
    .replace(/<[^>]*>/g, "");
  h = unescapeHtml(h).replace(FILLER, "").replace(/[ \t\u00a0]+/g, " ");
  return h.split("\n").map((l) => l.trim()).join("\n").replace(/\n\s*\n\s*\n+/g, "\n\n").trim();
}

/// A message's text: its first plain-text part, else its first HTML part made plain. Parts with a filename are
/// attachments, not text.
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

/// A message's attachments, in order: every part with a filename.
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

/// Labels by name, leaving out Gmail's categories.
const labelText = (ids, names) => (ids || []).filter((id) => !id.startsWith("CATEGORY_")).map((id) => names[id] || id).join(", ");

async function labelNames(ctx, email) {
  const { labels = [] } = await api(ctx, email, "GET", "labels");
  return Object.fromEntries(labels.map((l) => [l.id, l.name]));
}

/// A message as text: its headers, labels, attachments, then body.
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

/// Each message's headers, ten at a time, in order.
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

// ---- writing ----

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

/// Splits "a@x.org, Name <b@y.org>, "Last, First" <c@z.org>" at the commas outside quotes and angle brackets.
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

/// One recipient, given as "a@x.org", "Name <a@x.org>" or { name, email }.
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

/// Recipients, as an array (of strings or { name, email }) or one comma-separated string.
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

/// The email `in` asks for, threaded under reply_to when it's a reply: to its sender, with its subject, unless `in`
/// says otherwise.
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

// ---- the app ----

const str = (description) => ({ type: "string", description });
const ids = (description) => ({ type: "array", items: { type: "string" }, description });
const accountInput = str("which account: its address or a part of it only it has, as gmail_accounts lists them; default the first");
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });
const people = (what) => ({
  type: "array", items: { type: "string" },
  description: `${what}: addresses, each "a@example.org" or "Name <a@example.org>"`,
});
const maxResults = (usual, most) => ({ type: "integer", description: `at most this many; default ${usual}, at most ${most}` });
const pageToken = (op) => str(`the next page: the token the last ${op} ended with ("More: give pageToken …"), with the same other inputs`);
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
  instructions: "Gmail is the person's Gmail accounts. gmail_search finds messages with Gmail's search syntax " +
    "(q: from:, is:unread, in:inbox, newer_than:2d, ...); gmail_read reads one and gmail_thread a whole conversation; gmail_attachment gets one of a message's files. " +
    "gmail_label archives (remove INBOX), marks read (remove UNREAD), stars or labels messages. Write with gmail_draft and let the person send it, unless they asked you to send: " +
    "gmail_send and gmail_send_draft send real email to real people. With several accounts, each operation takes account; gmail_accounts lists them.",
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
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
      description: "The account's labels, Gmail's own (INBOX, UNREAD, STARRED, ...) first, then the person's: each its name, and its id where that differs. gmail_label and gmail_search's labelIds take either.",
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
      description: "Move messages to the trash. They stay there 30 days, where the person can take them out again in Gmail.",
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
      description: "Save an email as a draft (text, HTML or both, with files if given) for the person to check and send, or replace a draft. Use it rather than gmail_send unless the person asked you to send. Gives the draft's id.",
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
      description: "Send a draft as it is. This sends real email to real people: only when the person asked to send this one.",
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
      description: "Send an email (text, HTML or both, with files if given), or a reply. This sends real email to real people: only what the person asked to send; otherwise gmail_draft. Gives the sent message's id.",
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
    settings: { title: "Gmail", html: settingsScreen("Gmail", "Sign in to Gmail, and your agents can search, read and draft your email.") },
  },
};

/// The settings screen a shell opens for a connector: the accounts signed in, each with
/// whether it works, a way to allow it again and to sign it out, and signing in another at Google, which sends the
/// browser back here with { back: the address it came back to } to finish.
/// Opened with { add: true } (a shell's Add Account), it starts signing in at once.
function settingsScreen(title, blurb) {
  return `
<div id="list"></div>
<div id="connect"></div>
<script>
const list = document.getElementById("list"), connect = document.getElementById("connect");
let accounts = [], asked = 0, signing = false, blocked = null;
// Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
const leaving = new Set(), failed = {};
const shown = () => accounts.filter((a) => !leaving.has(a.account));
function draw(fetch) {
  return one.render(list, async () => {
    if (fetch) {
      const n = ++asked, got = await one.call("accounts");
      if (n === asked) accounts = got;
    }
    const all = shown();
    // Not set up for anyone yet, and not this person's to set up: what to do, and no button that can't work.
    const stuck = blocked ? one.problem({ title: blocked.title, text: blocked.text, warn: true }) : "";
    if (!all.length) return stuck || (signing ? "" : one.empty("${blurb}", { label: "Sign In to Google", run: () => start() }));
    // What needs signing in again is a problem box, as Backups shows one, and a warn line on its row.
    const needs = blocked ? "" : all.filter((a) => !a.ok).map((a) => one.problem({
      title: a.account + " needs signing in",
      text: one.plain(a.why || "Needs signing in again").title.replace(/^Needs signing in again\\.$/, "Sign in to Google again to let ${title} in."),
      warn: true, action: { label: "Sign In", run: () => start(a.account) } })).join("");
    return stuck + needs + '<h2 class="section">Accounts</h2><div class="list">' + all.map((a, i) => {
      const line = failed[a.account] || (a.ok ? (i ? "" : "Used first") : "Sign in again");
      const sub = line ? \`<div class="sub\${a.ok && !failed[a.account] ? "" : " warn"}">\${a.ok && !failed[a.account] ? "" : one.icon("warning-circle-fill")}\${one.esc(line)}</div>\` : "";
      return \`<div class="row plain"><div class="main"><div class="title">\${one.esc(a.account)}</div>\${sub}</div><button class="button plain" data-out="\${one.esc(a.account)}">Sign Out</button></div>\`;
    }).join("") + (blocked ? "" : '<button class="row plain add" id="add"><div class="main"><div class="title">Add Account</div></div></button>') + "</div>";
  }, { skeleton: 1 });
}
list.onclick = async (e) => {
  const add = e.target.closest("#add");
  if (add) return start(undefined, add);
  const b = e.target.closest("[data-out]");
  const account = b?.dataset.out;
  if (!account) return;
  leaving.add(account);
  delete failed[account];
  draw(false);
  try {
    await one.call("sign_out", { account });
    accounts = accounts.filter((a) => a.account !== account);
  } catch (err) {
    failed[account] = one.plain(err).title;
  }
  leaving.delete(account);
  draw(!failed[account]);
};
// A failure for a title: its first sentence, without "Try again." (what the button says).
const said = (e) => one.plain(e).title.replace(/(\\. Try again)?\\.$/, "");
const stop = () => { signing = false; connect.innerHTML = ""; };
async function start(account, b) {
  const back = b && one.busy(b);
  let link;
  try {
    link = (await one.call("sign_in", account ? { account } : {})).url;
  } catch (e) {
    // No Google sign-in yet: an admin sets one up for everyone, then it goes on; anyone else is told whom to ask, and
    // can't sign in until then.
    if (/No Google sign-in/.test(e.message) && one.vault) {
      try { if (await one.vault("client")) return start(account, b); } catch (err) { e = err; }
    }
    back?.();
    const ask = /Ask (.+?) to set up (.+?)(?::|$)/.exec(e.message);
    if (ask) {
      blocked = { title: ask[2] + " isn't set up", text: "Ask " + ask[1] + " to set it up. It's done once for everyone." };
      return draw(false);
    }
    connect.innerHTML = one.problem({ title: said(e), action: { label: "Try Again", run: () => start(account) } });
    return;
  }
  signing = true;
  draw(false);
  connect.innerHTML = \`<h2 class="section">Sign In</h2><div class="list">
      <a class="row" href="\${one.esc(link)}"><div class="main"><div class="title">Sign in to Google</div><div class="sub">and allow ${title}</div></div><span class="chev">›</span></a></div>
    <div class="footer">Google brings you back here when you're done.</div>\`;
}
// Back from Google, at the address it sent the browser to.
async function finish(back) {
  signing = true;
  draw(false);
  connect.innerHTML = one.loading("Signing In");
  try {
    const r = await one.call("finish_sign_in", { code: back });
    if (r?.signed_in === false) throw new Error(r.why);
  } catch (e) {
    // Finished already: this screen opened again at the same address.
    if (/no sign-in is waiting/.test(e.message)) return stop(), draw(true);
    connect.innerHTML = one.problem({ title: said(e), action: { label: "Try Again", run: () => start() } });
    return;
  }
  stop();
  await draw(true);
}
// Signed in where Google sent the browser back (another tab, or the app): done here too.
one.on("changed", async () => {
  const was = JSON.stringify(shown());
  await draw(true);
  if (signing && connect.querySelector("a.row") && JSON.stringify(shown()) !== was) stop(), draw(false);
});
draw(true);
if (one.params.back) finish(one.params.back);
else if (one.params.add) start();
</script>`;
}
