// iMessage: Messages on the person's Mac. Its service (service/) is a helper app inside the Mac app, which macOS lets reach Messages;
// this app is what people and agents use, wherever they are.

async function service(ctx, fn, input) {
  try {
    return await ctx.services.imessage[fn](input);
  } catch (err) {
    const why = String(err.message);
    if (/^no chat (called|with) /.test(why)) throw new Error(`${why}: imessage_chats lists the chats; anyone else by their phone number with its country code, or email address`);
    if (why.includes("matches several chats")) throw new Error(`${why}. Give more of the name, or the chat's id from imessage_chats`);
    throw err;
  }
}

const CHAT = "the chat: its name as imessage_chats gives it, a part of the name only one chat has, or its id; or a phone number with its country code (+61 400 000 000) or an email address";
const SINCE = { type: "string", description: "only messages after this: an ISO 8601 date (2026-11-01) or date-time (2026-11-01T09:00), in the Mac's time zone without an offset" };
const LIMIT = { type: "integer", description: "at most this many; default 30" };
const MESSAGE = "{ time, from, text, attachments }, time ISO 8601 in the Mac's time zone, from the sender's name from Contacts (or their number), or me for the person, and attachments the files' names";

export default {
  name: "imessage",
  title: "iMessage",
  tagline: "Your Mac's texts",
  icon: "chat-circle",
  color: "#34c759",
  description: "Messages on your Mac, for your apps and agents",
  needs: "Needs a Mac",
  instructions: "iMessage is the person's Messages on their Mac, iMessage and SMS. imessage_chats lists recent chats; imessage_read reads one, newest last; imessage_search finds messages. Each list pages back with limit and skip. imessage_send sends a real message to a real person: send only what the user asked to send, to whom they asked. Reading never marks a message read.",
  uses: { services: ["imessage"] },
  service: { network: [], person: true, platforms: ["mac"] },

  tools: {
    chats: {
      reads: true,
      description: "Recent chats, most recent first, to see what's new or find a chat to read: each { chat, id, with, last, from, text, unread }, chat its name, id for imessage_read, with the people in a group, and last (an ISO 8601 time), from and text its last message. Page back with skip.",
      input: {
        type: "object",
        properties: {
          unread: { type: "boolean", description: "only chats with unread messages" },
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the most recent, to page back: 30 for the second page of 30" },
        },
      },
      run: (ctx, input) => service(ctx, "chats", input),
    },
    read: {
      reads: true,
      description: `One chat's messages, newest last, without marking them read: each ${MESSAGE}. Page back with skip.`,
      input: {
        type: "object",
        required: ["chat"],
        properties: {
          chat: { type: "string", description: CHAT },
          since: SINCE,
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the newest, to read further back: 30 for the 30 before the latest 30" },
        },
      },
      run: (ctx, input) => service(ctx, "read", input),
    },
    search: {
      reads: true,
      description: `Messages containing some text, across every chat or in one, newest last: to find something said when you don't know which chat. Each is as imessage_read gives them, with chat, its chat's name. Page back with skip.`,
      input: {
        type: "object",
        required: ["query"],
        properties: {
          query: { type: "string", description: "text to look for, any case" },
          chat: { type: "string", description: `only in this chat: ${CHAT}` },
          since: SINCE,
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the newest matches, to page back" },
        },
      },
      run: (ctx, input) => service(ctx, "search", input),
    },
    send: {
      reads: false,
      confirm: true,
      description: "Send an iMessage from the person's account to a chat, a phone number or an email address. It sends a real message to a real person and can't be taken back: only what the person asked to send, to whom they asked. Says to whom it went.",
      input: {
        type: "object",
        required: ["to", "text"],
        properties: {
          to: { type: "string", description: "a phone number with its country code (+61 400 000 000), an email address, or a chat: its name or id as imessage_chats gives it" },
          text: { type: "string", description: "the message, exactly as the person approved it" },
        },
      },
      run: (ctx, input) => service(ctx, "send", input),
    },
  },
};
