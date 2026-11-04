// Messenger: the person's Facebook Messenger, read through messenger.com in an agent device's browser. The messenger service
// drives the browser, on each device whose owner installed it; this app is what people and agents use, and a
// connector: each account is a browser profile signed in to Messenger.

/// Runs one of the messenger service's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed; when a chat isn't found, where to find it.
async function service(ctx, op, input) {
  try {
    return await ctx.services.messenger[op](input);
  } catch (err) {
    const why = String(err.message);
    if (why.includes("no device runs messenger for you")) throw new Error(NEEDS);
    if (why.includes("matches several chats")) throw new Error(`${why}. Give more of the name`);
    if (why.startsWith("no chat or person called")) throw new Error(`${why}: messenger_chats lists recent chats; anyone else by the name Messenger shows for them`);
    throw err;
  }
}

const NEEDS = "Messenger needs Agent Devices on one of your devices. Add it in the App Store: whoever owns a device installs it there, and the App Store's Agent Devices page shows where. Then messenger_sign_in signs in.";

/// The caller's online devices that run the service, by name.
async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.services || []).includes("messenger")).map((s) => s.name);
}

// Each account is a Vault item too, so the person's Vault lists it: the browser profile and the devices it's on, plain,
// with no secret, since the sign-in itself stays in the profile. Signing in finishes in the browser after sign_in
// returns, so listing the accounts asks each device that runs the service, makes the items of new ones, and lets go
// of those whose every device answered without them. A device that didn't answer leaves its accounts as they were.
const TYPE = "Messenger account";
const VAULT = {
  account: { title: TYPE, type: TYPE, domain: "messenger.com", accounts: true, fields: { profile: { label: "Browser profile" }, device: { label: "On" } } },
};

/// `answers` is each device that answered, by name, with its accounts. A profile on several devices is one item, on
/// each of them; one kept before items said where stays until a device that answered has it.
async function keep(ctx, answers) {
  if (!ctx.user) return;
  const items = (await ctx.vault.items()).filter((i) => i.type === TYPE && i.scope === ctx.user.id);
  const on = new Map();
  for (const [device, listed] of answers) for (const a of listed) on.set(a.account, [...(on.get(a.account) || []), device]);
  for (const [account, where] of on) {
    const device = where.sort().join(", ");
    const item = items.find((i) => i.fields.profile?.value === account);
    if (!item) await ctx.vault.create({ title: account, type: TYPE, domain: "messenger.com", hosts: [], same: "profile", fields: { profile: { value: account }, device: { value: device } } });
    else if (item.fields.device?.value !== device) ctx.vault.update(item.id, { fields: { device: { value: device } } });
  }
  const answered = answers.map(([device]) => device);
  for (const item of items) {
    const was = String(item.fields.device?.value || "").split(", ").filter(Boolean);
    if (was.length && was.every((d) => answered.includes(d)) && !on.has(item.fields.profile?.value)) ctx.vault.release(item.id);
  }
}

/// Each of `devices`' accounts, from those that answered.
async function answers(ctx, devices) {
  const got = await Promise.all(devices.map((device) => ctx.services.messenger.accounts({}, { device }).then((listed) => Array.isArray(listed) ? [device, listed] : null, () => null)));
  return got.filter(Boolean);
}

const ACCOUNT = { type: "string", description: "which account: the browser profile's name, as messenger_accounts lists them; default the first" };
const CHAT = "the chat: its name as messenger_chats gives it, or a part of it only one recent chat has";

