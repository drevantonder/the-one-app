// Google Drive for apps: your own Drive accounts, signed in once. Each account's refresh token is a secret field of
// its Google account item in the Vault, which the node fills in only for Google's token address, so no app or agent
// ever reads it; signing in is the same in every Google app. Other apps use it as ctx.apps.drive.<op>, Drive's own API by another name (files_get, files_list, files_create,
// files_upload, files_update, generate_ids).

const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/drive";
const API = "https://www.googleapis.com/drive/v3";
// Where Google sends the browser after sign-in. Nothing answers there: the person copies the address it fails to load.
const APP = "drive";
const TITLE = "Google Drive";
// Where Google sends the browser back: the node on the person's device, at the port this install's nodes answer on.
const redirect = (ctx) => `http://127.0.0.1:${ctx.port}/hooks/${APP}/oauth`;

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

/// What Drive said, with its status: callers tell a missing file (404) from a refused one.
export class DriveError extends Error {}
const needsConnecting = () => new DriveError("Drive needs connecting: sign in to Google Drive again (drive_sign_in).");

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
  if (!ctx.user) throw new Error("this device doesn't keep your data, so it can't reach your accounts: use one of your own devices");
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
      if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
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


async function api(ctx, email, method, path, { query, json, raw, type } = {}) {
  const acc = { email: await account(ctx, email).catch(() => { throw needsConnecting(); }) };
  const access = await token(ctx, acc.email).catch((e) => { throw new DriveError(e.message); });
  const q = query ? `?${new URLSearchParams(Object.entries(query).filter(([, v]) => v !== undefined && v !== null).map(([k, v]) => [k, String(v)]))}` : "";
  const r = await fetch(`${path.startsWith("http") ? "" : API}${path}${q}`, {
    method,
    headers: { authorization: `Bearer ${access}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
    body: json ? JSON.stringify(json) : raw,
  });
  const text = await r.text();
  if (r.status === 401) { tokens.delete(`${ctx.user.id}/${acc.email}`); throw needsConnecting(); }
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
  icon: "google-drive-logo",
  color: "#fbbc04",
  description: "Your Google Drive: find, read, upload and move files",
  instructions: "Google Drive is your own Drive accounts. drive_accounts lists them; drive_sign_in connects one (a link to open, then drive_finish_sign_in with the address the browser ends up on). The files_* operations are Drive's own API (v3): files_get, files_list (q is Drive's search), files_create, files_upload, files_update (addParents and removeParents move a file), generate_ids.",
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    // Drive's sign-in answers { url }, as apps like Claims take it.
    sign_in: { ...signInOps.sign_in, async run(ctx, input) {
      if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
      return input.code ? finishSignIn(ctx, input.code) : { url: await startSignIn(ctx, input.account) };
    } },
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
      description: "Upload a file: metadata (id, name, parents, appProperties), its type, and its bytes as data (base64) or blob (a kept file's hash, as file put prints it).",
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


  screens: {
    settings: { title: "Google Drive", html: settingsScreen("Google Drive", "Sign in to Google Drive, and apps like Claims save a copy of what you add to your own Drive.") },
  },
};

/// The settings screen a shell opens for a connector: the accounts signed in, each with
/// whether it works, a way to allow it again and to sign it out, and signing in another, by link and pasted address.
/// Opened with { add: true } (a shell's Add Account), it starts signing in at once.
function settingsScreen(title, blurb) {
  return `
<style>
  .card { background: var(--card); border-radius: 12px; padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--tint); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--tint); background: none; border: 0; padding: 4px 0; margin-left: 12px; }
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
