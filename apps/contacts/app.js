// Contacts: Contacts on the person's Mac. The contacts service lives in the Mac app, which macOS lets reach Contacts;
// this app is what people and agents use, wherever they are.

/// Runs one of the contacts service's functions, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function service(ctx, fn, input) {
  try {
    return await ctx.services.contacts[fn](input);
  } catch (err) {
    if (String(err.message).includes("no device runs contacts for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Contacts runs on a Mac with The One App open, and none of yours is online with it.";

const CARD = "{ id, name, nickname, organization, job, phones, emails, addresses, birthday, urls }, each only when the card has it";

export default {
  name: "contacts",
  title: "Contacts",
  tagline: "Your Mac's contacts",
  icon: "user-circle",
  color: "#8e8e93",
  description: "Contacts on your Mac, for your apps and agents",
  connector: true,
  needs: "Needs a Mac",
  instructions: "Contacts are the person's Apple Contacts, through their Mac. contacts_search finds people by name, email address or phone number; contacts_me is the person's own card.",
  uses: { services: ["contacts"] },

  tools: {
    devices: {
      reads: true,
      description: "The Macs Contacts runs on: each { device, online }, device the Mac's name.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.services || []).includes("contacts")).map((s) => ({ device: s.name, online: s.online })),
    },
    remove: {
      reads: false,
      confirm: true,
      description: "Stop using Contacts on one of your Macs: removes it from that Mac, for one of its owners. The contacts themselves stay as they are.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the Mac's name, as contacts_devices lists it" } } },
      run: (ctx, { device }) => {
        ctx.service("contacts", device, false);
        return `Contacts is removed from ${device}.`;
      },
    },
    search: {
      reads: true,
      description: `Find people in the person's contacts by name, email address or phone number, for someone's number or address before writing to them. Gives each ${CARD}, by name.`,
      input: {
        type: "object",
        required: ["query"],
        properties: {
          query: { type: "string", description: "a name or part of one (Sam, Sam Lee), an email address, or a phone number in any format" },
          limit: { type: "integer", description: "at most this many, by name; default 25" },
        },
      },
      run: (ctx, input) => service(ctx, "search", input),
    },
    get: {
      reads: true,
      description: `One contact's card, by its id from contacts_search: ${CARD}. Use it to look again at someone already found; contacts_search gives the same cards.`,
      input: { type: "object", required: ["id"], properties: { id: { type: "string", description: "the contact's id, as contacts_search gives it" } } },
      run: (ctx, input) => service(ctx, "get", input),
    },
    me: {
      reads: true,
      description: `The person's own card, the one Contacts marks as theirs: ${CARD}. Use it for their own name, number or address.`,
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "me", input),
    },
  },
};
