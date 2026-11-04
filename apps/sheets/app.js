// Sheets: the person's own Google Sheets, through Google's API. Signing in is the same in every
// Google app: see "signing in" below.

const APP = "sheets";
const TITLE = "Google Sheets"; // also export default's title, a plain string there for the App Store to read
const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/spreadsheets";
const API = "https://sheets.googleapis.com/v4/";
// Where Google sends the browser back: the install's address, which opens this app's settings screen to finish. The
// Google sign-in (a Web application client) lists it as an authorized redirect URI.
function redirect(ctx) {
  if (!ctx.address) throw new Error("This device is out of date: update it, then sign in again");
  return `${ctx.address}/back`;
}

// ---- signing in ----

const b64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

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
    prompt: "consent", include_granted_scopes: "true", state: `${APP}/settings`, code_challenge: challenge, code_challenge_method: "S256", ...(hint ? { login_hint: hint } : {}),
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


// ---- spreadsheets ----

/// A spreadsheet's id, given it or its address.
const sheetId = (v) => {
  const s = String(v || "").trim();
  if (!s) throw new Error("give spreadsheetId: the spreadsheet's id or address");
  return /\/d\/([a-zA-Z0-9_-]+)/.exec(s)?.[1] || s;
};
const ss = (v) => `spreadsheets/${encodeURIComponent(sheetId(v))}`;
const sheetInput = str("the spreadsheet's id, or its address, like https://docs.google.com/spreadsheets/d/1AbC…/edit; drive_files_list finds spreadsheets (q mimeType = 'application/vnd.google-apps.spreadsheet' and name contains '…')");
const valuesInput = { type: "array", items: { type: "array", items: {} }, description: "rows, each an array of values, like [[\"Date\", \"Amount\"], [\"2026-11-03\", 42]]; text is read as if typed (valueInputOption USER_ENTERED), so =SUM(A1:A3) is a formula and 3/11 a date" };
const noSheet = (v) => `no spreadsheet ${sheetId(v)}: give its id or its address (docs.google.com/spreadsheets/d/…); drive_files_list finds spreadsheets`;
const RANGE_HINT = `ranges are A1 notation, like Budget!A1:F20, Budget!A:C or Budget; ${APP}_spreadsheet lists the sheets`;

/// The range asked for, or the whole first sheet.
async function rangeOr(ctx, email, spreadsheet, range) {
  if (range) return String(range);
  const s = await api(ctx, email, "GET", ss(spreadsheet), { query: { fields: "sheets.properties.title" }, missing: noSheet(spreadsheet) });
  return `'${s.sheets[0].properties.title.replace(/'/g, "''")}'`;
}

