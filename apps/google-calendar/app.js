// Google Calendar: the person's own Google calendars, through Google's API. Signing in is the same in every
// Google app: see "signing in" below. (The Apple Calendar on a Mac is the `calendar` sidecar; this is Google's own,
// from any device.)

const APP = "google-calendar";
const TITLE = "Google Calendar";
const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/calendar";
const API = "https://www.googleapis.com/calendar/v3/";
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

// ---- calendars and events ----

const cal = (id) => encodeURIComponent(id || "primary");
const DAY = 86_400_000;

/// A time Google takes: "2026-11-02" is a whole day; a date and time without an offset is in `zone`.
function moment(v, zone) {
  if (v == null || v === "") return undefined;
  const s = String(v).trim();
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return { date: s };
  const hasOffset = /(Z|[+-]\d{2}:?\d{2})$/i.test(s);
  return hasOffset ? { dateTime: s } : { dateTime: s.length === 16 ? `${s}:00` : s, timeZone: zone };
}

/// An instant for a list's bounds: given with or without an offset (then in `zone`), or a date.
function instant(v, zone, fallback) {
  if (!v) return new Date(fallback).toISOString();
  const s = String(v).trim();
  if (/(Z|[+-]\d{2}:?\d{2})$/i.test(s)) return new Date(s).toISOString();
  // Without an offset: the calendar's zone, found by asking what the wall time there is at that UTC time.
  const guess = new Date(/^\d{4}-\d{2}-\d{2}$/.test(s) ? `${s}T00:00:00Z` : `${s}Z`);
  const there = new Date(guess.toLocaleString("en-US", { timeZone: zone || "UTC" }));
  const here = new Date(guess.toLocaleString("en-US", { timeZone: "UTC" }));
  return new Date(guess.getTime() - (there - here)).toISOString();
}

const at = (t) => t?.dateTime || t?.date || "";

function line(e) {
  const allDay = !!e.start?.date;
  let s = `${e.id}  ${allDay ? `${e.start.date} (all day)` : `${at(e.start)} – ${at(e.end)}`}  ${e.summary || "(no title)"}`;
  if (e.location) s += `\n  at ${e.location}`;
  if (e.attendees?.length) s += `\n  with ${e.attendees.length} people`;
  if (e.status === "cancelled") s += "\n  cancelled";
  return s;
}

function whole(e) {
  const out = [`${e.summary || "(no title)"}`, `Id: ${e.id}`, `When: ${e.start?.date ? `${e.start.date} to ${e.end?.date} (all day)` : `${at(e.start)} – ${at(e.end)}`}${e.start?.timeZone ? ` (${e.start.timeZone})` : ""}`];
  if (e.recurrence?.length) out.push(`Repeats: ${e.recurrence.join("; ")}`);
  if (e.location) out.push(`Where: ${e.location}`);
  if (e.organizer?.email) out.push(`Organizer: ${e.organizer.displayName ? `${e.organizer.displayName} <${e.organizer.email}>` : e.organizer.email}`);
  for (const a of e.attendees || []) out.push(`Attendee: ${a.displayName ? `${a.displayName} <${a.email}>` : a.email} (${a.responseStatus}${a.optional ? ", optional" : ""})`);
  if (e.hangoutLink) out.push(`Meet: ${e.hangoutLink}`);
  if (e.htmlLink) out.push(`Link: ${e.htmlLink}`);
  if (e.description) out.push("", e.description);
  return out.join("\n");
}

async function zoneOf(ctx, email, calendar) {
  return (await api(ctx, email, "GET", `calendars/${cal(calendar)}`)).timeZone;
}

const eventFields = {
  summary: str("the event's title"), start: str("when it starts: 2026-11-02T09:30 (in the calendar's time zone unless it has an offset), or 2026-11-02 for a whole day"),
  end: str("when it ends, the same way; a whole day's end is the day after its last (default: an hour after start, or the next day)"),
  time_zone: str("the time zone start and end are in, like Australia/Melbourne; default the calendar's"),
  location: str("where"), description: str("notes"),
  attendees: { type: "array", items: { type: "string" }, description: "guests' email addresses (Google sends them nothing unless notify is given)" },
  notify: { type: "boolean", description: "email the guests about it (default false: nobody is told)" },
  calendar: str("the calendar's id, as calendars lists them; default your primary calendar"),
};