export default {
  name: "messenger",
  title: "Messenger",
  tagline: "Your chats",
  icon: "messenger-logo",
  color: "#0866ff",
  description: "Your Messenger chats, for your agents: read, search and send",
  connector: true,
  needs: "Needs Agent Devices on one of your devices",
  instructions: "Messenger is the person's Facebook Messenger, read through messenger.com in an agent device's browser. Each account is a browser profile signed in to it (Personal, Work), as messenger_accounts lists them; the others take account, the first by default. messenger_chats lists recent chats, messenger_read reads one, messenger_search finds messages. Each takes some seconds: a browser does the reading. If Messenger isn't signed in, messenger_sign_in gives the person a link to sign in; never sign in for them. messenger_send sends a real message to a real person: only when the person asked, with the exact text they approved.",
  uses: { services: ["messenger"] },
  vault: VAULT,

  ops: {
    accounts: {
      reads: true,
      description: "The browser profiles Messenger reads, the default first: each { account, ok }, account the profile's name, with why when ok is false and it needs signing in again. Give account to the others to pick one.",
      input: { type: "object", properties: {} },
      // A connector lists no accounts, rather than failing, where no device runs its service; its settings say why.
      async run(ctx, input) {
        const devices = await running(ctx);
        if (!devices.length) return [];
        const listed = await service(ctx, "accounts", input);
        await answers(ctx, devices).then((a) => keep(ctx, a)).catch(() => {}); // a call that only reads lists them all the same
        return listed;
      },
    },
    chats: {
      reads: true,
      description: "The recent Messenger chats messenger.com shows, newest first, to see what's new or find a chat to read: one a line, \"Sam (unread), <time>: last message\", the time as Messenger shows it. Older chats aren't reached.",
      input: { type: "object", properties: { account: ACCOUNT, unread: { type: "boolean", description: "only chats with unread messages" } } },
      run: (ctx, input) => service(ctx, "chats", input),
    },
    read: {
      reads: false,
      description: "A recent chat's latest messages, as messenger.com shows them when it opens, newest last: one a line, \"[time] Sam: text\", the time as Messenger shows it. Opening a chat marks it read, so one that was unread is marked unread again afterwards, but the sender may see it as seen.",
      input: { type: "object", required: ["chat"], properties: { account: ACCOUNT, chat: { type: "string", description: CHAT } } },
      run: (ctx, input) => service(ctx, "read", input),
    },
    search: {
      reads: true,
      description: "Messages containing some words, across every chat, as Messenger's search finds them: to find something said when you don't know which chat. One a line: the chat and the message when it has one match (\"Sam, <time>: text\"), or how many (\"Family: 3 matching messages\"), for messenger_read.",
      input: { type: "object", required: ["query"], properties: { account: ACCOUNT, query: { type: "string", description: "words to find, as you'd type them in Messenger's search" } } },
      run: (ctx, input) => service(ctx, "search", input),
    },
    send: {
      reads: false,
      confirm: true,
      description: "Send a Messenger message, as the person, to a recent chat or to someone by their exact name. It sends a real message to a real person and can't be taken back: only what the person asked to send, to whom they asked. Says to whom it went.",
      input: {
        type: "object",
        required: ["chat", "text"],
        properties: {
          account: ACCOUNT,
          chat: { type: "string", description: `${CHAT}; or anyone else by their exact name, as Messenger shows it` },
          text: { type: "string", description: "the message, exactly as the person approved it" },
        },
      },
      run: (ctx, input) => service(ctx, "send", input),
    },
    sign_in: {
      reads: false,
      description: "For the person: links Messenger to a browser profile (made if new) and gives a link to messenger.com in it, for them to sign in there themselves; never sign in for them. Signs the default profile in again when none is given.",
      input: { type: "object", properties: { profile: { type: "string", description: "the browser profile's name, like Personal or Work; agent-devices_profiles lists them" } } },
      run: (ctx, input) => service(ctx, "sign_in", input),
    },
    sign_out: {
      reads: false,
      description: "Stop reading a browser profile's Messenger. The profile, and its sign-ins to messenger.com and other sites, stay, so messenger_sign_in links it again.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the browser profile's name, as messenger_accounts lists them" } } },
      async run(ctx, input) {
        const out = await service(ctx, "sign_out", input);
        await keep(ctx, await answers(ctx, await running(ctx)));
        return out;
      },
    },
    _profiles: {
      reads: true,
      description: "The browser profiles, for the settings screen to pick one.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "_profiles", input),
    },
    _save: {
      reads: false,
      description: "The settings screen's Done: saves the browser profile signed in.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => service(ctx, "_save", input),
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
      title: "Messenger",
      html: `
<div id="list"></div>
<div id="connect"></div>
<div id="needs"></div>
<script>
// Messenger's accounts are browser profiles (Vault items of type Browser profile): adding one picks a profile, or makes one, and opens
// messenger.com in it to sign in.
const list = document.getElementById("list"), connect = document.getElementById("connect");
let accounts = [], asked = 0, picking = false, nowhere = false;
// Asked for as the screen opens, so Add Account has them at once.
const profiles = () => one.call("_profiles").then((r) => (r || []).map((p) => p.profile), () => []);
let known = profiles();
// Signed out here and maybe not yet on the device, so a redraw meanwhile doesn't bring them back.
const leaving = new Set(), failed = {};
function draw(fetch) {
  return one.render(list, async () => {
    if (fetch) {
      const n = ++asked, got = await one.call("accounts");
      if (n === asked) accounts = got;
    }
    const all = accounts.filter((a) => !leaving.has(a.account));
    // With nowhere to sign in, what's missing shows instead (below), with the way to add it.
    if (!all.length) return nowhere || picking ? "" : one.empty("Link Messenger to a browser profile signed in to messenger.com, and your agents can read your chats and, when you ask, send.", { label: "Add Account", run: () => pick() });
    const needs = all.filter((a) => !a.ok).map((a) => one.problem({ title: a.account + " needs signing in", text: "Sign it out, then add it again.", warn: true })).join("");
    return needs + '<h2 class="section">Browser profiles</h2><div class="list">' + all.map((a, i) => {
      const line = failed[a.account] || (a.ok ? (i ? "" : "Used first") : "Sign in again");
      const bad = !a.ok || failed[a.account];
      const sub = line ? \`<div class="sub\${bad ? " warn" : ""}">\${bad ? one.icon("warning-circle-fill") : ""}\${one.esc(line)}</div>\` : "";
      return \`<div class="row plain"><div class="main"><div class="title">\${one.esc(a.account)}</div>\${sub}</div><button class="button plain" data-out="\${one.esc(a.account)}">Sign Out</button></div>\`;
    }).join("") + (nowhere || picking ? "" : '<button class="row plain add" id="add"><div class="main"><div class="title">Add Account</div></div></button>') + "</div>";
  }, { skeleton: 1 });
}
list.onclick = async (e) => {
  const add = e.target.closest("#add");
  if (add) return pick(add);
  const account = e.target.closest("[data-out]")?.dataset.out;
  if (!account) return;
  leaving.add(account);
  delete failed[account];
  draw(false);
  try {
    await one.call("sign_out", { account });
    accounts = accounts.filter((a) => a.account !== account);
  } catch (err) {
    failed[account] = one.plain(err).title;
  }
  leaving.delete(account);
  draw(!failed[account]);
};
const stop = () => { picking = false; connect.innerHTML = ""; };

// Picks a profile not linked yet, or names a new one.
async function pick(b) {
  const back = b && one.busy(b);
  const [all] = await Promise.all([known, loaded]);
  known = profiles();
  back?.();
  picking = true;
  draw(false);
  const free = [...new Set(all)].filter((p) => !accounts.some((a) => a.account === p));
  connect.innerHTML = \`<h2 class="section">Browser profile</h2><div class="list">
      <label class="row plain"><div class="main">Profile</div><select class="field" id="profile" aria-label="Browser profile">\${free.map((p) => \`<option>\${one.esc(p)}</option>\`).join("")}<option value="">New profile…</option></select></label>
      <label class="row plain" id="named" \${free.length ? 'style="display:none"' : ""}><input class="field" id="name" placeholder="Its name, like Personal or Work" aria-label="Its name" autocapitalize="words" autocorrect="off"></label></div>
    <p class="footer">Each has its own sign-ins, like Personal or Work.</p>
    <div class="actions"><button class="button primary" id="go">Sign In to Messenger</button></div><div id="err"></div>\`;
  const select = document.getElementById("profile"), name = document.getElementById("name"), named = document.getElementById("named");
  select.onchange = () => (named.style.display = select.value ? "none" : "");
  document.getElementById("go").onclick = async (e) => {
    const profile = select.value || name.value.trim();
    if (!profile) return name.focus();
    const back = one.busy(e.currentTarget, "Opening a device…");
    try { signingIn(await one.call("sign_in", { profile })); }
    catch (err) { back(); document.getElementById("err").innerHTML = one.problem({ title: one.plain(err).title, action: { label: "Try Again", run: () => document.getElementById("go").click() } }); }
  };
}

function signingIn(r) {
  connect.innerHTML = \`<h2 class="section">Sign In</h2><div class="list">
      <a class="row" href="\${one.esc(r.url)}"><div class="main"><div class="title">Open the device</div><div class="sub">Its browser is on messenger.com, in the \${one.esc(r.profile)} profile.</div></div><span class="chev">›</span></a>
      <div class="row plain"><div class="main">Sign in there, and tick Keep me signed in.</div></div></div>
    <p class="footer">Come back and press Done.</p>
    <div class="actions"><button class="button primary" id="done">Done</button></div><div id="err"></div>\`;
  document.getElementById("done").onclick = async (e) => {
    const back = one.busy(e.currentTarget, "Saving…");
    try { await one.call("_save", { profile: r.profile }); }
    catch (err) { back(); document.getElementById("err").innerHTML = one.problem({ title: one.plain(err).title, action: { label: "Try Again", run: () => document.getElementById("done").click() } }); return; }
    stop();
    await draw(true);
  };
}

one.on("changed", () => draw(true));
const loaded = draw(true);
if (one.params.add) loaded.then(() => pick());
one.call("_running").then(({ devices }) => {
  if (devices.length) return;
  nowhere = true;
  draw(false);
  document.getElementById("needs").innerHTML = one.problem({ title: "Messenger needs Agent Devices.", text: "Add Agent Devices to one of your devices in the App Store, then sign in here.", warn: true,
    action: { label: "Open App Store", run: () => one.open("app", { app: "agent-devices" }, "apps") } });
}, () => {});
</script>`,
    },
  },
};
