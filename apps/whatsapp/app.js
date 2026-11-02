// WhatsApp: the person's WhatsApp accounts, each a phone linked to one of their devices as WhatsApp Desktop is linked.
// The whatsapp sidecar links the phone and keeps its messages, on each device whose owner installed it; this app is
// what people and agents use, and a connector.

/// Runs one of the whatsapp sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.whatsapp[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs whatsapp for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "WhatsApp isn't on any of your devices yet, so there's no phone to link: whoever owns a device installs it there, and the App Store's WhatsApp page shows where. Then whatsapp_sign_in links your phone.";

/// The caller's online devices that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.sidecars || []).includes("whatsapp")).map((s) => s.name);
}

// Each account is a Vault item too, so the person's Vault lists it: its number and the device it's linked on, plain,
// with no secret, since the link itself stays in the sidecar on that device. Linking finishes on the phone after
// sign_in returns, so listing the accounts makes the items of new ones, and lets go of those whose device, online,
// no longer has them.
const TYPE = "WhatsApp account";
const VAULT = {
  account: { title: TYPE, type: TYPE, domain: "whatsapp.com", accounts: true, fields: { phone: { label: "Number" }, device: { label: "Linked on" } } },
};

async function keep(ctx, listed, devices) {
  if (!ctx.user) return;
  const items = (await ctx.vault.items()).filter((i) => i.type === TYPE && i.scope === ctx.user.id);
  for (const a of listed) {
    const item = items.find((i) => i.fields.phone?.value === a.account);
    if (!item) await ctx.vault.create({ title: a.account, type: TYPE, domain: "whatsapp.com", hosts: [], same: "phone", fields: { phone: { value: a.account }, device: { value: a.device } } });
    // `server: null`: until 2026-11-02's rename has reached every account's item, which said server.
    else if (item.fields.device?.value !== a.device) ctx.vault.update(item.id, { fields: { device: { value: a.device }, server: null } });
  }
  for (const item of items) {
    if (devices.includes(item.fields.device?.value) && !listed.some((a) => a.account === item.fields.phone?.value)) ctx.vault.release(item.id);
  }
}

