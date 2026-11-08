// WhatsApp: the user's WhatsApp accounts, each a phone linked to one of their devices as WhatsApp Desktop is linked.
// The whatsapp service links the phone and keeps its messages, on each device whose owner installed it; this app is
// what users and agents use.

import settings from "./dist/settings.html" with { type: "text" };

async function service(ctx, fn, input = {}) {
  const { device, ...rest } = input;
  try {
    return await ctx.services.whatsapp[fn](rest, device ? { device } : undefined);
  } catch (err) {
    const why = String(err.message);
    if (why.startsWith("no chat called ")) throw new Error(`${why}: whatsapp_chats lists the chats; anyone else by their phone number with its country code`);
    if (why.includes("matches several chats")) throw new Error(`${why}. Give more of the name, or a phone number with its country code`);
    throw err;
  }
}

const ACCOUNT = { type: "string", description: "which WhatsApp account: its number, as whatsapp_accounts lists them; default the first linked here" };
const CHAT = "the chat: its name as whatsapp_chats gives it, or a part of it only one chat has; or a phone number with its country code (+61 400 000 000), or a JID (61400000000@s.whatsapp.net)";
const DEVICE = { type: "string", description: "which of your devices that run WhatsApp to link it on, by name, as apps_devices whatsapp says where it runs; default the nearest. One account may be linked on several" };
const LIMIT = { type: "integer", description: "at most this many; default 30" };
const LINES = "one a line, \"[3 Nov 2026 09:00] Sam: text\", times in the time zone of the device it's linked on, and You for the user";

async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.services || []).includes("whatsapp")).map((s) => s.name);
}

// Each account is a Vault item too, so the user's Vault lists it: its number and the device it's linked on, plain,
// with no secret, since the link itself stays in the service on that device. Linking finishes on the phone after
// sign_in returns, so listing the accounts makes the items of new ones, and lets go of those whose every device
// answered without them. A device that didn't answer leaves its accounts as they were.
const TYPE = "WhatsApp account";
const VAULT = {
  account: { title: TYPE, type: TYPE, domain: "whatsapp.com", accounts: true, fields: { phone: { label: "Number" }, device: { label: "Linked on" } } },
};

// An account linked on several devices is one item, linked on each of them. `answered` is the devices whose answer
// was complete; an item is let go only if every device it says it's linked on is one of them.
async function keep(ctx, accounts, answered) {
  if (!ctx.user) return;
  const items = (await ctx.vault.items()).filter((i) => i.type === TYPE && i.scope === ctx.user.id);
  const on = new Map();
  for (const a of accounts) on.set(a.account, [...(on.get(a.account) || []), a.device].sort());
  const listed = [...on].map(([account, where]) => ({ account, device: where.join(", ") }));
  for (const a of listed) {
    const item = items.find((i) => i.fields.phone?.value === a.account);
    if (!item) await ctx.vault.create({ title: a.account, type: TYPE, domain: "whatsapp.com", hosts: [], same: "phone", fields: { phone: { value: a.account }, device: { value: a.device } } });
    else if (item.fields.device?.value !== a.device) ctx.vault.update(item.id, { fields: { device: { value: a.device } } });
  }
  for (const item of items) {
    if (String(item.fields.device?.value || "").split(", ").every((d) => answered.includes(d)) && !listed.some((a) => a.account === item.fields.phone?.value)) ctx.vault.release(item.id);
  }
}

async function listAndKeep(ctx, input) {
  const { accounts, answered } = await service(ctx, "_accounts", input);
  await keep(ctx, accounts, answered).catch(() => {}); // a call that only reads lists them all the same
  return accounts;
}

