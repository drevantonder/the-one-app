// Contacts: Contacts on the user's Mac. Its service (service/) is a helper app inside the Mac app, which macOS lets
// reach Contacts; this app is what users and agents use, wherever they are.

const LABELLED = (what) => ({ type: "array", items: { type: "object", required: ["label", what], properties: { label: { type: "string" }, [what]: { type: "string" } } } });
const CONTACT = {
  type: "object",
  required: ["id", "name"],
  properties: {
    id: { type: "string", description: "the contact's id, for contacts_get" },
    name: { type: "string", description: "their name, or their organization when the card has no name" },
    nickname: { type: "string" },
    organization: { type: "string" },
    job: { type: "string" },
    phones: LABELLED("number"),
    emails: LABELLED("address"),
    addresses: LABELLED("address"),
    birthday: { type: "string", description: "year-month-day, or month-day when the card has no year" },
    urls: { type: "array", items: { type: "string" } },
  },
};
const CARD = "{ id, name, nickname, organization, job, phones, emails, addresses, birthday, urls }, each only when the card has it";

export default {
  name: "contacts",
  title: "Contacts",
  tagline: "Your Mac's contacts",
  icon: "user-circle",
  color: "#8e8e93",
  description: "Contacts on your Mac, for your apps and agents",
  needs: "Needs a Mac",
  instructions: "Contacts are the user's Apple Contacts, through their Mac. contacts_search finds people by name, email address or phone number; contacts_me is the user's own card.",
  uses: { services: ["contacts"] },
  service: { network: [], user: true, platforms: ["mac"] },

  tools: {
    search: {
      title: "Find people in Contacts",
      reads: true,
      description: `Find people in the user's contacts by name, email address or phone number, for someone's number or address before writing to them. Gives each ${CARD}, by name.`,
      input: {
        type: "object",
        required: ["query"],
        properties: {
          query: { type: "string", description: "a name or part of one (Sam, Sam Lee), an email address, or a phone number in any format" },
          limit: { type: "integer", description: "at most this many, by name; default 25" },
        },
      },
      output: { type: "array", items: CONTACT, description: "the matching contacts, by name" },
      run: (ctx, input) => ctx.services.contacts.search(input),
    },
    get: {
      title: "One contact's card",
      reads: true,
      description: `One contact's card, by its id from contacts_search: ${CARD}. Use it to look again at someone already found; contacts_search gives the same cards.`,
      input: { type: "object", required: ["id"], properties: { id: { type: "string", description: "the contact's id, as contacts_search gives it" } } },
      output: CONTACT,
      run: (ctx, input) => ctx.services.contacts.get(input),
    },
    me: {
      title: "The user's own card",
      reads: true,
      description: `The user's own card, the one Contacts marks as theirs: ${CARD}. Use it for their own name, number or address.`,
      input: { type: "object", properties: {} },
      output: CONTACT,
      run: (ctx, input) => ctx.services.contacts.me(input),
    },
  },
};
