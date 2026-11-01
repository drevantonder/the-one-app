// iMessage: Messages on the person's Mac. The imessage driver lives in the Mac app, which macOS lets reach Messages;
// this app is what people and agents use, wherever they are.

/// Runs one of the imessage driver's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function driver(ctx, op, input) {
  try {
    return await ctx.drivers.imessage[op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs imessage for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "iMessage runs on a Mac with The One App open, and none of yours is online with it.";

export default {
  name: "imessage",
  title: "iMessage",
  icon: "message.fill",
  color: "#34c759",
  description: "Messages on your Mac, for your apps and agents",
  instructions: "iMessage is the person's Messages on their Mac, iMessage and SMS. imessage_chats lists recent chats; imessage_read reads one, newest last; imessage_search finds messages. imessage_send sends a real message to a real person: send only what the user asked to send, to whom they asked. Reading never marks a message read.",
  uses: { drivers: ["imessage"] },

  ops: {
    chats: {
      description: "Recent chats, most recent first: who, when, the last message, and how many are unread.",
      input: {"properties": {"limit": {"description": "at most this many; default 30", "type": "integer"}, "skip": {"description": "skip this many of the most recent, to page back", "type": "integer"}, "unread": {"description": "only chats with unread messages", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => driver(ctx, "chats", input),
    },
    read: {
      description: "A chat's messages, newest last. Page back with skip.",
      input: {"properties": {"chat": {"description": "the chat's name, a part of it only one chat has, a phone number or email address, or an id chats gives", "type": "string"}, "limit": {"description": "this many; default 30", "type": "integer"}, "since": {"description": "only messages after this: an ISO 8601 date or date-time", "type": "string"}, "skip": {"description": "skip this many of the newest, to read further back", "type": "integer"}}, "required": ["chat"], "type": "object"},
      run: (ctx, input) => driver(ctx, "read", input),
    },
    search: {
      description: "Messages containing some text, across chats or in one, newest last.",
      input: {"properties": {"chat": {"description": "only in this chat", "type": "string"}, "limit": {"description": "at most this many; default 30", "type": "integer"}, "query": {"description": "text to look for, case-insensitive", "type": "string"}, "since": {"description": "only messages after this: an ISO 8601 date or date-time", "type": "string"}, "skip": {"description": "skip this many of the newest matches, to page back", "type": "integer"}}, "required": ["query"], "type": "object"},
      run: (ctx, input) => driver(ctx, "search", input),
    },
    send: {
      description: "Send an iMessage from the person's account, to a phone number, an email address or a chat. This sends a real message to a real person.",
      input: {"properties": {"text": {"description": "the message", "type": "string"}, "to": {"description": "a phone number with its country code, an email address, or a chat as chats names it", "type": "string"}}, "required": ["to", "text"], "type": "object"},
      run: (ctx, input) => driver(ctx, "send", input),
    },
  },
};
