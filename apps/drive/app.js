// Google Drive for apps: your own Drive accounts, signed in once. Each account's refresh token is a secret field of
// its Google account item in the Vault, which the node fills in only for Google's token address, so no app or agent
// ever reads it; signing in is the same in every Google app. Its operations are Drive's own API (v3) by its own names
// (files_get, files_list, files_create, files_upload, files_update, generate_ids), which agents and other apps
// (ctx.apps.drive.<op>) already know.

const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/drive";
const API = "https://www.googleapis.com/drive/v3";
const APP = "drive";
const TITLE = "Google Drive";
// Where Google sends the browser back: the node on the person's device, at the port this install's nodes answer on.
const redirect = (ctx) => `http://127.0.0.1:${ctx.port}/hooks/${APP}/oauth`;

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

/// What Drive said, with its status: callers tell a missing file (404) from a refused one.
export class DriveError extends Error {}
// "needs connecting" is how apps that save to Drive tell this from other errors.
const needsConnecting = () => new DriveError(`Drive needs connecting: sign in to a Google account first; ${APP}_sign_in signs one in`);

// The Vault keeps what signing in needs, the same for every Google app: one "Google sign-in" item, the OAuth client
// (its id, and its secret), and a "Google account" item for each account (its address, its refresh token and the
// scopes it allowed), sent only to Google's token address. Signing in to an account another Google app already uses
// adds this app's scopes to it (incremental consent). The browser ends on a 127.0.0.1 address the person pastes
// back; the verifier waits for it in this app's own data.

