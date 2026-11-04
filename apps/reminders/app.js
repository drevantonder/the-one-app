// Reminders: Reminders on the person's Mac. The reminders service lives in the Mac app, which macOS lets reach Reminders;
// this app is what people and agents use, wherever they are.

/// Runs one of the reminders service's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function service(ctx, op, input) {
  try {
    return await ctx.services.reminders[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs reminders for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Reminders runs on a Mac with The One App open, and none of yours is online with it.";

const ID = { type: "string", description: "the reminder's id, as reminders_list gives it" };
const PRIORITY = { type: "string", enum: ["high", "medium", "low", "none"], description: "high, medium, low or none" };
const REMINDER = "{ id, title, list, due, priority, notes, completed }, due a date or, with a time, an ISO 8601 date-time in the Mac's time zone, and completed when it was done";

export default {
  name: "reminders",
  title: "Reminders",
  tagline: "Your reminders",
  icon: "list-checks",
  color: "#007aff",
  description: "Reminders on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Reminders are the person's Apple Reminders, through their Mac, and sync to their other Apple devices. reminders_lists lists the lists; reminders_list lists reminders, soonest due first, leaving out completed ones unless asked. reminders_update also completes one.",
  uses: { services: ["reminders"] },

  ops: {
    accounts: {
      reads: true,
      description: "The Macs Reminders runs on: each { account, ok }, account the Mac's name, ok while it's online.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.services || []).includes("reminders")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      reads: false,
      confirm: true,
      description: "Stop using Reminders on one of your Macs: removes it from that Mac, for one of its owners. The reminders themselves stay as they are.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the Mac's name, as reminders_accounts lists it" } } },
      run: (ctx, { account }) => {
        ctx.service("reminders", account, false);
        return `Reminders is removed from ${account}.`;
      },
    },
    lists: {
      reads: true,
      description: "Every reminder list: each { name, id, account, default }, account being where it syncs (iCloud, Exchange). Use a name as list in reminders_list or reminders_create.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "lists", input),
    },
    list: {
      reads: true,
      description: `Reminders not yet done, soonest due first and those with no due date last, from every list or one; completed ones too if asked. Gives each ${REMINDER}.`,
      input: {
        type: "object",
        properties: {
          list: { type: "string", description: "only this list, by its name as reminders_lists lists them; default every list" },
          until: { type: "string", description: "only those due by then: an ISO 8601 date (2026-11-01, the whole day) or date-time (2026-11-01T09:00)" },
          query: { type: "string", description: "only those whose title or notes contain this text, any case" },
          completed: { type: "boolean", description: "include completed reminders too" },
          limit: { type: "integer", description: "at most this many, the soonest due; default 100. For more, narrow by list, until or query" },
        },
      },
      run: (ctx, input) => service(ctx, "list", input),
    },
    create: {
      reads: false,
      description: "Add a reminder to a list, which syncs to the person's other Apple devices; a due time alerts them then. Gives back the new reminder as reminders_list gives them, with its id for reminders_update or reminders_delete.",
      input: {
        type: "object",
        required: ["title"],
        properties: {
          title: { type: "string", description: "what to be reminded of, as the list shows it" },
          list: { type: "string", description: "a list's name, as reminders_lists lists them; default the Mac's default list" },
          due: { type: "string", description: "an ISO 8601 date (2026-11-01), or a date-time (2026-11-01T09:00, in the Mac's time zone without an offset) to be alerted then" },
          notes: { type: "string", description: "more detail, as plain text" },
          priority: PRIORITY,
        },
      },
      run: (ctx, input) => service(ctx, "create", input),
    },
    update: {
      reads: false,
      description: "Change a reminder, or mark it done with completed: only the fields given change. Gives back the reminder as it is now.",
      input: {
        type: "object",
        required: ["id"],
        properties: {
          id: ID,
          title: { type: "string", description: "a new title" },
          list: { type: "string", description: "move it to this list, by its name as reminders_lists lists them" },
          due: { type: "string", description: "an ISO 8601 date or date-time, or none to clear it" },
          notes: { type: "string", description: "new notes, in place of the old" },
          priority: PRIORITY,
          completed: { type: "boolean", description: "true marks it done; false makes it not done again" },
        },
      },
      run: (ctx, input) => service(ctx, "update", input),
    },
    delete: {
      reads: false,
      confirm: true,
      description: "Delete a reminder from its list, on every Apple device. It can't be undone; to tick one off instead, reminders_update with completed.",
      input: { type: "object", required: ["id"], properties: { id: ID } },
      run: (ctx, input) => service(ctx, "delete", input),
    },
  },
};
