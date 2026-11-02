// Reminders: Reminders on the person's Mac. The reminders sidecar lives in the Mac app, which macOS lets reach Reminders;
// this app is what people and agents use, wherever they are.

/// Runs one of the reminders sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.reminders[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs reminders for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Reminders runs on a Mac with The One App open, and none of yours is online with it.";

export default {
  name: "reminders",
  title: "Reminders",
  icon: "list-checks",
  color: "#007aff",
  description: "Reminders on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Reminders are the person's Apple Reminders, through their Mac, and sync to their other Apple devices. reminders_lists lists the lists; reminders_list lists reminders, soonest due first, leaving out completed ones unless asked. reminders_update also completes one.",
  uses: { sidecars: ["reminders"] },

  ops: {
    accounts: {
      reads: true,
      description: "The Macs it runs on: each { account, ok }, the device’s name, ok while it’s online.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.sidecars || []).includes("reminders")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      confirm: true,
      description: "Stop using Reminders on one of your devices: removes it from that device, for one of its owners.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the device, by name" } } },
      run: (ctx, { account }) => {
        ctx.sidecar("reminders", account, false);
        return `Reminders is removed from ${account}.`;
      },
    },
    lists: {
      description: "The reminder lists.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "lists", input),
    },
    list: {
      description: "Reminders, soonest due first; only those not completed, unless asked.",
      input: {"properties": {"completed": {"description": "include completed reminders", "type": "boolean"}, "limit": {"description": "at most this many; default 100", "type": "integer"}, "list": {"description": "a list's name; omit for every list", "type": "string"}, "query": {"description": "only reminders whose title or notes contain this", "type": "string"}, "until": {"description": "only reminders due by then: an ISO 8601 date or date-time", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "list", input),
    },
    create: {
      description: "Create a reminder.",
      input: {"properties": {"due": {"description": "an ISO 8601 date (2026-11-01), or a date-time (2026-11-01T09:00) to be alerted then", "type": "string"}, "list": {"description": "a list's name; default is the default list", "type": "string"}, "notes": {"description": "more detail", "type": "string"}, "priority": {"description": "high, medium, low or none", "type": "string"}, "title": {"description": "what to be reminded of", "type": "string"}}, "required": ["title"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "create", input),
    },
    update: {
      description: "Change a reminder, or complete it.",
      input: {"properties": {"completed": {"description": "true completes it; false makes it not done again", "type": "boolean"}, "due": {"description": "an ISO 8601 date or date-time, or \"none\" to clear it", "type": "string"}, "id": {"description": "the reminder's id, as list gives it", "type": "string"}, "list": {"description": "move it to this list", "type": "string"}, "notes": {"description": "new notes", "type": "string"}, "priority": {"description": "high, medium, low or none", "type": "string"}, "title": {"description": "a new title", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "update", input),
    },
    delete: {
      description: "Delete a reminder.",
      input: {"properties": {"id": {"description": "the reminder's id, as list gives it", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "delete", input),
    },
  },
};
