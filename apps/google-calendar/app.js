// Google Calendar: the person's own Google calendars, through Google's API. Signing in is the same in every
// Google app: see google/google.js. (The Apple Calendar on a Mac is the `calendar` service; this is Google's own,
// from any device.)

import settings from "./dist/settings.html" with { type: "text" };
import { google } from "./google/google.js";

const APP = "google-calendar";
const TITLE = "Google Calendar";
const SCOPES = "openid email https://www.googleapis.com/auth/calendar";
const API = "https://www.googleapis.com/calendar/v3/";
const { account, api, str, withAccount, vault, schema, tools: signIn } = google({ app: APP, title: TITLE, scopes: SCOPES, base: API });

const cal = (id) => encodeURIComponent(id || "primary");
const noCalendar = (id) => `no calendar ${id}: calendar ids come from ${APP}_calendars`;
const noEvent = (id, calendar) => `no event ${id} in ${calendar ? `calendar ${calendar}` : "your primary calendar"}: event ids come from ${APP}_events, with the same calendarId`;
const DAY = 86_400_000;

const zoneless = (t) => t?.dateTime && !t.timeZone && !/(Z|[+-]\d{2}:?\d{2})$/i.test(String(t.dateTime).trim());

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
  tagline: "Your Google events",
  icon: "calendar",
  color: "#4285f4",
  description: "Your Google calendars, from any of your devices: events, and adding and changing them",
  instructions: `Google Calendar is the person's Google calendars (the Mac's Apple Calendar is the calendar app). ${APP}_events lists events in a time range (default the coming week); ${APP}_event shows one. ${APP}_create_event, ${APP}_update_event and ${APP}_delete_event change the calendar, with Calendar's own Event fields (summary, start, end, ...): do them only when the person asked. Guests are told only with sendUpdates. With several accounts, each tool takes account.`,
  vault,
  schema,

  tools: {
    ...signIn,
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
      html: settings,
    },
  },
};
