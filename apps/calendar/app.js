// Calendar: Calendar on the user's Mac. Its service (service/) is a helper app inside the Mac app, which macOS lets reach Calendar;
// this app is what users and agents use, wherever they are.

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

const EVENT = {
  type: "object",
  required: ["id", "title", "start", "end"],
  properties: {
    id: { type: "string", description: "the event's id; a repeating event's names one occurrence" },
    title: { type: "string" },
    calendar: { type: "string", description: "the calendar's name" },
    start: { type: "string", description: "an ISO 8601 date-time in the Mac's time zone, or a date for an all-day event" },
    end: { type: "string", description: "an ISO 8601 date-time, or an all-day event's last day" },
    all_day: { type: "boolean", description: "true only for an all-day event" },
    repeats: { type: "boolean", description: "true only for a repeating event" },
    location: { type: "string" },
    notes: { type: "string" },
    url: { type: "string" },
    attendees: { type: "array", items: { type: "string" }, description: "each one's name, or their address when there's none" },
  },
};

export default {
  name: "calendar",
  title: "Calendar",
  tagline: "Your Mac's calendar",
  icon: "calendar",
  color: "#ff3b30",
  description: "Calendar on your Mac, for your apps and agents",
  needs: "Needs a Mac",
  instructions: "Calendar is the user's Apple Calendar through their Mac: every account it has, such as iCloud, Google or Exchange. calendar_events lists events in a time range, by default the coming week. Times are ISO 8601; without an offset they're in the Mac's time zone.",
  uses: { services: ["calendar"] },
  service: { network: [], user: true, platforms: ["mac"] },

  tools: {
    calendars: {
      title: "The Mac's calendars",
      reads: true,
      description: "Every calendar on the Mac: each { name, id, account, writable, default }, account being where it syncs (iCloud, Google, Exchange). Use a name as calendar in calendar_events or calendar_create; only writable ones take new events.",
      input: { type: "object", properties: {} },
      output: {
        type: "array",
        items: {
          type: "object",
          required: ["name", "id", "writable"],
          properties: {
            name: { type: "string" },
            id: { type: "string" },
            account: { type: "string", description: "where it syncs: iCloud, Google, Exchange" },
            writable: { type: "boolean", description: "whether it takes new events" },
            default: { type: "boolean", description: "true only for the calendar new events go to" },
          },
        },
      },
      run: (ctx, input) => ctx.services.calendar.calendars(input),
    },
    events: {
      title: "Events in a time range",
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
      output: { type: "array", items: EVENT, description: "the events, earliest first" },
      run: (ctx, input) => ctx.services.calendar.events(input),
    },
    create: {
      title: "Add an event",
      reads: false,
      description: "Add an event to one of the Mac's calendars, which syncs it to that calendar's account; invitees aren't added. Gives back the new event as calendar_events lists it, with its id for calendar_update or calendar_delete.",
      input: { type: "object", required: ["title", "start"], properties: FIELDS },
      output: EVENT,
      run: (ctx, input) => ctx.services.calendar.create(input),
    },
    update: {
      title: "Change an event",
      reads: false,
      description: "Change an event: only the fields given change, and moving start keeps its length unless end is given too. Gives back the event as it is now.",
      input: { type: "object", required: ["id"], properties: { id: ID, ...FIELDS, calendar: { type: "string", description: "move it to this calendar, by its name as calendar_calendars lists them" }, future: FUTURE } },
      output: EVENT,
      run: (ctx, input) => ctx.services.calendar.update(input),
    },
    delete: {
      title: "Delete an event",
      reads: false,
      confirm: true,
      description: "Delete an event from its calendar, or one occurrence of a repeating event (with future, every later one too). It can't be undone: the calendar's account deletes it everywhere.",
      input: { type: "object", required: ["id"], properties: { id: ID, future: FUTURE } },
      output: { type: "string", description: "that it was deleted, in words" },
      run: (ctx, input) => ctx.services.calendar.delete(input),
    },
  },
};
