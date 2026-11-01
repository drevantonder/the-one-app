// Sheets: the person's own Google Sheets, through Google's API. Signing in is apps/gmail's (and apps/drive's) way: the
// OAuth client's id is a setting, its secret and each account's refresh token are the app's secrets, sent only to
// Google's token address, and the browser ends on a 127.0.0.1 address the person pastes back.

const APP = "sheets";
const TITLE = "Google Sheets";
const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/spreadsheets";
const API = "https://sheets.googleapis.com/v4/";
const REDIRECT = `http://127.0.0.1:4747/hooks/${APP}/oauth`;

// ---- signing in, as in apps/gmail ----

const slug = (email) => email.toLowerCase().replace(/[^a-z0-9]+/g, "_").slice(0, 56);
const utf8 = new TextEncoder();
const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const fromB64 = (s) => Uint8Array.from(atob(String(s).replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));

async function setting(ctx, key) {
  for (const s of [ctx.user, ctx.global]) {
    const [row] = s ? await s.query("select value from settings where key = ?", [key]) : [];
    if (row) return JSON.parse(row.value);
  }
  return null;
}

async function accounts(ctx) {
  if (!ctx.user) throw new Error("this server doesn't keep your data, so it can't reach your accounts: use one of your own servers");
  return (await ctx.user.query("select email from accounts order by added, email")).map((a) => a.email);
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

const tokens = new Map(); // `${user}/${email}` → { value, expires }, for this runtime only
async function token(ctx, email) {
  const key = `${ctx.user.id}/${email}`;
  const t = tokens.get(key);
  if (t && t.expires > Date.now() + 60_000) return t.value;
  const client = await setting(ctx, "client_id");
  if (!client) throw new Error(`no OAuth client yet: ${APP}_set_client sets one`);
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=refresh_token&client_id=${encodeURIComponent(client)}&client_secret={{client_secret}}&refresh_token={{refresh_${slug(email)}}}`,
  });
  const body = await r.json().catch(() => ({}));
  if (!r.ok || !body.access_token) throw new Error(`${email}'s sign-in no longer works (${body.error_description || body.error || r.status}): ${APP}_sign_in signs it in again`);
  tokens.set(key, { value: body.access_token, expires: Date.now() + (body.expires_in || 3600) * 1000 });
  return body.access_token;
}

async function startSignIn(ctx) {
  const client = await setting(ctx, "client_id");
  if (!client) throw new Error(`no OAuth client yet: ${APP}_set_client sets one`);
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
  const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", utf8.encode(verifier))));
  ctx.user.secret("verifier", verifier, [OAUTH]); // filled in only when the code is exchanged at Google
  const q = new URLSearchParams({ client_id: client, redirect_uri: REDIRECT, response_type: "code", scope: SCOPES, access_type: "offline", prompt: "consent", code_challenge: challenge, code_challenge_method: "S256" });
  return `https://accounts.google.com/o/oauth2/v2/auth?${q}`;
}

/// A sign-in that's over without an account (Google takes each code once): it answers why rather than failing, so
/// the verifier's deletion is written, as a failed call writes nothing.
function failed(ctx, why) {
  ctx.user.secret("verifier", null);
  return { signed_in: false, why };
}

async function finishSignIn(ctx, given) {
  const client = await setting(ctx, "client_id");
  const s = String(given || "").trim();
  const code = s.includes("code=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams.get("code") : s;
  const refused = String(given).match(/[?&]error=([^&#]+)/)?.[1];
  if (refused) return failed(ctx, `Google didn't sign you in (${decodeURIComponent(refused)}). Start again with ${APP}_sign_in.`);
  if (!code) throw new Error("that address has no code: copy the whole address the browser ended up on");
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=authorization_code&code=${encodeURIComponent(code)}&client_id=${encodeURIComponent(client)}&client_secret={{client_secret}}&redirect_uri=${encodeURIComponent(REDIRECT)}&code_verifier={{verifier}}`,
  });
  const tok = await r.json().catch(() => ({}));
  if (!r.ok) return failed(ctx, `Sign-in failed: ${tok.error_description || tok.error || r.status}. Start again with ${APP}_sign_in.`);
  if (!tok.refresh_token) return failed(ctx, "Google didn't give a refresh token, so the sign-in wouldn't last; start again.");
  let email;
  try { email = JSON.parse(new TextDecoder().decode(fromB64(tok.id_token.split(".")[1]))).email; } catch {}
  if (!email) return failed(ctx, "Google didn't say which account signed in; start again.");
  ctx.user.secret(`refresh_${slug(email)}`, tok.refresh_token, [OAUTH]);
  ctx.user.secret("verifier", null);
  ctx.user.append("account", { email });
  tokens.set(`${ctx.user.id}/${email}`, { value: tok.access_token, expires: Date.now() + (tok.expires_in || 3600) * 1000 });
  return `Signed in as ${email}.`;
}

const str = (description) => ({ type: "string", description });
const accountInput = str(`which account: its address or a part of it only it has; default the first, as ${APP}_accounts lists them`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });

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
    description: "Sign in to a Google account, or another one: returns a link to open in a browser; then pass the address the browser ends up on to finish_sign_in.",
    input: { type: "object", properties: { code: str("the address the browser was sent to after signing in, to finish here instead") } },
    async run(ctx, { code }) {
      if (!ctx.user) throw new Error("this server doesn't keep your data: use one of your own servers");
      if (code) return finishSignIn(ctx, code);
      return `Open this link in a browser on any device and sign in. The browser then fails to load a 127.0.0.1 page: copy that page's whole address and pass it to ${APP}_finish_sign_in.\n\n${await startSignIn(ctx)}`;
    },
  },
  finish_sign_in: {
    description: "Finish signing in: the address the browser ended up on after sign_in's link (or just its code). Answers { signed_in: false, why } when Google didn't sign in.",
    input: { type: "object", required: ["code"], properties: { code: str("the address, or its code") } },
    run: (ctx, { code }) => finishSignIn(ctx, code),
  },
  set_client: {
    description: "Set the Google OAuth client to sign in with (a Desktop app client): its id here, its secret with one secret set; for you, or with scope global (admins) for everyone.",
    input: { type: "object", required: ["client_id"], properties: { client_id: str("the client's id"), client_secret: str("its secret, kept as a secret"), scope: str("user (the default) or global") } },
    run(ctx, { client_id, client_secret, scope }) {
      const s = scope === "global" ? ctx.global : ctx.user;
      if (!s) throw new Error(`${APP} isn't installed for ${scope || "you"} here`);
      if (scope === "global" && s.role !== "admin") throw new Error("only admins set everyone's client");
      s.append("setting", { key: "client_id", value: String(client_id) });
      if (client_secret) s.secret("client_secret", client_secret, [OAUTH]);
      return "Saved.";
    },
  },
  sign_out: {
    description: "Forget a Google account here.",
    input: { type: "object", required: ["account"], properties: { account: str("the account's address, or a part of it only it has") } },
    async run(ctx, { account: name }) {
      const email = await account(ctx, name);
      ctx.user.secret(`refresh_${slug(email)}`, null);
      ctx.user.append("forget", { email });
      tokens.delete(`${ctx.user.id}/${email}`);
      return `Signed out ${email}. To revoke access too, remove it at myaccount.google.com/permissions.`;
    },
  },
};

const signInSchema = {
  user: "create table accounts (email text primary key, added integer not null); create table settings (key text primary key, value text);",
  global: "create table settings (key text primary key, value text);",
};
const signInOn = {
  account(db, e) { db.exec("insert or ignore into accounts values (?, ?)", [e.data.email, e.time]); },
  forget(db, e) { db.exec("delete from accounts where email = ?", [e.data.email]); },
  setting(db, e) {
    if (e.scope === "global" && e.role !== "admin") return;
    db.exec("insert or replace into settings values (?, ?)", [e.data.key, JSON.stringify(e.data.value)]);
  },
};

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


// ---- spreadsheets ----

/// A spreadsheet's id, given it or its address.
const sheetId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give spreadsheet: its id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const ss = (v) => `spreadsheets/${encodeURIComponent(sheetId(v))}`;
const sheetInput = str("the spreadsheet's id, or its address (docs.google.com/spreadsheets/d/…)");
const rowsInput = { type: "array", items: { type: "array", items: {} }, description: "rows, each an array of values; text is read as if typed, so =SUM(A1:A3) is a formula and 3/11 a date" };

/// The range asked for, or the whole first tab.
async function rangeOr(ctx, email, spreadsheet, range) {
  if (range) return String(range);
  const s = await api(ctx, email, "GET", ss(spreadsheet), { query: { fields: "sheets.properties.title" } });
  return `'${s.sheets[0].properties.title.replace(/'/g, "''")}'`;
}

export default {
  name: APP,
  title: TITLE,
  icon: "tablecells",
  color: "#34a853",
  description: "Your Google Sheets: read ranges, add rows, change cells and make spreadsheets.",
  instructions: `Sheets is the person's Google Sheets. ${APP}_spreadsheet lists a spreadsheet's tabs; ${APP}_read reads a range (A1 notation, like Budget!A1:F20; the first tab when left out). ${APP}_append adds rows after a table, ${APP}_update writes over a range and ${APP}_create makes a spreadsheet: change only what the person asked. Spreadsheets are given by id or address. With several accounts, each operation takes account.`,
  schema: signInSchema,

  ops: {
    ...signInOps,
    spreadsheet: {
      reads: true,
      description: "A spreadsheet's title, address and tabs (each with its size).",
      input: withAccount({ spreadsheet: sheetInput }, ["spreadsheet"]),
      async run(ctx, { account: a, spreadsheet }) {
        const s = await api(ctx, await account(ctx, a), "GET", ss(spreadsheet), { query: { fields: "spreadsheetId,spreadsheetUrl,properties(title,timeZone,locale),sheets.properties(sheetId,title,index,gridProperties)" } });
        return {
          id: s.spreadsheetId, title: s.properties.title, url: s.spreadsheetUrl, time_zone: s.properties.timeZone,
          tabs: s.sheets.map(({ properties: p }) => ({ title: p.title, id: p.sheetId, rows: p.gridProperties?.rowCount, columns: p.gridProperties?.columnCount })),
        };
      },
    },
    read: {
      reads: true,
      description: "The values in a range, as rows (as shown, unless formulas is set), with the range they came from.",
      input: withAccount({
        spreadsheet: sheetInput, range: str("A1 notation, like Budget!A1:F20 or Budget; the whole first tab when left out"),
        formulas: { type: "boolean", description: "give formulas rather than what they show" },
        limit: { type: "integer", description: "at most this many rows; default 500" },
      }, ["spreadsheet"]),
      async run(ctx, { account: a, spreadsheet, range, formulas, limit }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "GET", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}`,
          { query: { valueRenderOption: formulas ? "FORMULA" : "FORMATTED_VALUE" } });
        const rows = r.values || [];
        const most = Number(limit) || 500;
        return { range: r.range, rows: rows.slice(0, most), ...(rows.length > most ? { more: `${rows.length - most} more rows: read a smaller range` } : {}) };
      },
    },
    append: {
      description: "Add rows after the table in a range (or the first tab). This changes the person's spreadsheet: do it only when they asked.",
      input: withAccount({ spreadsheet: sheetInput, range: str("the table's range or tab, like Expenses or Expenses!A:E; the first tab when left out"), rows: rowsInput }, ["spreadsheet", "rows"]),
      async run(ctx, { account: a, spreadsheet, range, rows }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "POST", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}:append`,
          { query: { valueInputOption: "USER_ENTERED", insertDataOption: "INSERT_ROWS" }, json: { values: rows } });
        return `Added ${r.updates?.updatedRows ?? rows.length} rows at ${r.updates?.updatedRange}.`;
      },
    },
    update: {
      confirm: true,
      description: "Write rows over a range, from its top-left cell, replacing what was there. Do it only when the person asked.",
      input: withAccount({ spreadsheet: sheetInput, range: str("where to write, like Budget!B2 or Budget!B2:D5"), rows: rowsInput }, ["spreadsheet", "range", "rows"]),
      async run(ctx, { account: a, spreadsheet, range, rows }) {
        const r = await api(ctx, await account(ctx, a), "PUT", `${ss(spreadsheet)}/values/${encodeURIComponent(String(range))}`,
          { query: { valueInputOption: "USER_ENTERED" }, json: { values: rows } });
        return `Wrote ${r.updatedCells} cells in ${r.updatedRange}.`;
      },
    },
    create: {
      description: "Make a spreadsheet in the person's Drive, with tabs and first rows if given. Answers its id and address.",
      input: withAccount({
        title: str("its title"), tabs: { type: "array", items: { type: "string" }, description: "its tabs' names; default one, Sheet1" },
        rows: { ...rowsInput, description: "rows to put in the first tab, from A1" },
      }, ["title"]),
      async run(ctx, { account: a, title, tabs, rows }) {
        const email = await account(ctx, a);
        const s = await api(ctx, email, "POST", "spreadsheets", { json: { properties: { title }, ...(tabs?.length ? { sheets: tabs.map((t) => ({ properties: { title: t } })) } : {}) } });
        if (rows?.length) {
          const first = `'${s.sheets[0].properties.title.replace(/'/g, "''")}'!A1`;
          await api(ctx, email, "PUT", `${ss(s.spreadsheetId)}/values/${encodeURIComponent(first)}`, { query: { valueInputOption: "USER_ENTERED" }, json: { values: rows } });
        }
        return { id: s.spreadsheetId, title: s.properties.title, url: s.spreadsheetUrl };
      },
    },
  },

  on: signInOn,

  screens: {
    settings: { title: TITLE, html: settingsScreen(TITLE, "Sign in to Google, and your agents can read your spreadsheets and, when you ask, add rows and change cells.") },
  },
};

/// The settings screen a shell opens for a connector (SPEC.md "Connectors"): the accounts signed in, each with
/// whether it still works and a way to sign it out, and signing in another, by link and pasted address. Opened with
/// { add: true } (a shell's Add account), it starts signing in at once.
function settingsScreen(title, blurb) {
  return `
<style>
  .card { background: var(--card); border-radius: 12px; padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--accent); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--accent); background: none; border: 0; padding: 4px 0; }
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
    ? '<h2 class="section">Accounts</h2><div class="list">' + accounts.map((a, i) => \`<div class="row plain"><span class="dot\${a.ok ? "" : " warn"}"></span><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub">\${one.esc(a.ok ? (i ? "Signed in" : "Signed in · used first") : a.why || "Needs signing in again")}</div></div><button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`).join("") + "</div>"
    : '<p class="muted">${blurb}</p>';
  for (const b of document.querySelectorAll("[data-out]")) b.onclick = async () => { await one.call("sign_out", { account: b.dataset.out }); show(); };
  const c = document.getElementById("connect");
  c.innerHTML = '<button class="primary" id="start">' + (accounts.length ? "Add account" : "Sign in to Google") + '</button><div id="err" class="error small"></div>';
  document.getElementById("start").onclick = start;
}
async function start() {
  const c = document.getElementById("connect");
  let link;
  try {
    const r = await one.call("sign_in");
    link = typeof r === "string" ? r.split("\\n").pop() : r.url;
  } catch (e) { document.getElementById("err").textContent = e.message; return; }
  c.innerHTML = \`<ol><li><a href="\${one.esc(link)}">Sign in to Google</a> and allow ${title}.</li><li>The browser then goes to a page that doesn't load. Copy its whole address and paste it here.</li></ol>
    <input id="code" placeholder="http://127.0.0.1:4747/hooks/…/oauth?code=…" autocapitalize="none" autocorrect="off"><button class="primary" id="finish">Sign in</button><div id="err" class="error small"></div>\`;
  document.getElementById("finish").onclick = async () => {
    try { const r = await one.call("finish_sign_in", { code: document.getElementById("code").value }); if (r?.signed_in === false) throw new Error(r.why); show(); }
    catch (e) { document.getElementById("err").textContent = e.message; }
  };
}
one.on("changed", show);
show().then(() => { if (one.params.add) start(); });
</script>`;
}
