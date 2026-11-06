// Contacts: Contacts on the person's Mac. Its service (service/) is a helper app inside the Mac app, which macOS lets
// reach Contacts; this app is what people and agents use, wherever they are.

const CARD = "{ id, name, nickname, organization, job, phones, emails, addresses, birthday, urls }, each only when the card has it";

export default {
  name: "contacts",
  title: "Contacts",
  tagline: "Your Mac's contacts",
  icon: "user-circle",
  color: "#8e8e93",
  description: "Contacts on your Mac, for your apps and agents",
  needs: "Needs a Mac",
  instructions: "Contacts are the person's Apple Contacts, through their Mac. contacts_search finds people by name, email address or phone number; contacts_me is the person's own card.",
  uses: { services: ["contacts"] },
  service: { network: [], person: true, platforms: ["mac"] },

  tools: {
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
      run: (ctx, input) => ctx.services.contacts.search(input),
    },
    get: {
      reads: true,
      description: `One contact's card, by its id from contacts_search: ${CARD}. Use it to look again at someone already found; contacts_search gives the same cards.`,
      input: { type: "object", required: ["id"], properties: { id: { type: "string", description: "the contact's id, as contacts_search gives it" } } },
      run: (ctx, input) => ctx.services.contacts.get(input),
    },
    me: {
      reads: true,
      description: `The person's own card, the one Contacts marks as theirs: ${CARD}. Use it for their own name, number or address.`,
      input: { type: "object", properties: {} },
      run: (ctx, input) => ctx.services.contacts.me(input),
    },
  },
};
