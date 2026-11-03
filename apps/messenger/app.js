// Messenger: the person's Facebook Messenger, read through messenger.com in an agent device's browser. The messenger sidecar
// drives the browser, on each device whose owner installed it; this app is what people and agents use, and a
// connector: each account is a browser profile signed in to Messenger.

/// Runs one of the messenger sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed; when a chat isn't found, where to find it.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.messenger[op](input);
  } catch (err) {
    const why = String(err.message);
    if (why.includes("no device runs messenger for you")) throw new Error(NEEDS);
    if (why.includes("matches several chats")) throw new Error(`${why}. Give more of the name`);
    if (why.startsWith("no chat or person called")) throw new Error(`${why}: messenger_chats lists recent chats; anyone else by the name Messenger shows for them`);
    throw err;
  }
}

const NEEDS = "Messenger needs Agent Devices on one of your devices. Add it in the App Store: whoever owns a device installs it there, and the App Store's Agent Devices page shows where. Then messenger_sign_in signs in.";

/// The caller's online devices that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.sidecars || []).includes("messenger")).map((s) => s.name);
}

// Each account is a Vault item too, so the person's Vault lists it: the browser profile, plain, with no secret, since
// the sign-in itself stays in the profile. Signing in finishes in the browser after sign_in returns, so listing the
// accounts makes the items of new ones, and lets go of those no longer linked.
const TYPE = "Messenger account";
const VAULT = {
  account: { title: TYPE, type: TYPE, domain: "messenger.com", accounts: true, fields: { profile: { label: "Browser profile" } } },
};

async function keep(ctx, listed) {
  if (!ctx.user) return;
  const items = (await ctx.vault.items()).filter((i) => i.type === TYPE && i.scope === ctx.user.id);
  for (const a of listed) {
    if (!items.some((i) => i.fields.profile?.value === a.account)) await ctx.vault.create({ title: a.account, type: TYPE, domain: "messenger.com", hosts: [], same: "profile", fields: { profile: { value: a.account } } });
  }
  for (const item of items) {
    if (!listed.some((a) => a.account === item.fields.profile?.value)) ctx.vault.release(item.id);
  }
}

const ACCOUNT = { type: "string", description: "which account: the browser profile's name, as messenger_accounts lists them; default the first" };
const CHAT = "the chat: its name as messenger_chats gives it, or a part of it only one recent chat has";

