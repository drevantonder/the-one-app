// Docs: the person's own Google Docs, through Google's API. Signing in is the same in every
// Google app: see "signing in" below.

const APP = "docs";
const TITLE = "Google Docs";
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
const noClient = () => new Error(`${TITLE} has no Google sign-in yet: choose one on its settings page, or ask the person for it with vault_request (app ${APP}, need client)`);

/// The Google accounts this app is switched on for, the first signed in first.
async function accountItems(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === "Google account" && i.fields.email?.value && i.fields.refresh_token).sort((a, b) => a.time - b.time);
}

async function accounts(ctx) {
  if (!ctx.user) throw new Error("this server doesn't keep your data, so it can't reach your accounts: use one of your own servers");
  return (await accountItems(ctx)).map((i) => i.fields.email.value);
}

/// The account named, by its address or a part of it only it has; the first signed in when none is named.
async function account(ctx, name) {
  const all = await accounts(ctx);
  if (!all.length) throw new Error(`no Google account is signed in: ${APP}_sign_in signs one in`);
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
  if (!item) throw new Error(`${email} isn't signed in to ${TITLE}: ${APP}_sign_in signs it in`);
  if (!allowed(item)) throw new Error(`${email} hasn't allowed ${TITLE} yet: sign in to it again (${APP}_sign_in with account)`);
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
  if (!r.ok || !body.access_token) throw new Error(`${email}'s sign-in no longer works (${body.error_description || body.error || r.status}): ${APP}_sign_in signs it in again`);
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
  if (!p) throw new Error(`no sign-in is waiting for that address: ${APP}_sign_in starts one`);
  const s = String(given || "").trim();
  const refused = s.match(/[?&]error=([^&#]+)/)?.[1];
  if (refused) return failed(ctx, `Google didn't sign you in (${decodeURIComponent(refused)}). Start again with ${APP}_sign_in.`);
  const code = s.includes("code=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams.get("code") : s;
  if (!code) throw new Error("that address has no code: copy the whole address the browser ended up on");
  const client = (await ctx.vault.items()).find((i) => i.id === p.client && i.fields.client_secret) || await googleClient(ctx);
  if (!client) throw noClient();
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=authorization_code&code=${encodeURIComponent(code)}&client_id=${encodeURIComponent(client.fields.client_id.value)}&client_secret={{${client.id}:client_secret}}&redirect_uri=${encodeURIComponent(redirect(ctx))}&code_verifier=${encodeURIComponent(p.verifier)}`,
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok) return failed(ctx, `Sign-in failed: ${tok.error_description || tok.error || r.status}. Start again with ${APP}_sign_in.`);
  if (!tok.refresh_token) return failed(ctx, "Google didn't give a refresh token, so the sign-in wouldn't last; start again.");
  const email = emailOf(tok.id_token);
  if (!email) return failed(ctx, "Google didn't say which account signed in; start again.");
  ctx.user.exec("delete from pending");
  const id = await ctx.vault.create({
    title: email, type: "Google account", domain: "google.com", hosts: [OAUTH], same: "email",
    fields: { email: { value: email }, refresh_token: { value: tok.refresh_token, secret: true }, scopes: { value: tok.scope || SCOPES }, client_id: { value: client.fields.client_id.value } },
  });
  tokens.set(`${ctx.user.id}/${email}`, { value: tok.access_token, expires: Date.now() + (tok.expires_in || 3600) * 1000 });
  return `Signed in as ${email}.`;
}

const signInSchema = { user: "create table pending (verifier text not null, client text);" };

const signInOps = {
  accounts: {
    reads: true,
    description: "The Google accounts signed in here, the default first: each { account, ok }, with why when ok is false and it needs signing in again.",
    input: { type: "object", properties: {} },
    async run(ctx) {
      return Promise.all((await accounts(ctx)).map(async (account) => {
        try { await token(ctx, account); return { account, ok: true }; } catch (e) { return { account, ok: false, why: e.message }; }
      }));
    },
  },
  sign_in: {
    description: "Sign in to a Google account, or another one, or with account, allow this app on one: returns a link to open in a browser; then pass the address the browser ends up on as code (here or to finish_sign_in).",
    input: { type: "object", properties: {
      code: { type: "string", description: "the address the browser was sent to after signing in, to finish here instead" },
      account: { type: "string", description: "the account's address, to suggest it to Google" },
    } },
    async run(ctx, { code, account }) {
      if (!ctx.user) throw new Error("this server doesn't keep your data: use one of your own servers");
      if (code) return finishSignIn(ctx, code);
      return `Open this link in a browser on any device and sign in. The browser then fails to load a 127.0.0.1 page: copy that page's whole address and pass it to ${APP}_finish_sign_in.\n\n${await startSignIn(ctx, account)}`;
    },
  },
  finish_sign_in: {
    description: "Finish signing in: the address the browser ended up on after sign_in's link (or just its code). Answers { signed_in: false, why } when Google didn't sign in.",
    input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the address, or its code" } } },
    run: (ctx, { code }) => finishSignIn(ctx, code),
  },
  sign_out: {
    description: "Sign a Google account out of this app; it stays signed in to the other Google apps that use it.",
    input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the account's address, or a part of it only it has" } } },
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
const accountInput = str(`which account: its address or a part of it only it has; default the first, as ${APP}_accounts lists them`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });

/// Calls Google's API as `email`.
async function api(ctx, email, method, path, { query, json } = {}) {
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
    throw new Error(`${TITLE}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}


// ---- documents ----

/// A document's id, given it or its address.
const docId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give document: its id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const docInput = str("the document's id, or its address (docs.google.com/document/d/…)");

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
  title: TITLE,
  icon: "file-text",
  color: "#4285f4",
  description: "Your Google Docs: read them as markdown or text, make them, and add or replace text.",
  instructions: `Docs is the person's Google Docs. ${APP}_read reads a document as markdown (or text). ${APP}_create makes one, ${APP}_append adds text at its end, and ${APP}_replace replaces every match of some text: change only what the person asked. Documents are given by id or address. With several accounts, each operation takes account.`,
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    read: {
      reads: true,
      description: "A document's title, address and text, as markdown (headings, lists, tables, bold, italic, links) or plain text.",
      input: withAccount({ document: docInput, format: str("markdown (the default) or text") }, ["document"]),
      async run(ctx, { account: a, document, format }) {
        const d = await api(ctx, await account(ctx, a), "GET", `documents/${encodeURIComponent(docId(document))}`);
        const text = render(d, format !== "text");
        return `${d.title}\nhttps://docs.google.com/document/d/${d.documentId}/edit\n\n${text}`;
      },
    },
    create: {
      description: "Make a document in the person's Drive, with text if given. Answers its id and address.",
      input: withAccount({ title: str("its title"), text: str("its text, as plain text") }, ["title"]),
      async run(ctx, { account: a, title, text }) {
        const email = await account(ctx, a);
        const d = await api(ctx, email, "POST", "documents", { json: { title } });
        if (text) await api(ctx, email, "POST", `documents/${d.documentId}:batchUpdate`, { json: { requests: [{ insertText: { text, location: { index: 1 } } }] } });
        return { id: d.documentId, title: d.title, url: `https://docs.google.com/document/d/${d.documentId}/edit` };
      },
    },
    append: {
      description: "Add text at the end of a document, as a new paragraph. This changes the person's document: do it only when they asked.",
      input: withAccount({ document: docInput, text: str("the text, as plain text") }, ["document", "text"]),
      async run(ctx, { account: a, document, text }) {
        const id = encodeURIComponent(docId(document));
        await api(ctx, await account(ctx, a), "POST", `documents/${id}:batchUpdate`, { json: { requests: [{ insertText: { text: `\n${text}`, endOfSegmentLocation: {} } }] } });
        return "Added it at the end.";
      },
    },
    replace: {
      confirm: true,
      description: "Replace every match of some text in a document. Do it only when the person asked.",
      input: withAccount({
        document: docInput, find: str("the text to find"), replace: str("what to put instead (empty removes it)"),
        match_case: { type: "boolean", description: "match capitals exactly (default false)" },
      }, ["document", "find"]),
      async run(ctx, { account: a, document, find, replace = "", match_case }) {
        const id = encodeURIComponent(docId(document));
        const r = await api(ctx, await account(ctx, a), "POST", `documents/${id}:batchUpdate`,
          { json: { requests: [{ replaceAllText: { containsText: { text: find, matchCase: !!match_case }, replaceText: replace } }] } });
        const n = r.replies?.[0]?.replaceAllText?.occurrencesChanged || 0;
        return n ? `Replaced ${n} matches.` : "Found no match: nothing changed.";
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
  .card { background: var(--card); border-radius: 12px; padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--accent); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--accent); background: none; border: 0; padding: 4px 0; margin-left: 12px; }
  input { width: 100%; box-sizing: border-box; font: inherit; padding: 11px 14px; border-radius: 10px; border: 0.5px solid var(--line); background: var(--bg); color: var(--text); margin: 8px 0; }
  ol { padding-left: 20px; margin: 8px 0; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: #34c759; flex: none; margin-right: 12px; }
  .dot.warn { background: #ff9f0a; }
</style>
<div id="list"></div>
<div class="card" id="connect"></div>
<script>
async function show() {
  const accounts = await one.call("accounts");
  document.getElementById("list").innerHTML = accounts.length
    ? '<h2 class="section">Accounts</h2><div class="list">' + accounts.map((a, i) => \`<div class="row plain"><span class="dot\${a.ok ? "" : " warn"}"></span><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub">\${one.esc(a.ok ? (i ? "Signed in" : "Signed in · used first") : a.why || "Needs signing in again")}</div></div>\${a.ok ? "" : \`<button class="plain" data-in="\${one.esc(a.account)}">Sign in</button>\`}<button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`).join("") + "</div>"
    : '<p class="muted">${blurb}</p>';
  for (const b of document.querySelectorAll("[data-out]")) b.onclick = async () => { await one.call("sign_out", { account: b.dataset.out }); show(); };
  for (const b of document.querySelectorAll("[data-in]")) b.onclick = () => start(b.dataset.in);
  const c = document.getElementById("connect");
  c.innerHTML = '<button class="primary" id="start">' + (accounts.length ? "Add Account" : "Sign In to Google") + '</button><div id="err" class="error small"></div>';
  document.getElementById("start").onclick = () => start();
}
async function start(account) {
  const c = document.getElementById("connect");
  let link;
  try {
    const r = await one.call("sign_in", account ? { account } : {});
    link = typeof r === "string" ? r.split("\\n").pop() : r.url;
  } catch (e) {
    // No Google sign-in yet: the person picks one from their Vault, or makes one, then it goes on.
    if (/no Google sign-in/.test(e.message) && one.vault) {
      try { if (await one.vault("client")) return start(account); } catch (err) { e = err; }
    }
    document.getElementById("err").textContent = e.message;
    return;
  }
  c.innerHTML = \`<ol><li><a href="\${one.esc(link)}">Sign in to Google</a> and allow ${title}.</li><li>The browser then goes to a page that doesn't load. Copy its whole address and paste it here.</li></ol>
    <input id="code" placeholder="http://127.0.0.1:…/hooks/…/oauth?code=…" autocapitalize="none" autocorrect="off"><button class="primary" id="finish">Sign In</button><div id="err" class="error small"></div>\`;
  document.getElementById("finish").onclick = async () => {
    try { const r = await one.call("finish_sign_in", { code: document.getElementById("code").value }); if (r?.signed_in === false) throw new Error(r.why); show(); }
    catch (e) { document.getElementById("err").textContent = e.message; }
  };
}
one.on("changed", show);
show().then(() => { if (one.params.add) start(); });
</script>`;
}