function eventBody(input, zone, existing) {
  const body = {};
  if (input.summary !== undefined) body.summary = input.summary;
  if (input.location !== undefined) body.location = input.location;
  if (input.description !== undefined) body.description = input.description;
  if (input.attendees !== undefined) body.attendees = (Array.isArray(input.attendees) ? input.attendees : String(input.attendees).split(",")).map((a) => a.trim()).filter(Boolean).map((email) => ({ email }));
  const start = moment(input.start, zone);
  let end = moment(input.end, zone);
  if (start && !end && !existing) {
    if (start.date) end = { date: new Date(new Date(`${start.date}T00:00:00Z`).getTime() + DAY).toISOString().slice(0, 10) };
    else {
      const ms = new Date(start.timeZone ? instant(start.dateTime, start.timeZone) : start.dateTime).getTime() + 3_600_000;
      end = { dateTime: new Date(ms).toISOString() };
    }
  }
  if (start) body.start = start;
  if (end) body.end = end;
  return body;
}

export default {
  name: APP,
  title: "Google Calendar",
  icon: "calendar",
  color: "#4285f4",
  description: "Your Google calendars, from any of your devices: events, and adding and changing them",
  instructions: `Google Calendar is the person's Google calendars (the Mac's Apple Calendar is the calendar app). ${APP}_events lists events in a time range (default the coming week); ${APP}_event shows one. ${APP}_create_event, ${APP}_update_event and ${APP}_delete_event change the calendar: do them only when the person asked. Guests are told only with notify. With several accounts, each operation takes account.`,
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    calendars: {
      reads: true,
      description: "Your calendars: id, name, your access, and time zone; the primary first.",
      input: withAccount({}),
      async run(ctx, { account: a }) {
        const email = await account(ctx, a);
        const { items = [] } = await api(ctx, email, "GET", "users/me/calendarList", { query: { maxResults: 250 } });
        items.sort((x, y) => (y.primary ? 1 : 0) - (x.primary ? 1 : 0) || (x.summary || "").localeCompare(y.summary || ""));
        return items.map((c) => `${c.id}  ${c.summaryOverride || c.summary}${c.primary ? " (primary)" : ""}  ${c.accessRole}, ${c.timeZone}${c.selected === false ? ", hidden" : ""}`).join("\n") || "No calendars.";
      },
    },
    events: {
      reads: true,
      description: "Events from one calendar in a time range, soonest first, each repeat on its own: id, start – end, title, place. Default: the coming week of your primary calendar.",
      input: withAccount({
        calendar: eventFields.calendar,
        from: str("the start of the range: 2026-11-02, or 2026-11-02T09:00 (in the calendar's time zone unless it has an offset); default now"),
        to: str("the end of the range, the same way; default a week after from"),
        query: str("words to find in events' titles, places, notes and guests"),
        limit: { type: "integer", description: "at most this many; default 50, at most 250" }, page: str("the page token the previous call gave"),
      }),
      async run(ctx, { account: a, calendar, from, to, query, limit, page }) {
        const email = await account(ctx, a);
        const zone = from || to ? await zoneOf(ctx, email, calendar) : undefined;
        const timeMin = instant(from, zone, ctx.now());
        const timeMax = instant(to, zone, new Date(timeMin).getTime() + 7 * DAY);
        const r = await api(ctx, email, "GET", `calendars/${cal(calendar)}/events`, {
          query: { timeMin, timeMax, q: query, singleEvents: true, orderBy: "startTime", maxResults: Math.min(Number(limit) || 50, 250), pageToken: page },
        });
        const items = r.items || [];
        const head = `${r.summary || calendar || "primary"} (${r.timeZone}), ${timeMin} to ${timeMax}:`;
        return [head, ...(items.length ? items.map(line) : ["Nothing."])].join("\n") + (r.nextPageToken ? `\n\nMore: give page ${r.nextPageToken}` : "");
      },
    },
    event: {
      reads: true,
      description: "One event: when, where, repeats, organizer, guests and their answers, its Meet link and notes.",
      input: withAccount({ id: str("the event's id, as events gives it"), calendar: eventFields.calendar }, ["id"]),
      async run(ctx, { account: a, id, calendar }) {
        return whole(await api(ctx, await account(ctx, a), "GET", `calendars/${cal(calendar)}/events/${encodeURIComponent(id)}`));
      },
    },
    create_event: {
      description: "Add an event to a calendar. This changes the person's calendar: do it only when they asked. Guests are told only with notify.",
      input: withAccount(eventFields, ["summary", "start"]),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const zone = input.time_zone || await zoneOf(ctx, email, input.calendar);
        const e = await api(ctx, email, "POST", `calendars/${cal(input.calendar)}/events`, { json: eventBody(input, zone), query: { sendUpdates: input.notify ? "all" : "none" } });
        return `Added "${e.summary}" (${at(e.start)} – ${at(e.end)}), id ${e.id}: ${e.htmlLink}`;
      },
    },
    update_event: {
      description: "Change an event: only what's given changes. This changes the person's calendar: do it only when they asked. Guests are told only with notify.",
      input: withAccount({ id: str("the event's id"), ...eventFields }, ["id"]),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const zone = input.time_zone || (input.start || input.end ? await zoneOf(ctx, email, input.calendar) : undefined);
        const e = await api(ctx, email, "PATCH", `calendars/${cal(input.calendar)}/events/${encodeURIComponent(input.id)}`, { json: eventBody(input, zone, true), query: { sendUpdates: input.notify ? "all" : "none" } });
        return `Changed "${e.summary}" (${at(e.start)} – ${at(e.end)}).`;
      },
    },
    delete_event: {
      confirm: true,
      description: "Delete an event (all of its repeats, given a repeating event's id). Do it only when the person asked. Guests are told only with notify.",
      input: withAccount({ id: str("the event's id"), calendar: eventFields.calendar, notify: eventFields.notify }, ["id"]),
      async run(ctx, { account: a, id, calendar, notify }) {
        await api(ctx, await account(ctx, a), "DELETE", `calendars/${cal(calendar)}/events/${encodeURIComponent(id)}`, { query: { sendUpdates: notify ? "all" : "none" } });
        return "Deleted the event.";
      },
    },
  },


  screens: {
    settings: {
      title: "Google Calendar",
      html: settingsScreen("Google Calendar", "Sign in to Google, and your agents can read your calendars and, when you ask, add and change events."),
    },
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
    ? '<h2 class="section">Accounts</h2><div class="list">' + accounts.map((a, i) => \`<div class="row plain"><span class="dot\${a.ok ? "" : " warn"}"></span><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub">\${one.esc(a.ok ? (i ? "Signed in" : "Signed in · used first") : one.plain(a.why || "Needs signing in again").title)}</div></div>\${a.ok ? "" : \`<button class="plain" data-in="\${one.esc(a.account)}">Sign in</button>\`}<button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`).join("") + "</div>"
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
    if (/Choose a Google sign-in/.test(e.message) && one.vault) {
      try { if (await one.vault("client")) return start(account); } catch (err) { e = err; }
    }
    document.getElementById("err").textContent = one.plain(e).title;
    return;
  }
  c.innerHTML = \`<ol><li><a href="\${one.esc(link)}">Sign in to Google</a> and allow ${title}.</li><li>The browser then goes to a page that doesn't load. Copy its whole address and paste it here.</li></ol>
    <input id="code" placeholder="http://127.0.0.1:…/hooks/…/oauth?code=…" autocapitalize="none" autocorrect="off"><button class="primary" id="finish">Sign In</button><div id="err" class="error small"></div>\`;
  document.getElementById("finish").onclick = async () => {
    try { const r = await one.call("finish_sign_in", { code: document.getElementById("code").value }); if (r?.signed_in === false) throw new Error(r.why); show(); }
    catch (e) { document.getElementById("err").textContent = one.plain(e).title; }
  };
}
one.on("changed", show);
show().then(() => { if (one.params.add) start(); });
</script>`;
}