export default {
  name: APP,
  title: "Google Sheets",
  tagline: "Your Google Sheets",
  icon: "table",
  color: "#34a853",
  description: "Your Google Sheets: read ranges, add rows, change cells and make spreadsheets",
  instructions: `Sheets is the person's Google Sheets. ${APP}_spreadsheet lists a spreadsheet's sheets (tabs); ${APP}_read reads a range (A1 notation, like Budget!A1:F20; the first sheet when left out). ${APP}_append adds rows after a table, ${APP}_update writes over a range and ${APP}_create makes a spreadsheet: change only what the person asked. Spreadsheets are given as spreadsheetId, an id or address. With several accounts, each operation takes account.`,
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    spreadsheet: {
      reads: true,
      description: "A spreadsheet's title, address, time zone and sheets (tabs), each with its size (Sheets' spreadsheets.get, trimmed): { id, title, url, time_zone, sheets: [{ title, id, rows, columns }] }. Sheet titles are what ranges start with.",
      input: withAccount({ spreadsheetId: sheetInput }, ["spreadsheetId"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet }) {
        const s = await api(ctx, await account(ctx, a), "GET", ss(spreadsheet), { query: { fields: "spreadsheetId,spreadsheetUrl,properties(title,timeZone,locale),sheets.properties(sheetId,title,index,gridProperties)" }, missing: noSheet(spreadsheet) });
        return {
          id: s.spreadsheetId, title: s.properties.title, url: s.spreadsheetUrl, time_zone: s.properties.timeZone,
          sheets: s.sheets.map(({ properties: p }) => ({ title: p.title, id: p.sheetId, rows: p.gridProperties?.rowCount, columns: p.gridProperties?.columnCount })),
        };
      },
    },
    read: {
      reads: true,
      description: "The values in a range (Sheets' values.get): { range, values }, values a row per array, with the range they came from. Over limit rows, it says how many more.",
      input: withAccount({
        spreadsheetId: sheetInput, range: str("A1 notation, like Budget!A1:F20, Budget!A:C or Budget; the whole first sheet when left out"),
        valueRenderOption: { type: "string", enum: ["FORMATTED_VALUE", "UNFORMATTED_VALUE", "FORMULA"], description: "FORMATTED_VALUE: as shown (the default); UNFORMATTED_VALUE: numbers unformatted; FORMULA: formulas, like =SUM(B2:B9)" },
        limit: { type: "integer", description: "at most this many rows; default 500 (the rest are left out, and more says how many)" },
      }, ["spreadsheetId"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, valueRenderOption, limit }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "GET", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}`,
          { query: { valueRenderOption: valueRenderOption || "FORMATTED_VALUE" }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        const rows = r.values || [];
        const most = Number(limit) || 500;
        return { range: r.range, values: rows.slice(0, most), ...(rows.length > most ? { more: `${rows.length - most} more ${rows.length - most === 1 ? "row" : "rows"}: read a smaller range, like one starting at row ${most + 1}` } : {}) };
      },
    },
    append: {
      reads: false,
      description: "Add rows after the table in a range (or the first sheet) (Sheets' values.append); says where they went. This changes the person's spreadsheet: do it only when they asked.",
      input: withAccount({ spreadsheetId: sheetInput, range: str("the table's range or sheet, like Expenses or Expenses!A:E; the first sheet when left out"), values: valuesInput }, ["spreadsheetId", "values"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, values: rows }) {
        const email = await account(ctx, a);
        const r = await api(ctx, email, "POST", `${ss(spreadsheet)}/values/${encodeURIComponent(await rangeOr(ctx, email, spreadsheet, range))}:append`,
          { query: { valueInputOption: "USER_ENTERED", insertDataOption: "INSERT_ROWS" }, json: { values: rows }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        return `Added ${r.updates?.updatedRows ?? rows.length} rows at ${r.updates?.updatedRange}.`;
      },
    },
    update: {
      reads: false,
      confirm: true,
      description: "Write rows over a range, from its top-left cell, replacing what was there (Sheets' values.update); says which cells changed. Do it only when the person asked.",
      input: withAccount({ spreadsheetId: sheetInput, range: str("where to write, in A1 notation, like Budget!B2 or Budget!B2:D5"), values: valuesInput }, ["spreadsheetId", "range", "values"]),
      async run(ctx, { account: a, spreadsheetId: spreadsheet, range, values: rows }) {
        const r = await api(ctx, await account(ctx, a), "PUT", `${ss(spreadsheet)}/values/${encodeURIComponent(String(range))}`,
          { query: { valueInputOption: "USER_ENTERED" }, json: { values: rows }, missing: noSheet(spreadsheet), hint: RANGE_HINT });
        return `Wrote ${r.updatedCells} cells in ${r.updatedRange}.`;
      },
    },
    create: {
      reads: false,
      description: "Make a spreadsheet in the person's Drive (in My Drive; Sheets' spreadsheets.create), with sheets and first rows if given. Gives { id, title, url }.",
      input: withAccount({
        title: str("its title, like Budget 2026"), sheets: { type: "array", items: { type: "string" }, description: "its sheets' (tabs') titles, like [\"Income\", \"Expenses\"]; default one, Sheet1" },
        values: { ...valuesInput, description: `rows to put in the first sheet, from A1, ${valuesInput.description.slice(6)}` },
      }, ["title"]),
      async run(ctx, { account: a, title, sheets: tabs, values: rows }) {
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


  screens: {
    settings: { title: TITLE, html: settingsScreen(TITLE, "Sign in to Google, and your agents can read your spreadsheets and, when you ask, add rows and change cells.") },
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
