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


const str = (description) => ({ type: "string", description });
const accountInput = str(`which account: its address or a part of it only it has, as ${APP}_accounts lists them; default the first`);
const withAccount = (properties, required) => ({ type: "object", properties: { ...properties, account: accountInput }, ...(required ? { required } : {}) });

/// Calls Google's API as `email`; `missing` is what to say when what the path names isn't there.
async function api(ctx, email, method, path, { query, json, missing } = {}) {
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
    if (missing && (r.status === 404 || r.status === 410)) throw new Error(missing);
    throw new Error(`${TITLE}: ${message}`);
  }
  return text ? JSON.parse(text) : null;
}

// ---- calendars and events ----

const cal = (id) => encodeURIComponent(id || "primary");
const noCalendar = (id) => `no calendar ${id}: calendar ids come from ${APP}_calendars`;
const noEvent = (id, calendar) => `no event ${id} in ${calendar ? `calendar ${calendar}` : "your primary calendar"}: event ids come from ${APP}_events, with the same calendarId`;
const DAY = 86_400_000;

/// Whether an EventDateTime's dateTime needs a timeZone: it has no offset, and none was given.
const zoneless = (t) => t?.dateTime && !t.timeZone && !/(Z|[+-]\d{2}:?\d{2})$/i.test(String(t.dateTime).trim());

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
  return (await api(ctx, email, "GET", `calendars/${cal(calendar)}`, { missing: noCalendar(calendar) })).timeZone;
}

const when = (what, more = "") => ({
  type: "object",
  description: `${what}, as Calendar's EventDateTime: { dateTime: "2026-11-02T09:30:00+11:00" }, { dateTime: "2026-11-02T09:30:00", timeZone: "Australia/Sydney" } (without either, the calendar's time zone), or { date: "2026-11-02" } for a whole day${more}`,
  properties: {
    dateTime: str("an RFC 3339 date-time, like 2026-11-02T09:30:00+11:00"), date: str("a whole day, like 2026-11-02"),
    timeZone: str("an IANA time zone, like Australia/Sydney"),
  },
});
const calendarId = str(`the calendar's id, like someone@gmail.com or …@group.calendar.google.com, from ${APP}_calendars; default primary`);
const eventId = str(`the event's id, from ${APP}_events (one repeat's id, like abc123_20261103T000000Z, names just that repeat)`);
const sendUpdates = { type: "string", enum: ["all", "externalOnly", "none"], description: "who Google emails about it: all guests, externalOnly (guests outside your organisation), or none (the default)" };
const eventFields = {
  calendarId,
  summary: str("the event's title, like Dentist"),
  start: when("when it starts"),
  end: when("when it ends", ", the day after its last; default an hour after start, or the next day"),
  location: str("where, like an address or a room"), description: str("notes, as plain text or simple HTML"),
  attendees: { type: "array", description: "the guests, each { email, optional }, like [{ \"email\": \"sam@example.org\" }]; Google tells them only with sendUpdates", items: { type: "object", required: ["email"], properties: { email: str("their address"), optional: { type: "boolean", description: "whether they're optional" } } } },
  recurrence: { type: "array", items: { type: "string" }, description: "how it repeats, as RFC 5545 lines, like [\"RRULE:FREQ=WEEKLY;BYDAY=MO;COUNT=10\"]" },
  sendUpdates,
};

/// The Event resource events.insert and events.patch take, from the inputs given: a dateTime without an offset or a
/// timeZone is in the calendar's zone, and a new event's end defaults to an hour (or a day) after its start.
async function eventBody(ctx, email, input, existing) {
  const body = {};
  for (const k of ["summary", "location", "description", "attendees", "recurrence"]) if (input[k] !== undefined) body[k] = input[k];
  for (const k of ["start", "end"]) if (input[k]) body[k] = { ...input[k] };
  if (body.attendees) body.attendees = body.attendees.map((a) => (typeof a === "string" ? { email: a } : a));
  if (zoneless(body.start) || zoneless(body.end)) {
    const zone = await zoneOf(ctx, email, input.calendarId);
    for (const t of [body.start, body.end]) if (zoneless(t)) t.timeZone = zone;
  }
  if (body.start && !body.end && !existing) {
    const s = body.start;
    body.end = s.date
      ? { date: new Date(new Date(`${s.date}T00:00:00Z`).getTime() + DAY).toISOString().slice(0, 10) }
      : { dateTime: new Date(new Date(instant(s.dateTime, s.timeZone)).getTime() + 3_600_000).toISOString() };
  }
  return body;
}

