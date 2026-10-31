// Google Drive for apps: your own Drive accounts, signed in once. The app keeps each account's refresh token as one of
// its own secrets, which the node fills in only for Google's token address, so no app or agent ever reads it. Other
// apps use it as ctx.apps.drive.<op>, Drive's own API by another name (files_get, files_list, files_create,
// files_upload, files_update, generate_ids).

const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/drive";
const API = "https://www.googleapis.com/drive/v3";
// Where Google sends the browser after sign-in. Nothing answers there: the person copies the address it fails to load.
const REDIRECT = "http://127.0.0.1:4747/hooks/drive/oauth";

const slug = (email) => email.toLowerCase().replace(/[^a-z0-9]+/g, "_");
const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

/// What Drive said, with its status: callers tell a missing file (404) from a refused one.
export class DriveError extends Error {}
const needsConnecting = () => new DriveError("Drive needs connecting: sign in to Google Drive again (drive_sign_in).");

async function setting(ctx, key) {
  for (const s of [ctx.user, ctx.global]) {
    const [row] = s ? await s.query("select value from settings where key = ?", [key]) : [];
    if (row) return JSON.parse(row.value);
  }
  return null;
}

async function accounts(ctx) {
  if (!ctx.user) throw new Error("Drive accounts are in your own scope, which this server doesn't hold");
  return ctx.user.query("select email, added from accounts order by added");
}

/// The account to use: the one named, else the first.
async function account(ctx, email) {
  const all = await accounts(ctx);
  if (!all.length) throw needsConnecting();
  if (!email) return all[0];
  const found = all.find((a) => a.email.toLowerCase() === String(email).toLowerCase());
  if (!found) throw new Error(`no Drive account ${email}; they are: ${all.map((a) => a.email).join(", ")}`);
  return found;
}

