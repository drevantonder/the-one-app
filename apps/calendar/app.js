// Calendar: Calendar on the person's Mac. The calendar service lives in the Mac app, which macOS lets reach Calendar;
// this app is what people and agents use, wherever they are.

/// Runs one of the calendar service's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function service(ctx, op, input) {
  try {
    return await ctx.services.calendar[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs calendar for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Calendar runs on a Mac with The One App open, and none of yours is online with it.";

const TIME = "an ISO 8601 date (2026-11-01) or date-time (2026-11-01T09:00); without an offset, in the Mac's time zone";
const ID = { type: "string", description: "the event's id, as calendar_events gives it; a repeating event's names one occurrence" };
const FUTURE = { type: "boolean", description: "for a repeating event: this occurrence and every later one, not just this one" };
const FIELDS = {
  title: { type: "string", description: "what it is, as the calendar shows it" },
  start: { type: "string", description: "when it starts: an ISO 8601 date-time (2026-11-01T09:00, in the Mac's time zone without an offset), or a date (2026-11-01) for an all-day event" },
  end: { type: "string", description: "when it ends: an ISO 8601 date-time, or an all-day event's last day; default an hour after start, or the same day" },
  all_day: { type: "boolean", description: "an all-day event; a date as start makes one too" },
  calendar: { type: "string", description: "a calendar's name, as calendar_calendars lists them; default the Mac's default calendar" },
  location: { type: "string", description: "where, as text: an address or a room" },
  notes: { type: "string", description: "more detail, as plain text" },
};

export default {
  name: "calendar",
  title: "Calendar",
  tagline: "Your Mac's calendar",
  icon: "calendar",
  color: "#ff3b30",
  description: "Calendar on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Calendar is the person's Apple Calendar through their Mac: every account it has, such as iCloud, Google or Exchange. calendar_events lists events in a time range, by default the coming week. Times are ISO 8601; without an offset they're in the Mac's time zone.",
  uses: { services: ["calendar"] },

  tools: {
    devices: {
      reads: true,
      description: "The Macs Calendar runs on: each { device, online }, device the Mac's name. Calendar's own accounts (iCloud, Google, Exchange) are in calendar_calendars.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.services || []).includes("calendar")).map((s) => ({ device: s.name, online: s.online })),
    },
    remove: {
      reads: false,
      confirm: true,
      description: "Stop using Calendar on one of your Macs: removes it from that Mac, for one of its owners. The calendars themselves stay as they are.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the Mac's name, as calendar_devices lists it" } } },
      run: (ctx, { device }) => {
        ctx.service("calendar", device, false);
        return `Calendar is removed from ${device}.`;
      },
    },
    calendars: {
      reads: true,
      description: "Every calendar on the Mac: each { name, id, account, writable, default }, account being where it syncs (iCloud, Google, Exchange). Use a name as calendar in calendar_events or calendar_create; only writable ones take new events.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "calendars", input),
    },
    events: {
      reads: true,
      description: "Events between from and to, by default the coming week, earliest first, from every calendar or one. Gives each { id, title, calendar, start, end, all_day, repeats, location, notes, url, attendees }, as EventKit has them, times in the Mac's time zone. Use it to see what's on, or for an event's id to update or delete.",
      input: {
        type: "object",
        properties: {
          from: { type: "string", description: `the range's start: ${TIME}; default the start of today` },
          to: { type: "string", description: `the range's end: ${TIME}; a date includes that whole day; default a week after from` },
          calendar: { type: "string", description: "only this calendar, by its name as calendar_calendars lists them; default every calendar" },
          query: { type: "string", description: "only events whose title, location or notes contain this text, any case" },
          limit: { type: "integer", description: "at most this many, the earliest; default 200. For more, narrow from and to" },
        },
      },
      run: (ctx, input) => service(ctx, "events", input),
    },
    create: {
      reads: false,
      description: "Add an event to one of the Mac's calendars, which syncs it to that calendar's account; invitees aren't added. Gives back the new event as calendar_events lists it, with its id for calendar_update or calendar_delete.",
      input: { type: "object", required: ["title", "start"], properties: FIELDS },
      run: (ctx, input) => service(ctx, "create", input),
    },
    update: {
      reads: false,
      description: "Change an event: only the fields given change, and moving start keeps its length unless end is given too. Gives back the event as it is now.",
      input: { type: "object", required: ["id"], properties: { id: ID, ...FIELDS, calendar: { type: "string", description: "move it to this calendar, by its name as calendar_calendars lists them" }, future: FUTURE } },
      run: (ctx, input) => service(ctx, "update", input),
    },
    delete: {
      reads: false,
      confirm: true,
      description: "Delete an event from its calendar, or one occurrence of a repeating event (with future, every later one too). It can't be undone: the calendar's account deletes it everywhere.",
      input: { type: "object", required: ["id"], properties: { id: ID, future: FUTURE } },
      run: (ctx, input) => service(ctx, "delete", input),
    },
  },
};
