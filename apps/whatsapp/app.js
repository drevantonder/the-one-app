// WhatsApp: the person's WhatsApp accounts, each a phone linked to one of their devices as WhatsApp Desktop is linked.
// The whatsapp sidecar links the phone and keeps its messages, on each device whose owner installed it; this app is
// what people and agents use, and a connector.

/// Runs one of the whatsapp sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed; when a chat isn't found, where to find it.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.whatsapp[op](input);
  } catch (err) {
    const why = String(err.message);
    if (why.includes("no device runs whatsapp for you")) throw new Error(NEEDS);
    if (why.startsWith("no chat called ")) throw new Error(`${why}: whatsapp_chats lists the chats; anyone else by their phone number with its country code`);
    if (why.includes("matches several chats")) throw new Error(`${why}. Give more of the name, or a phone number with its country code`);
    throw err;
  }
}

const NEEDS = "WhatsApp isn't on any of your devices yet. It runs on one that stays on, which your phone links to: that device's owner installs it there, and apps_devices whatsapp says where it runs. Then whatsapp_sign_in links your phone.";

const ACCOUNT = { type: "string", description: "which WhatsApp account: its number, as whatsapp_accounts lists them; default the first linked here" };
const CHAT = "the chat: its name as whatsapp_chats gives it, or a part of it only one chat has; or a phone number with its country code (+61 400 000 000), or a JID (61400000000@s.whatsapp.net)";
const LIMIT = { type: "integer", description: "at most this many; default 30" };
const LINES = "one a line, \"[3 Nov 2026 09:00] Sam: text\", times in the time zone of the device it's linked on, and You for the person";

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
    else if (item.fields.device?.value !== a.device) ctx.vault.update(item.id, { fields: { device: { value: a.device } } });
  }
  for (const item of items) {
    if (devices.includes(item.fields.device?.value) && !listed.some((a) => a.account === item.fields.phone?.value)) ctx.vault.release(item.id);
  }
}

