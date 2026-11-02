// Contacts: Contacts on the person's Mac. The contacts sidecar lives in the Mac app, which macOS lets reach Contacts;
// this app is what people and agents use, wherever they are.

/// Runs one of the contacts sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.contacts[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs contacts for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Contacts runs on a Mac with The One App open, and none of yours is online with it.";

export default {
  name: "contacts",
  title: "Contacts",
  icon: "user-circle",
  color: "#8e8e93",
  description: "Contacts on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Contacts are the person's Apple Contacts, through their Mac. contacts_search finds people by name, email address or phone number; contacts_me is the person's own card.",
  uses: { sidecars: ["contacts"] },

  ops: {
    accounts: {
      reads: true,
      description: "The Macs it runs on: each { account, ok }, the device’s name, ok while it’s online.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.sidecars || []).includes("contacts")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      confirm: true,
      description: "Stop using Contacts on one of your devices: removes it from that device, for one of its owners.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the device, by name" } } },
      run: (ctx, { account }) => {
        ctx.sidecar("contacts", account, false);
        return `Contacts is removed from ${account}.`;
      },
    },
    search: {
      description: "Find contacts by name, email address or phone number.",
      input: {"properties": {"limit": {"description": "at most this many; default 25", "type": "integer"}, "query": {"description": "a name or part of one, an email address, or a phone number", "type": "string"}}, "required": ["query"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "search", input),
    },
    get: {
      description: "One contact, by the id search gives.",
      input: {"properties": {"id": {"description": "the contact's id", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "get", input),
    },
    me: {
      description: "The person's own contact card.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "me", input),
    },
  },
};
