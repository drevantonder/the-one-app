// Docs: the person's own Google Docs, through Google's API. Signing in is the same in every
// Google app: see "signing in" below.

const APP = "docs";
const TITLE = "Google Docs"; // also export default's title, a plain string there for the App Store to read
const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/documents";
const API = "https://docs.googleapis.com/v1/";
// Where Google sends the browser back: the node on the person's device, at the port this install's nodes answer on.
const redirect = (ctx) => `http://127.0.0.1:${ctx.port}/hooks/${APP}/oauth`;

// ---- signing in ----

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

// The Vault keeps what signing in needs, the same for every Google app: one "Google sign-in" item, the OAuth client
// (its id, and its secret), and a "Google account" item for each account (its address, its refresh token and the
// scopes it allowed), sent only to Google's token address. Signing in to an account another Google app already uses
// adds this app's scopes to it (incremental consent). The browser ends on a 127.0.0.1 address the person pastes
// back; the verifier waits for it in this app's own data.

const VAULT = {
  client: {
    title: "Google sign-in", type: "Google sign-in", domain: "google.com", hosts: [OAUTH],
    fields: { client_id: { label: "Client ID" }, client_secret: { label: "Client secret", secret: true } },
  },
  account: {
    title: "Google account", type: "Google account", domain: "google.com", hosts: [OAUTH], accounts: true,
    fields: { email: { label: "Email" }, refresh_token: { label: "Refresh token", secret: true } },
  },
};

/// The Google sign-in (OAuth client) this app is switched on for: yours first, else your group's or everyone's.
async function googleClient(ctx, id) {
  const all = (await ctx.vault.items()).filter((i) => i.fields.client_id?.value && i.fields.client_secret);
  return all.find((i) => i.fields.client_id.value === id) || all.find((i) => i.scope.startsWith("user:")) || all[0] || null;
}
const noClient = () => new Error(`Choose a Google sign-in from your Vault first: ${TITLE} has none yet; it's chosen on its settings page, or ask the person for it with vault_request (app ${APP}, need client)`);

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

/// A link to sign in to Google with, `hint` the account to suggest; the verifier waits in this app's own data.
async function startSignIn(ctx, hint) {
  const client = await googleClient(ctx);
  if (!client) throw noClient();
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
  const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
  ctx.user.exec("delete from pending");
  ctx.user.exec("insert into pending values (?, ?)", [verifier, client.id ?? null]);
  const q = new URLSearchParams({
    client_id: client.fields.client_id.value, redirect_uri: redirect(ctx), response_type: "code", scope: SCOPES, access_type: "offline",
    prompt: "consent", include_granted_scopes: "true", code_challenge: challenge, code_challenge_method: "S256", ...(hint ? { login_hint: hint } : {}),
  });
  return `https://accounts.google.com/o/oauth2/v2/auth?${q}`;
}