export default {
  name: "whatsapp",
  title: "WhatsApp",
  icon: "whatsapp-logo",
  color: "#25d366",
  domain: "whatsapp.com",
  description: "Your WhatsApp chats, for your agents: read, search and send",
  connector: true,
  needs: "Needs a device of yours to link your phone to",
  instructions: "WhatsApp is the person's WhatsApp accounts, each a phone linked to one of their devices, as WhatsApp Desktop is linked; calls find the device. whatsapp_accounts lists them; the others take account, the first by default. whatsapp_chats lists recent chats; whatsapp_read reads one, newest last; whatsapp_search finds messages. whatsapp_send sends a real message to a real person: send only what the user asked to send, to whom they asked. Reading never tells WhatsApp a message was read. whatsapp_move moves a link to another of their devices; whatsapp_sign_out unlinks one and deletes its messages, only when the user asks.",
  uses: { sidecars: ["whatsapp"] },
  vault: VAULT,

  ops: {
    accounts: {
      reads: true,
      description: "The WhatsApp accounts linked on any of your devices, this one's first: each { account, ok, device }, with why when ok is false.",
      input: {"properties": {}, "type": "object"},
      // A connector lists no accounts, rather than failing, where no device runs its sidecar; its settings say why.
      async run(ctx, input) {
        const devices = await running(ctx);
        if (!devices.length) return [];
        const listed = await sidecar(ctx, "accounts", input);
        await keep(ctx, listed, devices).catch(() => {}); // a call that only reads lists them all the same
        return listed;
      },
    },
    status: {
      reads: true,
      description: "Which accounts are linked and connected here, and how many messages each keeps; or on which of your devices WhatsApp is linked.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "status", input),
    },
    sign_in: {
      description: "Link a WhatsApp account here, or another one, as WhatsApp Desktop is linked: returns a QR code to scan on the phone (WhatsApp > Settings > Linked devices > Link a device) and, given phone, a code to type there instead (Link with phone number instead), for when the QR code is on the phone itself.",
      input: {"properties": {"phone": {"description": "the account's number with its country code, e.g. +61 400 000 000, for a code to type on the phone", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "sign_in", input),
    },
    chats: {
      reads: true,
      description: "Recent WhatsApp chats, most recent first: who, when, the last message, and how many are unread.",
      input: {"properties": {"account": {"description": "which account: its number, as whatsapp_accounts lists them; default the first linked here", "type": "string"}, "limit": {"description": "at most this many; default 30", "type": "integer"}, "skip": {"description": "skip this many of the most recent, to page back", "type": "integer"}, "unread": {"description": "only unread chats", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "chats", input),
    },
    read: {
      reads: true,
      description: "A WhatsApp chat's messages, newest last. Page back with skip. WhatsApp isn't told they were read: the sender sees no blue ticks, and the chat stays unread on the phone.",
      input: {"properties": {"account": {"description": "which account: its number, as whatsapp_accounts lists them; default the first linked here", "type": "string"}, "chat": {"description": "the chat's name, a part of it only one chat has, a phone number, or a JID", "type": "string"}, "limit": {"description": "this many; default 30", "type": "integer"}, "skip": {"description": "skip this many of the newest, to read further back", "type": "integer"}}, "required": ["chat"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "read", input),
    },
    search: {
      reads: true,
      description: "WhatsApp messages containing some text, across chats or in one, newest last.",
      input: {"properties": {"account": {"description": "which account: its number, as whatsapp_accounts lists them; default the first linked here", "type": "string"}, "chat": {"description": "only in this chat", "type": "string"}, "limit": {"description": "at most this many; default 30", "type": "integer"}, "query": {"description": "text to look for, case-insensitive", "type": "string"}, "skip": {"description": "skip this many of the newest matches, to page back", "type": "integer"}}, "required": ["query"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "search", input),
    },
    send: {
      confirm: true,
      description: "Send a WhatsApp message, as the linked account, to a chat, a contact or a phone number. This sends a real message to a real person.",
      input: {"properties": {"account": {"description": "which account: its number, as whatsapp_accounts lists them; default the first linked here", "type": "string"}, "chat": {"description": "the chat's name, a part of it only one chat has, a contact's exact name, a phone number with its country code, or a JID", "type": "string"}, "text": {"description": "the message", "type": "string"}}, "required": ["chat", "text"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "send", input),
    },
    move: {
      description: "Move a WhatsApp link, with its kept messages, from the device it's on to another of yours that runs WhatsApp, without linking again: the phone's Linked devices list doesn't change. It's deleted here once the other device has it.",
      input: {"properties": {"account": {"description": "which account: its number, as whatsapp_accounts lists them; default the first linked here", "type": "string"}, "to": {"description": "the device to move it to, by name", "type": "string"}}, "required": ["to"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "move", input),
    },
    sign_out: {
      confirm: true,
      description: "Unlink a WhatsApp account: removes this device from its linked devices and deletes the messages kept here. Destructive: linking again needs the phone, and the deleted messages don't come back. Only when the user asks to sign out.",
      input: {"properties": {"account": {"description": "the account's number; needed when several are linked here", "type": "string"}}, "type": "object"},
      async run(ctx, input) {
        const out = await sidecar(ctx, "sign_out", input);
        const devices = await running(ctx);
        await keep(ctx, await sidecar(ctx, "accounts", {}), devices);
        return out;
      },
    },
    _pairing: {
      reads: true,
      description: "The QR code and phone code while a phone is linking, for the settings screen.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => sidecar(ctx, "_pairing", input),
    },
    _running: {
      reads: true,
      description: "Which of your devices it runs on, for the screens to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ devices: await running(ctx), needs: NEEDS }),
    },
  },

  screens: {
    settings: {
      title: "WhatsApp",
      html: `
<style>
  .card { background: var(--card); border-radius: 12px; padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--accent); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--accent); background: none; border: 0; padding: 4px 0; }
  input { width: 100%; box-sizing: border-box; font: inherit; padding: 11px 14px; border-radius: 10px; border: 0.5px solid var(--line); background: var(--bg); color: var(--text); margin: 8px 0; }
  ol { padding-left: 20px; margin: 8px 0; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: #34c759; flex: none; margin-right: 12px; }
  .dot.warn { background: #ff9f0a; }
  #qr { display: block; width: 220px; height: 220px; margin: 10px auto; background: white; border-radius: 8px; image-rendering: pixelated; }
  .code { font: 600 24px ui-monospace, monospace; letter-spacing: 2px; text-align: center; margin: 8px 0; }
</style>
<div id="list"></div>
<div class="card" id="connect"></div>
<script>
// Each account is a phone linked to one of your devices, as WhatsApp Desktop is linked.
let shown = 0;
async function show() {
  const accounts = await one.call("accounts");
  shown = accounts.length;
  document.getElementById("list").innerHTML = accounts.length
    ? '<h2 class="section">Accounts</h2><div class="list">' + accounts.map((a, i) => \`<div class="row plain"><span class="dot\${a.ok ? "" : " warn"}"></span><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub">\${one.esc(a.ok ? \`Linked on \${a.device}\${i ? "" : " · used first"}\` : a.why || "Needs linking again")}</div></div><button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`).join("") + "</div>"
    : '<p class="muted">Link WhatsApp, and your agents can read your chats and, when you ask, send.</p>';
  for (const b of document.querySelectorAll("[data-out]")) b.onclick = async () => {
    await one.confirm("#connect", { title: \`Sign out of \${b.dataset.out}?\`, detail: "This device is unlinked from it, and the messages kept for it are deleted. Linking again needs the phone.",
      action: "Sign Out", danger: true, run: () => one.call("sign_out", { account: b.dataset.out }) });
    show();
  };
  const c = document.getElementById("connect");
  c.innerHTML = '<input id="phone" type="tel" placeholder="Its number, for a code to type instead (optional)"><button class="primary" id="start">Add account</button><div id="err" class="error small"></div>';
  document.getElementById("start").onclick = start;
}

async function start(e) {
  const phone = document.getElementById("phone").value.trim();
  e.target.disabled = true;
  e.target.textContent = "Asking WhatsApp…";
  try { await one.call("sign_in", phone ? { phone } : {}); }
  catch (err) { document.getElementById("err").textContent = err.message; e.target.disabled = false; e.target.textContent = "Add account"; return; }
  document.getElementById("connect").innerHTML = \`<ol><li>On the phone, open WhatsApp › Settings › Linked devices › Link a device.</li>
    <li>Scan this code\${phone ? ", or tap Link with phone number instead and type the code under it" : ""}.</li></ol>
    <img id="qr" alt=""><div class="code" id="code"></div><div class="small muted" id="note">The code changes every 20 seconds. WhatsApp then sends recent chats, which takes a few minutes.</div>\`;
  watch();
}

// Shows the current codes until the phone links, or the codes run out.
async function watch() {
  for (let i = 0; i < 90; i++) {
    const p = await one.call("_pairing").catch(() => ({}));
    if (!p.qr) {
      const accounts = await one.call("accounts").catch(() => []);
      if (accounts.length > shown) return show();
      document.getElementById("note").textContent = "The codes ran out before the phone linked.";
      document.getElementById("qr").hidden = true;
      return setTimeout(show, 3000);
    }
    document.getElementById("qr").src = p.qr;
    document.getElementById("code").textContent = p.code || "";
    await new Promise((r) => setTimeout(r, 2000));
  }
  show();
}

one.on("changed", show);
show().then(() => { if (one.params.add) document.getElementById("phone").focus(); });
</script>
<div class="card" id="needs" hidden></div>
<script>
one.call("_running").then(({ devices, needs }) => {
  if (devices.length) return;
  const n = document.getElementById("needs");
  n.textContent = needs;
  n.hidden = false;
  document.getElementById("connect").hidden = true; 
});
</script>`,
    },
  },
};