const tokens = new Map(); // email → { value, expires }, for this runtime only
async function token(ctx, email) {
  const t = tokens.get(email);
  if (t && t.expires > Date.now() + 60_000) return t.value;
  const client = await setting(ctx, "client_id");
  if (!client) throw new Error("Drive has no OAuth client yet: an admin sets one with drive_set_client");
  const r = await fetch(`https://${OAUTH}/token`, {
    method: "POST",
    headers: { "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=refresh_token&client_id=${encodeURIComponent(client)}&client_secret={{client_secret}}&refresh_token={{refresh_${slug(email)}}}`,
  });
  const body = await r.json().catch(() => ({}));
  if (!r.ok || !body.access_token) {
    if (r.status === 400 || r.status === 401) throw needsConnecting();
    throw new DriveError(`Drive ${r.status}: ${body.error_description || body.error || "Google didn't answer"}`);
  }
  tokens.set(email, { value: body.access_token, expires: Date.now() + (body.expires_in || 3600) * 1000 });
  return body.access_token;
}

async function api(ctx, email, method, path, { query, json, raw, type } = {}) {
  const acc = await account(ctx, email);
  const q = query ? `?${new URLSearchParams(Object.entries(query).filter(([, v]) => v !== undefined && v !== null).map(([k, v]) => [k, String(v)]))}` : "";
  const r = await fetch(`${path.startsWith("http") ? "" : API}${path}${q}`, {
    method,
    headers: { authorization: `Bearer ${await token(ctx, acc.email)}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
    body: json ? JSON.stringify(json) : raw,
  });
  const text = await r.text();
  if (r.status === 401) { tokens.delete(acc.email); throw needsConnecting(); }
  if (!r.ok) {
    let message = text.slice(0, 200);
    try { message = JSON.parse(text).error?.message || message; } catch {}
    throw new DriveError(`Drive ${r.status}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}

const fromB64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const accountInput = { type: "string", description: "which Drive account, by address; the first when left out" };

export default {
  name: "drive",
  title: "Google Drive",
  icon: "externaldrive",
  color: "#fbbc04",
  instructions: "Google Drive is your own Drive accounts. drive_accounts lists them; drive_sign_in connects one (a link to open, then drive_finish_sign_in with the address the browser ends up on). The files_* operations are Drive's own API (v3): files_get, files_list (q is Drive's search), files_create, files_upload, files_update (addParents and removeParents move a file), generate_ids.",
  schema: {
    user: "create table accounts (email text primary key, added integer not null); create table settings (key text primary key, value text);",
    global: "create table settings (key text primary key, value text);",
  },

  ops: {
    accounts: {
      reads: true,
      description: "Your Drive accounts, the default first: each { account, ok }, with why when ok is false and it needs signing in again.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        return Promise.all((await accounts(ctx)).map(async ({ email: account }) => {
          try { await token(ctx, account); return { account, ok: true }; } catch (e) { return { account, ok: false, why: e.message }; }
        }));
      },
    },
    set_client: {
      description: "Set the Google OAuth client Drive signs in with: its id here, and its secret (read from stdin with one secret set drive client_secret --hosts oauth2.googleapis.com). scope global (admins) or user.",
      input: { type: "object", required: ["client_id"], properties: { client_id: { type: "string" }, scope: { type: "string" } } },
      async run(ctx, { client_id, scope = "global" }) {
        const s = scope === "global" ? ctx.global : ctx.user;
        if (!s) throw new Error(`drive isn't installed for ${scope}`);
        if (scope === "global" && s.role !== "admin") throw new Error("only admins set everyone's client");
        s.append("setting", { key: "client_id", value: client_id });
        return "Saved.";
      },
    },
    sign_in: {
      description: "Start connecting a Google Drive account: answers a link to open in a browser. After signing in, the browser goes to an address that doesn't load; pass that address to finish_sign_in.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const client = await setting(ctx, "client_id");
        if (!client) throw new Error("Drive has no OAuth client yet: an admin sets one with drive_set_client");
        if (!ctx.user) throw new Error("this server doesn't hold your scope");
        const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
        const challenge = b64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
        // The verifier waits as a secret, filled in only when the code is exchanged at Google.
        ctx.user.secret("verifier", verifier, [OAUTH]);
        const q = new URLSearchParams({ client_id: client, redirect_uri: REDIRECT, response_type: "code", scope: SCOPES, access_type: "offline", prompt: "consent", code_challenge: challenge, code_challenge_method: "S256" });
        return { url: `https://accounts.google.com/o/oauth2/v2/auth?${q}` };
      },
    },
    finish_sign_in: {
      description: "Finish connecting Drive: the address the browser ended up on after sign_in's link (or just its code).",
      input: { type: "object", required: ["code"], properties: { code: { type: "string" } } },
      async run(ctx, { code }) {
        const client = await setting(ctx, "client_id");
        const given = String(code).trim();
        const value = given.includes("code=") ? new URL(given.startsWith("http") ? given : `http://x/?${given.split("?").pop()}`).searchParams.get("code") : given;
        if (!value) throw new Error("that address has no code: copy the whole address the browser ended up on");
        const r = await fetch(`https://${OAUTH}/token`, {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body: `grant_type=authorization_code&code=${encodeURIComponent(value)}&client_id=${encodeURIComponent(client)}&client_secret={{client_secret}}&redirect_uri=${encodeURIComponent(REDIRECT)}&code_verifier={{verifier}}`,
        });
        const body = await r.json().catch(() => ({}));
        if (!r.ok || !body.refresh_token) throw new Error(`Google didn't sign you in: ${body.error_description || body.error || r.status}. Start again with sign_in.`);
        const who = await (await fetch("https://openidconnect.googleapis.com/v1/userinfo", { headers: { authorization: `Bearer ${body.access_token}` } })).json();
        if (!who.email) throw new Error("Google didn't say which account that is");
        ctx.user.secret(`refresh_${slug(who.email)}`, body.refresh_token, [OAUTH]);
        ctx.user.secret("verifier", null);
        ctx.user.append("account", { email: who.email });
        tokens.set(who.email, { value: body.access_token, expires: Date.now() + (body.expires_in || 3600) * 1000 });
        return `Connected ${who.email}.`;
      },
    },
    sign_out: {
      description: "Forget a Drive account.",
      input: { type: "object", required: ["account"], properties: { account: accountInput } },
      async run(ctx, { account: email }) {
        const acc = await account(ctx, email);
        ctx.user.secret(`refresh_${slug(acc.email)}`, null);
        ctx.user.append("forget", { email: acc.email });
        return `Forgot ${acc.email}.`;
      },
    },
    files_get: {
      description: "A file's or folder's metadata (fields like id,name,parents,mimeType,trashed). fileId root is My Drive.",
      input: { type: "object", required: ["fileId"], properties: { fileId: { type: "string" }, fields: { type: "string" }, account: accountInput } },
      run: (ctx, { fileId, fields, account: a }) => api(ctx, a, "GET", `/files/${encodeURIComponent(fileId)}`, { query: { fields, supportsAllDrives: true } }),
    },
    files_list: {
      description: "Files matching Drive's search q (like 'root' in parents and trashed = false), with fields, orderBy, pageSize and pageToken.",
      input: { type: "object", properties: { q: { type: "string" }, fields: { type: "string" }, orderBy: { type: "string" }, pageSize: { type: "number" }, pageToken: { type: "string" }, account: accountInput } },
      run: (ctx, { account: a, ...query }) => api(ctx, a, "GET", "/files", { query }),
    },
    files_create: {
      description: "Make a file without content, like a folder (mimeType application/vnd.google-apps.folder): body is its metadata (id from generate_ids, name, parents, appProperties).",
      input: { type: "object", required: ["body"], properties: { body: { type: "object" }, fields: { type: "string" }, account: accountInput } },
      run: (ctx, { body, fields, account: a }) => api(ctx, a, "POST", "/files", { json: body, query: { fields } }),
    },
    files_upload: {
      description: "Upload a file: metadata (id, name, parents, appProperties), its type, and its bytes as data (base64) or blob (a blob's hash).",
      input: { type: "object", required: ["metadata", "type"], properties: { metadata: { type: "object" }, type: { type: "string" }, data: { type: "string" }, blob: { type: "string" }, fields: { type: "string" }, account: accountInput } },
      async run(ctx, { metadata, type, data, blob, fields, account: a }) {
        const bytes = blob ? await ctx.blob.get(blob) : fromB64(data || "");
        const boundary = "one-drive-upload";
        const head = new TextEncoder().encode(`--${boundary}\r\ncontent-type: application/json; charset=utf-8\r\n\r\n${JSON.stringify({ ...metadata, mimeType: metadata.mimeType || type })}\r\n--${boundary}\r\ncontent-type: ${type}\r\n\r\n`);
        const tail = new TextEncoder().encode(`\r\n--${boundary}--\r\n`);
        const body = new Uint8Array(head.length + bytes.length + tail.length);
        body.set(head); body.set(bytes, head.length); body.set(tail, head.length + bytes.length);
        return api(ctx, a, "POST", "https://www.googleapis.com/upload/drive/v3/files", { raw: body, type: `multipart/related; boundary=${boundary}`, query: { uploadType: "multipart", fields } });
      },
    },
    files_update: {
      description: "Change a file's metadata (body: name, appProperties) and move it (addParents, removeParents: folder ids). Its id and links stay.",
      input: { type: "object", required: ["fileId"], properties: { fileId: { type: "string" }, body: { type: "object" }, addParents: { type: "string" }, removeParents: { type: "string" }, fields: { type: "string" }, account: accountInput } },
      run: (ctx, { fileId, body = {}, addParents, removeParents, fields, account: a }) =>
        api(ctx, a, "PATCH", `/files/${encodeURIComponent(fileId)}`, { json: body, query: { addParents, removeParents, fields } }),
    },
    generate_ids: {
      description: "Ids for files not made yet, so making one twice (a retry) never makes two.",
      input: { type: "object", required: ["count"], properties: { count: { type: "number" }, account: accountInput } },
      run: (ctx, { count, account: a }) => api(ctx, a, "GET", "/files/generateIds", { query: { count, space: "drive", type: "files" } }),
    },
  },

  on: {
    account(db, e) { db.exec("insert or ignore into accounts values (?, ?)", [e.data.email, e.time]); },
    forget(db, e) { db.exec("delete from accounts where email = ?", [e.data.email]); },
    setting(db, e) {
      if (e.scope === "global" && e.role !== "admin") return;
      db.exec("insert or replace into settings values (?, ?)", [e.data.key, JSON.stringify(e.data.value)]);
    },
  },

  screens: {
    settings: { title: "Google Drive", html: settingsScreen("Google Drive", "Sign in to Google Drive, and apps like Claims save a copy of what you add to your own Drive.") },
  },
};

/// The settings screen a shell opens for a connected app (SPEC.md "Connected apps"): the accounts signed in, each with
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
    try { await one.call("finish_sign_in", { code: document.getElementById("code").value }); show(); }
    catch (e) { document.getElementById("err").textContent = e.message; }
  };
}
one.on("changed", show);
show().then(() => { if (one.params.add) start(); });
</script>`;
}