/// A sign-in that's over without an account: it answers why rather than failing, so its verifier is forgotten, as a
/// failed call writes nothing.
function failed(ctx, why) {
  ctx.user.exec("delete from pending");
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
  const [p] = await ctx.user.query("select verifier, client from pending");
  if (!p) throw new Error(`Start signing in again: no sign-in is waiting for that address; ${APP}_sign_in starts one`);
  const s = String(given || "").trim();
  const refused = s.match(/[?&]error=([^&#]+)/)?.[1];
  if (refused) return failed(ctx, `Google didn't sign you in (${decodeURIComponent(refused)}). Try again: ${APP}_sign_in starts again.`);
  const code = s.includes("code=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams.get("code") : s;
  if (!code) throw new Error("That address has no code. Copy the whole address the browser ended up on.");
  const client = (await ctx.vault.items()).find((i) => i.id === p.client && i.fields.client_secret) || await googleClient(ctx);
  if (!client) throw noClient();
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=authorization_code&code=${encodeURIComponent(code)}&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&redirect_uri=${encodeURIComponent(redirect(ctx))}&code_verifier=${encodeURIComponent(p.verifier)}`,
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok) return failed(ctx, `Google didn't sign you in (${tok.error_description || tok.error || r.status}). Try again: ${APP}_sign_in starts again.`);
  if (!tok.refresh_token) return failed(ctx, "Google didn't sign you in. Try again: it gave no refresh_token, so the sign-in wouldn't last.");
  const email = emailOf(tok.id_token);
  if (!email) return failed(ctx, "Google didn't say which account signed in. Try again.");
  ctx.user.exec("delete from pending");
  const id = await ctx.vault.create({
    title: email, type: "Google account", domain: "google.com", hosts: [OAUTH], same: "email",
    fields: { email: { value: email }, refresh_token: { value: tok.refresh_token, secret: true }, scopes: { value: tok.scope || SCOPES }, client_id: { value: client.fields.client_id.value } },
  });
  tokens.set(`${ctx.user.id}/${email}`, { value: tok.access_token, expires: Date.now() + (tok.expires_in || 3600) * 1000 });
  return { signed_in: true, account: email };
}

const signInSchema = { user: "create table pending (verifier text not null, client text);" };

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
    description: `Start signing a Google account in to ${TITLE}, or allowing ${TITLE} on one already signed in to another Google app: gives { url, then }. The person opens url and signs in; then give the address their browser ends on to ${APP}_finish_sign_in.`,
    input: { type: "object", properties: {
      account: { type: "string", description: "the account's address, like someone@gmail.com, to suggest it to Google; for one that needs signing in again, as accounts lists it" },
    } },
    async run(ctx, { account }) {
      if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
      return {
        url: await startSignIn(ctx, account),
        then: `The person opens url and signs in. Their browser then fails to load a 127.0.0.1 page: give that page's whole address to ${APP}_finish_sign_in.`,
      };
    },
  },
  finish_sign_in: {
    reads: false,
    description: `Finish signing in, with the address the browser ended on after ${APP}_sign_in's url. Gives { signed_in: true, account }, or { signed_in: false, why } when Google didn't sign in.`,
    input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the whole address the browser ended on, like http://127.0.0.1:4747/hooks/…/oauth?code=…, or just its code" } } },
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


const str = (description) => ({ type: "string", description });
const accountInput = str(`which account: its address or a part of it only it has, as ${APP}_accounts lists them; default the first`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });

/// Calls Google's API as `email`. `missing` is what to say when what the path names isn't there, or the account can't
/// open it; `hint` what to add to what Google says about an input it can't take.
async function api(ctx, email, method, path, { query, json, missing, hint } = {}) {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(query || {})) if (v !== undefined && v !== null && v !== "") q.append(k, String(v));
  const r = await fetch(API + path + ([...q].length ? `?${q}` : ""), {
    method,
    headers: { authorization: `Bearer ${await token(ctx, email)}`, ...(json ? { "content-type": "application/json" } : {}) },
    body: json ? JSON.stringify(json) : undefined,
  });
  const text = await r.text();
  if (r.status === 401) tokens.delete(`${ctx.user.id}/${email}`);
  if (!r.ok) {
    let message = text.slice(0, 300) || r.statusText;
    try { message = JSON.parse(text).error?.message || message; } catch {}
    if (missing && r.status === 404) throw new Error(missing);
    if (missing && r.status === 403) throw new Error(`${email} can't open it (${message.replace(/\.$/, "")}): use the account it's shared with (${APP}_accounts lists them), or ask its owner to share it`);
    if (hint && r.status === 400) throw new Error(`${TITLE}: ${message.replace(/\.$/, "")}: ${hint}`);
    throw new Error(`${TITLE}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}


// ---- documents ----

/// A document's id, given it or its address.
const docId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give documentId: the document's id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const docInput = str("the document's id, or its address, like https://docs.google.com/document/d/1AbC…/edit; drive_files_list finds documents (q mimeType = 'application/vnd.google-apps.document' and name contains '…')");
const noDoc = (v) => `no document ${docId(v)}: give its id or its address (docs.google.com/document/d/…); drive_files_list finds documents`;

const ORDERED = /DECIMAL|ALPHA|ROMAN|ZERO/;

/// A paragraph's runs as text, with markdown for bold, italic, code and links when `md`.
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

/// A document's body as markdown (headings, lists, tables, emphasis, links) or plain text.
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
  icon: "file-text",
  color: "#4285f4",
  description: "Your Google Docs: read them as markdown or text, make them, and add or replace text",
  instructions: `Docs is the person's Google Docs. ${APP}_read reads a document as markdown (or text). ${APP}_create makes one, ${APP}_append adds text at its end, and ${APP}_replace replaces every match of some text: change only what the person asked. Documents are given as documentId, an id or address. With several accounts, each operation takes account.`,
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    read: {
      reads: true,
      description: "A document (Docs' documents.get) as text: its title, address and body, as markdown (headings, lists, tables, bold, italic, links) or plain text.",
      input: withAccount({ documentId: docInput, format: { type: "string", enum: ["markdown", "text"], description: "markdown (the default) or text" } }, ["documentId"]),
      async run(ctx, { account: a, documentId: document, format }) {
        const d = await api(ctx, await account(ctx, a), "GET", `documents/${encodeURIComponent(docId(document))}`, { missing: noDoc(document) });
        const text = render(d, format !== "text");
        return `${d.title}\nhttps://docs.google.com/document/d/${d.documentId}/edit\n\n${text}`;
      },
    },
    create: {
      reads: false,
      description: "Make a document in the person's Drive (in My Drive; Docs' documents.create), with text if given. Gives { id, title, url }.",
      input: withAccount({ title: str("its title, like Meeting notes 2026-11-03"), text: str("its first text, as plain text: markdown stays as typed") }, ["title"]),
      async run(ctx, { account: a, title, text }) {
        const email = await account(ctx, a);
        const d = await api(ctx, email, "POST", "documents", { json: { title } });
        if (text) await api(ctx, email, "POST", `documents/${d.documentId}:batchUpdate`, { json: { requests: [{ insertText: { text, location: { index: 1 } } }] } });
        return { id: d.documentId, title: d.title, url: `https://docs.google.com/document/d/${d.documentId}/edit` };
      },
    },
    append: {
      reads: false,
      description: "Add text at the end of a document, as a new paragraph (a batchUpdate insertText). This changes the person's document: do it only when they asked.",
      input: withAccount({ documentId: docInput, text: str("the text, as plain text: markdown stays as typed") }, ["documentId", "text"]),
      async run(ctx, { account: a, documentId: document, text }) {
        const id = encodeURIComponent(docId(document));
        await api(ctx, await account(ctx, a), "POST", `documents/${id}:batchUpdate`, { json: { requests: [{ insertText: { text: `\n${text}`, endOfSegmentLocation: {} } }] }, missing: noDoc(document) });
        return `Added it at the end of https://docs.google.com/document/d/${docId(document)}/edit`;
      },
    },
    replace: {
      reads: false,
      confirm: true,
      description: "Replace every match of some text in a document (a batchUpdate replaceAllText); says how many changed. Do it only when the person asked.",
      input: withAccount({
        documentId: docInput, find: str("the text to find, exactly as docs_read gives it as text"), replace: str("what to put instead, as plain text (empty removes it)"),
        matchCase: { type: "boolean", description: "match capitals exactly (default false)" },
      }, ["documentId", "find"]),
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
    settings: { title: TITLE, html: settingsScreen(TITLE, "Sign in to Google, and your agents can read your documents and, when you ask, write in them.") },
  },
};

/// The settings screen a shell opens for a connector: the accounts signed in, each with
/// whether it works, a way to allow it again and to sign it out, and signing in another, by link and pasted address.
/// Opened with { add: true } (a shell's Add Account), it starts signing in at once.
function settingsScreen(title, blurb) {
  return `
<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--tint); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--tint); background: none; border: 0; padding: 4px 0; margin-left: 12px; }
  button:disabled { opacity: 0.6; }
  button .spinner { display: inline-block; vertical-align: -2px; margin-right: 6px; }
  input { width: 100%; box-sizing: border-box; font: inherit; padding: 11px 14px; border-radius: 10px; border: 0.5px solid var(--line); background: var(--bg); color: var(--text); margin: 8px 0; }
  ol { padding-left: 20px; margin: 8px 0; }
  .sub.warn { color: var(--warn); }
</style>
<div id="list"></div>
<div class="card" id="connect"><button class="primary" id="start">Sign In to Google</button><div id="err" class="error small"></div></div>
<script>
const list = document.getElementById("list"), connect = document.getElementById("connect");
let accounts = [], asked = 0;
// Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
const leaving = new Set(), failed = {};
const shown = () => accounts.filter((a) => !leaving.has(a.account));
const label = () => (shown().length ? "Add Account" : "Sign In to Google");
function busy(b, on, text = b?.textContent) {
  if (!b) return;
  b.disabled = on;
  if (on) b.innerHTML = '<span class="spinner small" aria-hidden="true"></span>' + one.esc(text);
  else b.textContent = text;
}
function draw(fetch) {
  return one.render(list, async () => {
    if (fetch) {
      const n = ++asked, got = await one.call("accounts");
      if (n === asked) accounts = got;
    }
    const s = document.getElementById("start");
    if (s && !s.disabled) s.textContent = label();
    const all = shown();
    return all.length
      ? '<h2 class="section">Accounts</h2><div class="list">' + all.map((a, i) => {
          const why = failed[a.account] || (a.ok ? (i ? "" : "Used first") : one.plain(a.why || "Needs signing in again").title);
          return \`<div class="row plain"><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub\${a.ok && !failed[a.account] ? "" : " warn"}">\${one.esc(why)}</div></div>\${a.ok ? "" : \`<button class="plain" data-in="\${one.esc(a.account)}">Sign in</button>\`}<button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`;
        }).join("") + "</div>"
      : '<p class="muted">${blurb}</p>';
  }, { skeleton: 1 });
}
list.onclick = async (e) => {
  const b = e.target.closest("button");
  if (b?.dataset.in) return start(b.dataset.in, b);
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
function ready() {
  connect.innerHTML = '<button class="primary" id="start">' + label() + '</button><div id="err" class="error small"></div>';
  document.getElementById("start").onclick = (e) => start(undefined, e.currentTarget);
}
async function start(account, b) {
  busy(b, true);
  let link;
  try {
    link = (await one.call("sign_in", account ? { account } : {})).url;
  } catch (e) {
    // No Google sign-in yet: the person picks one from their Vault, or makes one, then it goes on.
    if (/Choose a Google sign-in/.test(e.message) && one.vault) {
      try { if (await one.vault("client")) return start(account, b); } catch (err) { e = err; }
    }
    busy(b, false, b?.id === "start" ? label() : b?.textContent);
    document.getElementById("err").textContent = one.plain(e).title;
    return;
  }
  connect.innerHTML = \`<ol><li><a href="\${one.esc(link)}">Sign in to Google</a> and allow ${title}.</li><li>The browser then goes to a page that doesn't load. Copy its whole address and paste it here.</li></ol>
    <input id="code" placeholder="http://127.0.0.1:…/hooks/…/oauth?code=…" autocapitalize="none" autocorrect="off"><button class="primary" id="finish">Sign In</button><div id="err" class="error small"></div>\`;
  const finish = document.getElementById("finish");
  finish.onclick = async () => {
    busy(finish, true);
    try {
      const r = await one.call("finish_sign_in", { code: document.getElementById("code").value });
      if (r?.signed_in === false) throw new Error(r.why);
    } catch (e) {
      busy(finish, false);
      document.getElementById("err").textContent = one.plain(e).title;
      return;
    }
    await draw(true);
    ready();
  };
}
document.getElementById("start").onclick = (e) => start(undefined, e.currentTarget);
one.on("changed", () => draw(true));
draw(true);
if (one.params.add) start(undefined, document.getElementById("start"));
</script>`;
}
