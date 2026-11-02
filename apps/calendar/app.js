// Calendar: Calendar on the person's Mac. The calendar sidecar lives in the Mac app, which macOS lets reach Calendar;
// this app is what people and agents use, wherever they are.

/// Runs one of the calendar sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.calendar[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs calendar for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Calendar runs on a Mac with The One App open, and none of yours is online with it.";

export default {
  name: "calendar",
  title: "Calendar",
  icon: "calendar",
  color: "#ff3b30",
  description: "Calendar on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Calendar is the person's Apple Calendar through their Mac: every account it has, such as iCloud, Google or Exchange. calendar_events lists events in a time range, by default the coming week. Times are ISO 8601; without an offset they're in the Mac's time zone.",
  uses: { sidecars: ["calendar"] },

  ops: {
    accounts: {
      reads: true,
      description: "The Macs it runs on: each { account, ok }, the device’s name, ok while it’s online.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.servers()).filter((s) => (s.sidecars || []).includes("calendar")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      confirm: true,
      description: "Stop using Calendar on one of your devices: removes it from that device, for one of its owners.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the device, by name" } } },
      run: (ctx, { account }) => {
        ctx.sidecar("calendar", account, false);
        return `Calendar is removed from ${account}.`;
      },
    },
    calendars: {
      description: "The calendars, with whether each can be changed.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "calendars", input),
    },
    events: {
      description: "Events in a time range, earliest first.",
      input: {"properties": {"calendar": {"description": "a calendar's name; omit for every calendar", "type": "string"}, "from": {"description": "ISO 8601; default the start of today", "type": "string"}, "limit": {"description": "at most this many; default 200", "type": "integer"}, "query": {"description": "only events whose title, location or notes contain this", "type": "string"}, "to": {"description": "ISO 8601; a date includes that whole day; default a week after from", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "events", input),
    },
    create: {
      description: "Create an event.",
      input: {"properties": {"all_day": {"description": "an all-day event", "type": "boolean"}, "calendar": {"description": "a calendar's name; default is the default calendar", "type": "string"}, "end": {"description": "an ISO 8601 date-time, or the last day of an all-day event; default an hour after start, or the same day", "type": "string"}, "location": {"description": "where", "type": "string"}, "notes": {"description": "more detail", "type": "string"}, "start": {"description": "an ISO 8601 date-time (2026-11-01T09:00), or a date for an all-day event", "type": "string"}, "title": {"description": "what it is", "type": "string"}}, "required": ["title", "start"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "create", input),
    },
    update: {
      description: "Change an event: only the fields given.",
      input: {"properties": {"all_day": {"description": "an all-day event", "type": "boolean"}, "calendar": {"description": "a calendar's name; default is the default calendar", "type": "string"}, "end": {"description": "an ISO 8601 date-time, or the last day of an all-day event; default an hour after start, or the same day", "type": "string"}, "future": {"description": "for a repeating event, this occurrence and every later one, not just this one", "type": "boolean"}, "id": {"description": "the event's id, as events gives it; for a repeating event it names one occurrence", "type": "string"}, "location": {"description": "where", "type": "string"}, "notes": {"description": "more detail", "type": "string"}, "start": {"description": "an ISO 8601 date-time (2026-11-01T09:00), or a date for an all-day event", "type": "string"}, "title": {"description": "what it is", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "update", input),
    },
    delete: {
      description: "Delete an event.",
      input: {"properties": {"future": {"description": "for a repeating event, this occurrence and every later one, not just this one", "type": "boolean"}, "id": {"description": "the event's id, as events gives it; for a repeating event it names one occurrence", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "delete", input),
    },
  },
};