export default {
  name: APP,
  title: "Google Calendar",
  icon: "calendar",
  color: "#4285f4",
  description: "Your Google calendars, from any of your devices: events, and adding and changing them",
  instructions: `Google Calendar is the person's Google calendars (the Mac's Apple Calendar is the calendar app). ${APP}_events lists events in a time range (default the coming week); ${APP}_event shows one. ${APP}_create_event, ${APP}_update_event and ${APP}_delete_event change the calendar, with Calendar's own Event fields (summary, start, end, ...): do them only when the person asked. Guests are told only with sendUpdates. With several accounts, each operation takes account.`,
  vault: VAULT,
  schema: signInSchema,

  ops: {
    ...signInOps,
    calendars: {
      reads: true,
      description: "The account's calendars (Calendar's calendarList.list), the primary first, as text: each one's id (what calendarId takes elsewhere), name, your access (owner, writer or reader) and time zone.",
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
      description: `Events in a time range (Calendar's events.list, each repeat on its own, soonest first), as text: each event's id, start – end (RFC 3339, or a date for a whole day), summary, and where. Default: the coming week of the primary calendar. ${APP}_event gives one event's guests, link and notes. A long list ends with More: give pageToken <token>.`,
      input: withAccount({
        calendarId,
        timeMin: str("the start of the range, RFC 3339, like 2026-11-02T00:00:00+11:00 (without an offset, or a date like 2026-11-02, in the calendar's time zone); default now"),
        timeMax: str("the end of the range, the same way; default a week after timeMin"),
        q: str("words to find in events' summaries, places, notes and guests, like standup"),
        maxResults: { type: "integer", description: "at most this many; default 50, at most 250" },
        pageToken: str("the next page: the token the last events ended with (\"More: give pageToken …\"), with the same calendarId, timeMin and timeMax"),
      }),
      async run(ctx, { account: a, calendarId, timeMin: from, timeMax: to, q, maxResults, pageToken }) {
        const email = await account(ctx, a);
        const zone = from || to ? await zoneOf(ctx, email, calendarId) : undefined;
        const timeMin = instant(from, zone, ctx.now());
        const timeMax = instant(to, zone, new Date(timeMin).getTime() + 7 * DAY);
        const r = await api(ctx, email, "GET", `calendars/${cal(calendarId)}/events`, {
          query: { timeMin, timeMax, q, singleEvents: true, orderBy: "startTime", maxResults: Math.min(Number(maxResults) || 50, 250), pageToken },
          missing: noCalendar(calendarId),
        });
        const items = r.items || [];
        const head = `${r.summary || calendarId || "primary"} (${r.timeZone}), ${timeMin} to ${timeMax}:`;
        return [head, ...(items.length ? items.map(line) : ["Nothing."])].join("\n") + (r.nextPageToken ? `\n\nMore: give pageToken ${r.nextPageToken}` : "");
      },
    },
    event: {
      reads: true,
      description: "One event (Calendar's events.get), as text: its summary, when, how it repeats, where, organizer, guests and their answers, its Meet link, its link in Google Calendar and its notes.",
      input: withAccount({ eventId, calendarId }, ["eventId"]),
      async run(ctx, { account: a, eventId, calendarId }) {
        return whole(await api(ctx, await account(ctx, a), "GET", `calendars/${cal(calendarId)}/events/${encodeURIComponent(eventId)}`, { missing: noEvent(eventId, calendarId) }));
      },
    },
    create_event: {
      reads: false,
      description: "Add an event to a calendar (Calendar's events.insert). This changes the person's calendar: do it only when they asked. Guests are told only with sendUpdates. Gives its id and link.",
      input: withAccount(eventFields, ["summary", "start"]),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const e = await api(ctx, email, "POST", `calendars/${cal(input.calendarId)}/events`, { json: await eventBody(ctx, email, input), query: { sendUpdates: input.sendUpdates || "none" }, missing: noCalendar(input.calendarId) });
        return `Added "${e.summary}" (${at(e.start)} – ${at(e.end)}), id ${e.id}: ${e.htmlLink}`;
      },
    },
    update_event: {
      reads: false,
      description: "Change an event (Calendar's events.patch): only what's given changes; attendees and recurrence, given, replace the whole list. A repeating event's id changes every repeat; one repeat's id changes just that one. This changes the person's calendar: do it only when they asked. Guests are told only with sendUpdates.",
      input: withAccount({ eventId, ...eventFields }, ["eventId"]),
      async run(ctx, input) {
        const email = await account(ctx, input.account);
        const e = await api(ctx, email, "PATCH", `calendars/${cal(input.calendarId)}/events/${encodeURIComponent(input.eventId)}`, { json: await eventBody(ctx, email, input, true), query: { sendUpdates: input.sendUpdates || "none" }, missing: noEvent(input.eventId, input.calendarId) });
        return `Changed "${e.summary}" (${at(e.start)} – ${at(e.end)}).`;
      },
    },
    delete_event: {
      reads: false,
      confirm: true,
      description: "Delete an event for good (Calendar's events.delete): a repeating event's id deletes every repeat, one repeat's id just that one. Do it only when the person asked. Guests are told only with sendUpdates.",
      input: withAccount({ eventId, calendarId, sendUpdates }, ["eventId"]),
      async run(ctx, { account: a, eventId, calendarId, sendUpdates }) {
        const email = await account(ctx, a);
        const e = await api(ctx, email, "GET", `calendars/${cal(calendarId)}/events/${encodeURIComponent(eventId)}`, { missing: noEvent(eventId, calendarId) });
        if (e.status === "cancelled") throw new Error(noEvent(eventId, calendarId));
        await api(ctx, email, "DELETE", `calendars/${cal(calendarId)}/events/${encodeURIComponent(eventId)}`, { query: { sendUpdates: sendUpdates || "none" }, missing: noEvent(eventId, calendarId) });
        return `Deleted "${e.summary || "(no title)"}" (${at(e.start)}).`;
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
    if (/Ask .* to set up/.test(e.message) && b) b.disabled = true;
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