const VAULT = {
  client: {
    title: "Google sign-in", type: "Google sign-in", domain: "google.com", hosts: [OAUTH],
    why: "Make one in Google Cloud Console: APIs & Services › Credentials › Create OAuth client ID, type Desktop app. It's set once, for everyone's Google apps.",
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


/// Calls Drive's API as the account named (the first when none is); `missing` is what to say to a 404, `hint` what
/// to add to a 400. Errors keep Drive's status first ("Drive 404: …"), as apps that save to Drive tell them apart by it.
async function api(ctx, name, method, path, { query, json, raw, type, missing, hint } = {}) {
  if (!(await accounts(ctx)).length) throw needsConnecting();
  const email = await account(ctx, name);
  const access = await token(ctx, email).catch((e) => { throw new DriveError(e.message); });
  const q = query ? `?${new URLSearchParams(Object.entries(query).filter(([, v]) => v !== undefined && v !== null && v !== "").map(([k, v]) => [k, String(v)]))}` : "";
  const r = await fetch(`${path.startsWith("http") ? "" : API}${path}${q}`, {
    method,
    headers: { authorization: `Bearer ${access}`, ...(json ? { "content-type": "application/json" } : type ? { "content-type": type } : {}) },
    body: json ? JSON.stringify(json) : raw,
  });
  const text = await r.text();
  if (r.status === 401) { tokens.delete(`${ctx.user.id}/${email}`); throw needsConnecting(); }
  if (!r.ok) {
    let message = text.slice(0, 200);
    try { message = JSON.parse(text).error?.message || message; } catch {}
    if (r.status === 404 && missing) message = missing;
    else if (r.status === 400 && hint) message = `${message.replace(/\.$/, "")}: ${hint}`;
    throw new DriveError(`Drive ${r.status}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}

const fromB64 = (s) => Uint8Array.from(atob(s), (c) => c.charCodeAt(0));
const str = (description) => ({ type: "string", description });
const accountInput = str(`which account: its address or a part of it only it has, as ${APP}_accounts lists them; default the first`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });
const noFile = (id) => `no file ${id}, or not one this account can see: file ids come from drive_files_list (root is My Drive)`;
const FIELDS_HINT = "fields is Drive's field list, like id,name,mimeType,parents,modifiedTime,webViewLink";

// What each operation gives when fields is left out: what an agent uses, without Drive's kind on everything.
const FILE = "id,name,mimeType,parents,modifiedTime,size,webViewLink,trashed";
const LIST = "nextPageToken,files(id,name,mimeType,parents,modifiedTime,webViewLink)";
const MADE = "id,name,mimeType,parents,webViewLink";

const fileId = str("a file's or folder's id, from drive_files_list (or the part after /d/ or /folders/ in its address); root is My Drive");
const fields = (usual) => str(`which fields to give back, in Drive's field syntax, like id,name,parents,appProperties; default ${usual}`);
const metadataInput = (what) => ({
  type: "object",
  description: `${what}, as Drive's File resource: name (like "Budget 2026.pdf"), parents (folder ids, like ["root"]), mimeType (application/vnd.google-apps.folder for a folder), description, appProperties (your app's own key-value pairs), id (one from drive_generate_ids, so a retry never makes two)`,
});

export default {
  name: "drive",
  title: "Google Drive",
  icon: "google-drive-logo",
  color: "#fbbc04",
  description: "Your Google Drive: find, upload, rename and move files",
  instructions: "Google Drive is the person's Drive accounts, through Drive's own API (v3), by its names: files_list finds files with Drive's q search (name contains 'budget' and trashed = false), files_get gives one, files_create makes a folder, files_upload uploads a file, files_update renames, moves (addParents, removeParents) or trashes one, generate_ids gives ids to make files with safely. Results give the fields asked for; fields asks for more. With several accounts, each operation takes account.",
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    files_get: {
      reads: true,
      description: `One file's or folder's metadata, as Drive's files.get gives it: by default ${FILE} (webViewLink opens it; size is in bytes; times are ISO 8601). Its contents: Google Docs and Sheets read them; other files aren't read here.`,
      input: withAccount({ fileId, fields: fields(FILE) }, ["fileId"]),
      run: (ctx, { fileId, fields, account: a }) => api(ctx, a, "GET", `/files/${encodeURIComponent(fileId)}`, { query: { fields: fields || FILE, supportsAllDrives: true }, missing: noFile(fileId), hint: FIELDS_HINT }),
    },
    files_list: {
      reads: true,
      description: `Find files and folders with Drive's search (files.list): { files: [${LIST.slice(LIST.indexOf("(") + 1, -1)}], nextPageToken } by default. When nextPageToken is there, give it as pageToken, with the same q, for the rest.`,
      input: withAccount({
        q: str("Drive's search query, like name contains 'budget' and trashed = false, or 'FOLDER_ID' in parents and trashed = false, or mimeType = 'application/vnd.google-apps.folder', or fullText contains 'invoice', or modifiedTime > '2026-10-01T00:00:00'; default every file, trashed ones too"),
        fields: fields(LIST),
        orderBy: str("how to sort, like modifiedTime desc, or folder,name, or name; keys: createdTime, folder, modifiedByMeTime, modifiedTime, name, quotaBytesUsed, recency, sharedWithMeTime, starred, viewedByMeTime"),
        pageSize: { type: "integer", description: "at most this many files, 1 to 1000; default 100" },
        pageToken: str("the nextPageToken the last files_list gave, with the same q, for the next page"),
      }),
      run: (ctx, { account: a, fields, ...query }) => api(ctx, a, "GET", "/files", {
        query: { ...query, fields: fields || LIST },
        hint: "q is Drive's search syntax, like name contains 'budget' and trashed = false, with values in single quotes; orderBy is like modifiedTime desc; " + FIELDS_HINT,
      }),
    },
    files_create: {
      reads: false,
      description: `Make a file with no content, like a folder (files.create): body is its metadata. Gives ${MADE} by default. To upload a file's bytes, files_upload.`,
      input: withAccount({ body: metadataInput("the new file's metadata, like { name: \"Receipts\", mimeType: \"application/vnd.google-apps.folder\", parents: [\"root\"] }"), fields: fields(MADE) }, ["body"]),
      run: (ctx, { body, fields, account: a }) => api(ctx, a, "POST", "/files", { json: body, query: { fields: fields || MADE, supportsAllDrives: true }, missing: "no folder among body.parents: folder ids come from drive_files_list (root is My Drive)", hint: FIELDS_HINT }),
    },
    files_upload: {
      reads: false,
      description: `Upload a file (files.create with its bytes): metadata names it and says where it goes. Gives ${MADE} by default.`,
      input: withAccount({
        metadata: metadataInput("the new file's metadata, like { name: \"receipt.pdf\", parents: [\"FOLDER_ID\"] }; name defaults to file's"),
        file: { type: "object", format: "file", description: "the file to upload, like one gmail_attachment gave back" },
        type: str("its MIME type, like application/pdf; default file's, or metadata's mimeType"),
        data: str("or its bytes, in base64"), blob: str("or a kept file's hash, as file put prints it"),
        fields: fields(MADE),
      }, ["metadata"]),
      async run(ctx, { metadata = {}, file, type, data, blob, fields, account: a }) {
        if (!file && !blob && data == null) throw new Error("give file (or data, base64, or blob): what to upload");
        const bytes = blob ? await ctx.blob.get(blob) : fromB64(file ? file.base64 : data);
        type = type || file?.type || metadata.mimeType;
        if (!type) throw new Error("give type: the file's MIME type, like application/pdf");
        const meta = { ...metadata, name: metadata.name || file?.name, mimeType: metadata.mimeType || type };
        const boundary = "one-drive-upload";
        const head = new TextEncoder().encode(`--${boundary}\r\ncontent-type: application/json; charset=utf-8\r\n\r\n${JSON.stringify(meta)}\r\n--${boundary}\r\ncontent-type: ${type}\r\n\r\n`);
        const tail = new TextEncoder().encode(`\r\n--${boundary}--\r\n`);
        const body = new Uint8Array(head.length + bytes.length + tail.length);
        body.set(head); body.set(bytes, head.length); body.set(tail, head.length + bytes.length);
        return api(ctx, a, "POST", "https://www.googleapis.com/upload/drive/v3/files", {
          raw: body, type: `multipart/related; boundary=${boundary}`, query: { uploadType: "multipart", fields: fields || MADE, supportsAllDrives: true },
          missing: "no folder among metadata.parents: folder ids come from drive_files_list (root is My Drive)", hint: FIELDS_HINT,
        });
      },
    },
    files_update: {
      reads: false,
      description: `Rename, move or trash a file or folder (files.update); its id and links stay. Gives ${MADE} by default. Moving: addParents the new folder, removeParents the old one (its parents, from files_get).`,
      input: withAccount({
        fileId,
        body: { type: "object", description: "what changes, as Drive's File resource, like { name: \"Budget 2026.pdf\" }, { trashed: true } (to the trash, where it stays 30 days) or { trashed: false } (back out), description, appProperties" },
        addParents: str("folder ids to put it in, comma-separated, from drive_files_list (root is My Drive)"),
        removeParents: str("folder ids to take it out of, comma-separated, as files_get gives its parents"),
        fields: fields(MADE),
      }, ["fileId"]),
      run: (ctx, { fileId, body = {}, addParents, removeParents, fields, account: a }) =>
        api(ctx, a, "PATCH", `/files/${encodeURIComponent(fileId)}`, { json: body, query: { addParents, removeParents, fields: fields || MADE, supportsAllDrives: true }, missing: noFile(fileId), hint: FIELDS_HINT }),
    },
    generate_ids: {
      reads: true,
      description: "Ids for files not made yet ({ ids }), to give files_create or files_upload as metadata's id: making one twice with the same id (a retry) never makes two.",
      input: withAccount({ count: { type: "integer", description: "how many, 1 to 1000" } }, ["count"]),
      run: async (ctx, { count, account: a }) => ({ ids: (await api(ctx, a, "GET", "/files/generateIds", { query: { count, space: "drive", type: "files" } })).ids }),
    },
  },


  screens: {
    settings: { title: "Google Drive", html: settingsScreen("Google Drive", "Sign in, and your apps can save copies to your Drive.") },
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
    // No Google sign-in yet: an admin sets one up for everyone, then it goes on; anyone else is told whom to ask, and
    // can't sign in until then.
    if (/No Google sign-in/.test(e.message) && one.vault) {
      try { if (await one.vault("client")) return start(account, b); } catch (err) { e = err; }
    }
    busy(b, false, b?.id === "start" ? label() : b?.textContent);
    const ask = /Ask (.+?) to set up (.+?)(?::|$)/.exec(e.message);
    if (ask) {
      connect.className = "";
      connect.innerHTML = one.problem({ title: ask[2] + " isn't set up", text: "Ask " + ask[1] + " to set it up. It's done once for everyone." });
      return;
    }
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