export default {
  name: "messenger",
  title: "Messenger",
  icon: "messenger-logo",
  color: "#0866ff",
  description: "Your Messenger chats, for your agents: read, search and send",
  connector: true,
  needs: "Needs Agent Devices on one of your devices",
  instructions: "Messenger is the person's Facebook Messenger, read through messenger.com in an agent device's browser. Each account is a browser profile signed in to it (Personal, Work), as messenger_accounts lists them; the others take account, the first by default. messenger_chats lists recent chats, messenger_read reads one, messenger_search finds messages. Each takes some seconds: a browser does the reading. If Messenger isn't signed in, messenger_sign_in gives the person a link to sign in; never sign in for them. messenger_send sends a real message to a real person: only when the person asked, with the exact text they approved.",
  uses: { sidecars: ["messenger"] },
  vault: VAULT,

  ops: {
    accounts: {
      reads: true,
      description: "The browser profiles Messenger reads, the default first: each { account, ok }, account the profile's name, with why when ok is false and it needs signing in again. Give account to the others to pick one.",
      input: { type: "object", properties: {} },
      // A connector lists no accounts, rather than failing, where no device runs its sidecar; its settings say why.
      async run(ctx, input) {
        if (!(await running(ctx)).length) return [];
        const listed = await sidecar(ctx, "accounts", input);
        await keep(ctx, listed).catch(() => {}); // a call that only reads lists them all the same
        return listed;
      },
    },
    chats: {
      reads: true,
      description: "The recent Messenger chats messenger.com shows, newest first, to see what's new or find a chat to read: one a line, \"Sam (unread), <time>: last message\", the time as Messenger shows it. Older chats aren't reached.",
      input: { type: "object", properties: { account: ACCOUNT, unread: { type: "boolean", description: "only chats with unread messages" } } },
      run: (ctx, input) => sidecar(ctx, "chats", input),
    },
    read: {
      reads: false,
      description: "A recent chat's latest messages, as messenger.com shows them when it opens, newest last: one a line, \"[time] Sam: text\", the time as Messenger shows it. Opening a chat marks it read, so one that was unread is marked unread again afterwards, but the sender may see it as seen.",
      input: { type: "object", required: ["chat"], properties: { account: ACCOUNT, chat: { type: "string", description: CHAT } } },
      run: (ctx, input) => sidecar(ctx, "read", input),
    },
    search: {
      reads: true,
      description: "Messages containing some words, across every chat, as Messenger's search finds them: to find something said when you don't know which chat. One a line: the chat and the message when it has one match (\"Sam, <time>: text\"), or how many (\"Family: 3 matching messages\"), for messenger_read.",
      input: { type: "object", required: ["query"], properties: { account: ACCOUNT, query: { type: "string", description: "words to find, as you'd type them in Messenger's search" } } },
      run: (ctx, input) => sidecar(ctx, "search", input),
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
      run: (ctx, input) => sidecar(ctx, "send", input),
    },
    sign_in: {
      reads: false,
      description: "For the person: links Messenger to a browser profile (made if new) and gives a link to messenger.com in it, for them to sign in there themselves; never sign in for them. Signs the default profile in again when none is given.",
      input: { type: "object", properties: { profile: { type: "string", description: "the browser profile's name, like Personal or Work; agent-devices_profiles lists them" } } },
      run: (ctx, input) => sidecar(ctx, "sign_in", input),
    },
    sign_out: {
      reads: false,
      description: "Stop reading a browser profile's Messenger. The profile, and its sign-ins to messenger.com and other sites, stay, so messenger_sign_in links it again.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the browser profile's name, as messenger_accounts lists them" } } },
      async run(ctx, input) {
        const out = await sidecar(ctx, "sign_out", input);
        await keep(ctx, await sidecar(ctx, "accounts", {}));
        return out;
      },
    },
    _profiles: {
      reads: true,
      description: "The browser profiles, for the settings screen to pick one.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => sidecar(ctx, "_profiles", input),
    },
    _save: {
      reads: false,
      description: "The settings screen's Done: saves the browser profile signed in.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => sidecar(ctx, "_save", input),
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
<style>
  .card { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 12px 0; }
  button.primary { font: inherit; font-weight: 600; color: white; background: var(--tint); border: 0; border-radius: 10px; padding: 11px 16px; }
  button.plain { font: inherit; color: var(--tint); background: none; border: 0; padding: 4px 0; }
  button:disabled { opacity: 0.6; }
  button .spinner { display: inline-block; vertical-align: -2px; margin-right: 6px; }
  input, select { width: 100%; box-sizing: border-box; font: inherit; padding: 11px 14px; border-radius: 10px; border: 0.5px solid var(--line); background: var(--bg); color: var(--text); margin: 8px 0; }
  ol { padding-left: 20px; margin: 8px 0; }
  .sub.warn { color: var(--warn); }
</style>
<div id="list"></div>
<div class="card" id="connect"><button class="primary" id="start">Add account</button><div id="err" class="error small"></div></div>
<div id="needs" hidden></div>
<script>
// Messenger's accounts are browser profiles (Vault items of type Browser profile): adding one picks a profile, or makes one, and opens
// messenger.com in it to sign in.
const list = document.getElementById("list"), connect = document.getElementById("connect"), form = connect.innerHTML;
let accounts = [], asked = 0;
// Asked for as the screen opens, so Add account has them at once.
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
    return all.length
      ? '<h2 class="section">Browser profiles</h2><div class="list">' + all.map((a, i) => {
          const why = failed[a.account] || (a.ok ? (i ? "" : "Used first") : one.plain(a.why || "Needs signing in again").title);
          return \`<div class="row plain"><div class="main"><div class="title">\${one.esc(a.account)}</div><div class="sub\${a.ok && !failed[a.account] ? "" : " warn"}">\${one.esc(why)}</div></div><button class="plain" data-out="\${one.esc(a.account)}">Sign out</button></div>\`;
        }).join("") + "</div>"
      : '<p class="muted">Link Messenger to a browser profile signed in to messenger.com, and your agents can read your chats and, when you ask, send.</p>';
  }, { skeleton: 1 });
}
list.onclick = async (e) => {
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
function ready() {
  connect.innerHTML = form;
  document.getElementById("start").onclick = pick;
}

// Picks a profile not linked yet, or names a new one.
async function pick(e) {
  const b = e?.currentTarget;
  if (b) {
    b.disabled = true;
    b.innerHTML = '<span class="spinner small" aria-hidden="true"></span>Add account';
  }
  const [all] = await Promise.all([known, loaded]);
  known = profiles();
  const free = [...new Set(all)].filter((p) => !accounts.some((a) => a.account === p));
  connect.innerHTML = \`<div class="small muted">Which browser profile? Each has its own sign-ins, like Personal or Work.</div>
    <select id="profile">\${free.map((p) => \`<option>\${one.esc(p)}</option>\`).join("")}<option value="">New profile…</option></select>
    <input id="name" placeholder="Its name, like Personal or Work" autocapitalize="words" autocorrect="off" \${free.length ? "hidden" : ""}>
    <button class="primary" id="go">Sign in to Messenger</button><div id="err" class="error small"></div>\`;
  const select = document.getElementById("profile"), name = document.getElementById("name");
  select.onchange = () => (name.hidden = !!select.value);
  document.getElementById("go").onclick = async (e) => {
    const profile = select.value || name.value.trim();
    if (!profile) return name.focus();
    e.target.disabled = true;
    e.target.innerHTML = '<span class="spinner small" aria-hidden="true"></span>Opening a device…';
    try { signingIn(await one.call("sign_in", { profile })); }
    catch (err) { document.getElementById("err").textContent = one.plain(err).title; e.target.disabled = false; e.target.textContent = "Sign in to Messenger"; }
  };
}

function signingIn(r) {
  connect.innerHTML = \`<ol><li><a href="\${one.esc(r.url)}">Open the device</a>: its browser is on messenger.com, in the \${one.esc(r.profile)} profile.</li>
    <li>Sign in there, and tick Keep me signed in.</li><li>Come back and press Done.</li></ol>
    <button class="primary" id="done">Done</button><div id="err" class="error small"></div>\`;
  document.getElementById("done").onclick = async (e) => {
    e.target.disabled = true;
    e.target.innerHTML = '<span class="spinner small" aria-hidden="true"></span>Saving…';
    try { await one.call("_save", { profile: r.profile }); }
    catch (err) { document.getElementById("err").textContent = one.plain(err).title; e.target.disabled = false; e.target.textContent = "Done"; return; }
    await draw(true);
    ready();
  };
}

document.getElementById("start").onclick = pick;
one.on("changed", () => draw(true));
const loaded = draw(true);
if (one.params.add) document.getElementById("start").click();
// With nowhere to sign in, Add account goes: what's missing shows instead, with the way to add it.
one.call("_running").then(({ devices }) => {
  if (devices.length) return;
  connect.hidden = true;
  const n = document.getElementById("needs");
  n.innerHTML = one.problem({ title: "Messenger needs Agent Devices.", text: "Add Agent Devices to one of your devices in the App Store, then sign in here.", warn: true,
    action: { label: "Open App Store", run: () => one.open("app", { app: "agent-devices" }, "apps") } });
  n.hidden = false;
}, () => {});
</script>`,
    },
  },
};
