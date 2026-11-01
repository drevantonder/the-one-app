// Google Calendar: the person's own Google calendars, through Google's API. Signing in is apps/gmail's (and
// apps/drive's) way: the OAuth client's id is a setting, its secret and each account's refresh token are the app's
// secrets, sent only to Google's token address, and the browser ends on a 127.0.0.1 address the person pastes back.
// (The Apple Calendar on a Mac is the `calendar` driver; this is Google's own, from any server.)

const APP = "google-calendar";
const TITLE = "Google Calendar";
const OAUTH = "oauth2.googleapis.com";
const SCOPES = "openid email https://www.googleapis.com/auth/calendar";
const API = "https://www.googleapis.com/calendar/v3/";
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
  description: "Your Google calendars, from any of your servers: events, and adding and changing them.",
  instructions: `Google Calendar is the person's Google calendars (the Mac's Apple Calendar is the calendar app). ${APP}_events lists events in a time range (default the coming week); ${APP}_event shows one. ${APP}_create_event, ${APP}_update_event and ${APP}_delete_event change the calendar: do them only when the person asked. Guests are told only with notify. With several accounts, each operation takes account.`,
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

  on: signInOn,

  screens: {
    settings: {
      title: "Google Calendar",
      html: settingsScreen("Google Calendar", "Sign in to Google, and your agents can read your calendars and, when you ask, add and change events."),
    },
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