export default {
  name: "whatsapp",
  title: "WhatsApp",
  tagline: "Your chats",
  icon: "whatsapp-logo",
  color: "#25d366",
  domain: "whatsapp.com",
  description: "Your WhatsApp chats, for your agents: read, search and send",
  needs: "Needs a device of yours to link your phone to",
  instructions: "WhatsApp is the user's WhatsApp accounts, each a phone linked to one of their devices, as WhatsApp Desktop is linked; calls find the device. whatsapp_accounts lists them; the others take account, the first by default. whatsapp_chats lists recent chats; whatsapp_read reads one, newest last; whatsapp_search finds messages. Each list pages back with limit and skip. whatsapp_send sends a real message to a real person: send only what the user asked to send, to whom they asked. Reading never tells WhatsApp a message was read. whatsapp_move moves a link to another of their devices; whatsapp_sign_out unlinks one and deletes its messages, only when the user asks.",
  uses: { services: ["whatsapp"] },
  service: { network: ["web.whatsapp.com", "*.whatsapp.net"], user: false, platforms: ["linux", "mac"] },
  vault: VAULT,

  tools: {
    accounts: {
      reads: true,
      accounts: true,
      description: "The WhatsApp accounts linked on any of your devices, this one's first: each { account, ok, device }, account its number and device where it's linked, with why when ok is false. Give account to the others to pick one.",
      input: { type: "object", properties: {} },
      async run(ctx, input) {
        if (!(await running(ctx)).length) return [];
        return listAndKeep(ctx, input);
      },
    },
    status: {
      reads: true,
      description: "Whether WhatsApp is linked and connected on this device, as which number, and how many messages it keeps here; or on which of your devices it's linked instead. For why chats are missing; whatsapp_accounts lists every account.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "status", input),
    },
    sign_in: {
      reads: false,
      description: "For the user: link a WhatsApp account to this device, as WhatsApp Desktop is linked. Gives a QR code to scan on their phone (WhatsApp > Settings > Linked devices > Link a device) and, given phone, a code to type there instead (Link with phone number instead), for when the QR code shows on the phone itself. Chats arrive over the next few minutes.",
      input: { type: "object", properties: { phone: { type: "string", description: "the account's number with its country code, like +61 400 000 000, for a code to type on the phone" }, device: DEVICE } },
      run: (ctx, input) => service(ctx, "sign_in", input),
    },
    chats: {
      reads: true,
      description: "Recent WhatsApp chats, most recent first, to see what's new or find a chat to read: one a line, \"Sam (2 unread), 09:00: You: last message\", the time in the time zone of the device it's linked on. Page back with skip.",
      input: {
        type: "object",
        properties: {
          account: ACCOUNT,
          unread: { type: "boolean", description: "only chats with unread messages" },
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the most recent, to page back: 30 for the second page of 30" },
        },
      },
      run: (ctx, input) => service(ctx, "chats", input),
    },
    read: {
      reads: true,
      description: `One WhatsApp chat's messages, newest last, ${LINES}. WhatsApp isn't told they were read: the sender sees no blue ticks, and the chat stays unread on the phone. Page back with skip.`,
      input: {
        type: "object",
        required: ["chat"],
        properties: {
          account: ACCOUNT,
          chat: { type: "string", description: CHAT },
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the newest, to read further back: 30 for the 30 before the latest 30" },
        },
      },
      run: (ctx, input) => service(ctx, "read", input),
    },
    search: {
      reads: true,
      description: "WhatsApp messages containing some text, across every chat or in one, newest last: to find something said when you don't know which chat. One a line, \"[3 Nov 2026 09:00] Sam in Family: text\", the chat only when it isn't Sam's own and \"You to Sam\" for the user's, times in the time zone of the device it's linked on. Page back with skip.",
      input: {
        type: "object",
        required: ["query"],
        properties: {
          account: ACCOUNT,
          query: { type: "string", description: "text to look for, any case" },
          chat: { type: "string", description: `only in this chat: ${CHAT}` },
          limit: LIMIT,
          skip: { type: "integer", description: "leave out this many of the newest matches, to page back" },
        },
      },
      run: (ctx, input) => service(ctx, "search", input),
    },
    send: {
      reads: false,
      confirm: true,
      description: "Send a WhatsApp message, as the linked account, to a chat, a contact or a phone number. It sends a real message to a real person and can't be taken back: only what the user asked to send, to whom they asked. Says to whom it went.",
      input: {
        type: "object",
        required: ["chat", "text"],
        properties: {
          account: ACCOUNT,
          chat: { type: "string", description: `${CHAT}; or a contact's exact name, for someone with no chat yet` },
          text: { type: "string", description: "the message, exactly as the user approved it" },
        },
      },
      run: (ctx, input) => service(ctx, "send", input),
    },
    move: {
      reads: false,
      description: "Move a WhatsApp link, with its kept messages, from the device it's on to another of yours that runs WhatsApp, without linking again: the phone's Linked devices list doesn't change. It's deleted from the first device once the other has it. Says where it went.",
      input: { type: "object", required: ["to"], properties: { account: ACCOUNT, to: { type: "string", description: "the device to move it to, by its name as whatsapp_status or the App Store's WhatsApp page shows" } } },
      run: (ctx, input) => service(ctx, "move", input),
    },
    sign_out: {
      reads: false,
      confirm: true,
      description: "Unlink a WhatsApp account: removes this device from the phone's linked devices and deletes the messages kept here. It can't be undone: linking again needs the phone, and the deleted messages don't come back. Only when the user asks to sign out.",
      input: { type: "object", properties: { account: { type: "string", description: "the account's number, as whatsapp_accounts lists them; needed when several are linked here" }, device: { type: "string", description: "the device it's linked on, as whatsapp_accounts lists it: for an account linked on several" } } },
      async run(ctx, input) {
        const out = await service(ctx, "sign_out", input);
        await listAndKeep(ctx, {});
        return out;
      },
    },
    _pairing: {
      reads: true,
      description: "The QR code and phone code while a phone is linking, for the settings screen.",
      input: { type: "object", properties: { device: DEVICE } },
      run: (ctx, input) => service(ctx, "_pairing", input),
    },
    _running: {
      reads: true,
      description: "Which of your devices it runs on, for the screens to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ devices: await running(ctx) }),
    },
  },

  screens: {
    settings: { title: "WhatsApp", html: settings },
  },
};