export default {
  name: "whatsapp",
  title: "WhatsApp",
  tagline: "Your chats",
  icon: "whatsapp-logo",
  color: "#25d366",
  description: "Your WhatsApp chats, for your agents: read, search and send",
  connector: true,
  needs: "Needs a device of yours to link your phone to",
  instructions: "WhatsApp is the person's WhatsApp accounts, each a phone linked to one of their devices, as WhatsApp Desktop is linked; calls find the device. whatsapp_accounts lists them; the others take account, the first by default. whatsapp_chats lists recent chats; whatsapp_read reads one, newest last; whatsapp_search finds messages. Each list pages back with limit and skip. whatsapp_send sends a real message to a real person: send only what the user asked to send, to whom they asked. Reading never tells WhatsApp a message was read. whatsapp_move moves a link to another of their devices; whatsapp_sign_out unlinks one and deletes its messages, only when the user asks.",
  uses: { sidecars: ["whatsapp"] },
  vault: VAULT,

  ops: {
    accounts: {
      reads: true,
      description: "The WhatsApp accounts linked on any of your devices, this one's first: each { account, ok, device }, account its number and device where it's linked, with why when ok is false. Give account to the others to pick one.",
      input: { type: "object", properties: {} },
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
      description: "Whether WhatsApp is linked and connected on this device, as which number, and how many messages it keeps here; or on which of your devices it's linked instead. For why chats are missing; whatsapp_accounts lists every account.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => sidecar(ctx, "status", input),
    },
    sign_in: {
      reads: false,
      description: "For the person: link a WhatsApp account to this device, as WhatsApp Desktop is linked. Gives a QR code to scan on their phone (WhatsApp > Settings > Linked devices > Link a device) and, given phone, a code to type there instead (Link with phone number instead), for when the QR code shows on the phone itself. Chats arrive over the next few minutes.",
      input: { type: "object", properties: { phone: { type: "string", description: "the account's number with its country code, like +61 400 000 000, for a code to type on the phone" } } },
      run: (ctx, input) => sidecar(ctx, "sign_in", input),
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
      run: (ctx, input) => sidecar(ctx, "chats", input),
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
      run: (ctx, input) => sidecar(ctx, "read", input),
    },
    search: {
      reads: true,
      description: "WhatsApp messages containing some text, across every chat or in one, newest last: to find something said when you don't know which chat. One a line, \"[3 Nov 2026 09:00] Sam in Family: text\", the chat only when it isn't Sam's own and \"You to Sam\" for the person's, times in the time zone of the device it's linked on. Page back with skip.",
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
      run: (ctx, input) => sidecar(ctx, "search", input),
    },
    send: {
      reads: false,
      confirm: true,
      description: "Send a WhatsApp message, as the linked account, to a chat, a contact or a phone number. It sends a real message to a real person and can't be taken back: only what the person asked to send, to whom they asked. Says to whom it went.",
      input: {
        type: "object",
        required: ["chat", "text"],
        properties: {
          account: ACCOUNT,
          chat: { type: "string", description: `${CHAT}; or a contact's exact name, for someone with no chat yet` },
          text: { type: "string", description: "the message, exactly as the person approved it" },
        },
      },
      run: (ctx, input) => sidecar(ctx, "send", input),
    },
    move: {
      reads: false,
      description: "Move a WhatsApp link, with its kept messages, from the device it's on to another of yours that runs WhatsApp, without linking again: the phone's Linked devices list doesn't change. It's deleted from the first device once the other has it. Says where it went.",
      input: { type: "object", required: ["to"], properties: { account: ACCOUNT, to: { type: "string", description: "the device to move it to, by its name as whatsapp_status or the App Store's WhatsApp page shows" } } },
      run: (ctx, input) => sidecar(ctx, "move", input),
    },
    sign_out: {
      reads: false,
      confirm: true,
      description: "Unlink a WhatsApp account: removes this device from the phone's linked devices and deletes the messages kept here. It can't be undone: linking again needs the phone, and the deleted messages don't come back. Only when the person asks to sign out.",
      input: { type: "object", properties: { account: { type: "string", description: "the account's number, as whatsapp_accounts lists them; needed when several are linked here" } } },
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
      run: async (ctx) => ({ devices: await running(ctx) }),
    },
  },

  screens: {
    settings: {
      title: "WhatsApp",
      html: `
<style>
  #qr { display: block; width: 220px; height: 220px; margin: 10px auto; background: white; border-radius: 12px; image-rendering: pixelated; }
  .code { font: 600 24px ui-monospace, monospace; letter-spacing: 2px; text-align: center; margin: 8px 0; }
</style>
<div id="list"></div>
<div id="ask"></div>
<div id="connect" hidden><div class="list"><label class="row plain"><input class="field" id="phone" type="tel" placeholder="Its number, for a code to type instead (optional)" aria-label="Its number"></label></div><div class="actions"><button class="button primary" id="start">Add Account</button></div><div id="err"></div></div>
<div id="needs" hidden></div>
<script>
// Each account is a phone linked to one of your devices, as WhatsApp Desktop is linked.
const list = document.getElementById("list"), connect = document.getElementById("connect"), form = connect.innerHTML;
let accounts = [], asked = 0;
// Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
const leaving = new Set();
function draw(fetch) {
  return one.render(list, async () => {
    if (fetch) {
      const n = ++asked, got = await one.call("accounts");
      if (n === asked) accounts = got;
    }
    const all = accounts.filter((a) => !leaving.has(a.account));
    if (!all.length) return '<p class="footer">Link WhatsApp, and your agents can read your chats and, when you ask, send.</p>';
    const needs = all.filter((a) => !a.ok).map((a) => one.problem({ title: a.account + " needs linking again", text: one.plain(a.why || "Needs linking again").title.replace(/^Needs linking again\\.$/, "Link it from the phone again."), warn: true, action: { label: "Link Again", run: () => start() } })).join("");
    return needs + '<h2 class="section">Accounts</h2><div class="list">' + all.map((a, i) => \`<div class="row plain"><div class="main"><div class="title">\${one.esc(a.account)}</div>\${a.ok ? \`<div class="sub">Linked on \${one.esc(a.device)}\${i ? "" : " · used first"}</div>\` : \`<div class="sub warn">\${one.icon("warning-circle-fill")}Link again</div>\`}</div><button class="button plain" data-out="\${one.esc(a.account)}">Sign Out</button></div>\`).join("") + "</div>";
  }, { skeleton: 1 });
}
list.onclick = (e) => {
  const account = e.target.closest("[data-out]")?.dataset.out;
  if (!account) return;
  one.confirm("#ask", { title: \`Sign out of \${account}?\`, detail: "This device is unlinked from it, and the messages kept for it are deleted. Linking again needs the phone.",
    action: "Sign Out", danger: true, run: async () => {
      leaving.add(account);
      draw(false);
      try {
        await one.call("sign_out", { account });
        accounts = accounts.filter((a) => a.account !== account);
      } finally {
        leaving.delete(account);
        draw(true);
      }
    } });
};
function ready() {
  connect.innerHTML = form;
  document.getElementById("start").onclick = (e) => start(e.currentTarget);
}

async function start(b) {
  const phone = document.getElementById("phone")?.value.trim() || "";
  const back = b && one.busy(b, "Asking WhatsApp…");
  try { await one.call("sign_in", phone ? { phone } : {}); }
  catch (err) {
    back?.();
    document.getElementById("err").innerHTML = one.problem({ title: one.plain(err).title, action: { label: "Try Again", run: () => start() } });
    return;
  }
  connect.innerHTML = \`<h2 class="section">Link a Device</h2><div class="list"><div class="row plain"><div class="main">On the phone, open WhatsApp › Settings › Linked devices › Link a device.</div></div>
    <div class="row plain"><div class="main">Scan this code\${phone ? ", or tap Link with phone number instead and type the code under it" : ""}.</div></div></div>
    <img id="qr" alt=""><div class="code" id="code"></div><p class="footer" id="note">The code changes every 20 seconds. WhatsApp then sends recent chats, which takes a few minutes.</p>\`;
  watch();
}

// Shows the current codes until the phone links, or the codes run out.
async function watch() {
  const had = accounts.length;
  for (let i = 0; i < 90; i++) {
    const p = await one.call("_pairing").catch(() => ({}));
    if (!p.qr) {
      const now = await one.call("accounts").catch(() => []);
      if (now.length > had) return draw(true).then(ready);
      document.getElementById("note").textContent = "The codes ran out before the phone linked.";
      document.getElementById("qr").hidden = true;
      return setTimeout(() => draw(true).then(ready), 3000);
    }
    document.getElementById("qr").src = p.qr;
    document.getElementById("code").textContent = p.code || "";
    await new Promise((r) => setTimeout(r, 2000));
  }
  draw(true).then(ready);
}

document.getElementById("start").onclick = (e) => start(e.currentTarget);
one.on("changed", () => draw(true));
// The accounts and whether any device runs WhatsApp come together, so the form shows once, or not at all.
Promise.all([one.call("_running").catch(() => null), draw(true)]).then(([r]) => {
  if (r && !r.devices.length) {
    // None of their devices has it: what to do, on the App Store's WhatsApp page, which says where it can go.
    const n = document.getElementById("needs");
    n.innerHTML = one.problem({ title: "WhatsApp isn't on any of your devices yet.", text: "Add it to one in the App Store, then link your phone here.", warn: true,
      action: { label: "Open App Store", run: () => one.open("app", { app: "whatsapp" }, "apps") } });
    n.hidden = false;
    list.innerHTML = "";
    return;
  }
  connect.hidden = false;
  if (one.params.add) document.getElementById("phone").focus();
});
</script>`,
    },
  },
};
