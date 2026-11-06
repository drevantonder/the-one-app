// Messenger: the person's Facebook Messenger, read through messenger.com in the browser of a device borrowed from Agent
// Devices. Messenger has no API for personal accounts and its chats are end-to-end encrypted, so messenger.com in a
// browser is the way in. Each account is a browser profile signed in to it. This app borrows a device with the profile
// (agent-devices_borrow), drives its browser through Computer Use's tools (computer_*) and reads the page as the outline
// the browser tools give; its database keeps which profiles are linked and which device each one has borrowed, and a
// schedule gives back those left idle.

import settings from "./dist/settings.html" with { type: "text" };

const HOME = "https://www.messenger.com/";
/// How long a borrowed device is kept without a call.
const IDLE = 5 * 60 * 1000;
/// The chat list's rows each have a button with this and the chat's name.
const MENU = "More options for ";

const sleep = (ms) => new Promise((done) => setTimeout(done, ms));

const NEEDS = "Messenger needs Agent Devices on one of your nodes. Add it in the App Store, choosing the node it runs on; its page there shows where it runs. Then messenger_sign_in signs in.";

/// Runs a tool of Agent Devices (`agent-devices`) or Computer Use (`computer`) for the caller. When none of their
/// devices runs Agent Devices, says what's needed instead of how the call failed.
async function use(ctx, app, op, input) {
  try {
    return await ctx.apps[app][op](input);
  } catch (err) {
    const why = String(err.message);
    if (why.includes("none of yours runs it") || why.includes("No device of yours lets agents use it")) throw new Error(NEEDS);
    throw new Error(`agent devices ${op}: ${why}`);
  }
}

// What the device's browser tells of a page is an outline, one node per line:
//
//   - row "Kim Unread message: Lunch? 2 hours ago More options for Kim"
//     - link "Kim Unread message: Lunch? 2 hours ago" [focusable=true]
//   - statictext "Kim"
//
// Messenger's chat list, a conversation and search results are read from it.

/// `- role "label"` lines as { role, label }, the label unquoted.
function outline(text) {
  const nodes = [];
  for (const l of String(text).split("\n")) {
    const rest = l.trimStart().startsWith("- ") ? l.trimStart().slice(2) : null;
    if (rest === null) continue;
    const role = rest.match(/^[a-z]+/)?.[0];
    if (!role) continue;
    let label = rest.slice(role.length).startsWith(' "') ? quoted(rest.slice(role.length + 2)) ?? "" : "";
    if (label.includes("\\")) {
      try { label = JSON.parse(`"${label}"`); } catch {}
    }
    nodes.push({ role, label });
  }
  return nodes;
}

/// The text up to the closing quote, escapes and all; null if it isn't closed.
function quoted(s) {
  for (let i = 0; i < s.length; i++) {
    if (s[i] === "\\") i++;
    else if (s[i] === '"') return s.slice(0, i);
  }
  return null;
}

/// A list's rows: each one's link, its time, and its texts before the time.
function rows(nodes) {
  const out = [];
  let timed = false;
  for (const n of nodes) {
    if (n.role === "row") {
      out.push({ label: n.label, link: "", time: "", texts: [] });
      timed = false;
      continue;
    }
    const row = out[out.length - 1];
    if (!row) continue;
    if (n.role === "link" && !row.link) row.link = n.label;
    else if (timed) continue;
    else if (n.role === "abbr") [row.time, timed] = [n.label, true];
    else if (n.role === "statictext" || n.role === "image") {
      const label = n.label.split(". To get missing image descriptions")[0].trim();
      if (label) row.texts.push(label);
    }
  }
  return out;
}

/// The chat list's rows: { name, last, time, unread, link } (the link by its name among the page's refs).
function chatsOf(nodes) {
  const out = [];
  for (const r of rows(nodes)) {
    const i = r.label.lastIndexOf(MENU);
    if (i < 0) continue;
    const c = { name: r.label.slice(i + MENU.length), last: "", time: r.time, unread: false, link: r.link };
    const texts = [];
    for (const t of r.texts) {
      if (t === "Unread message:") c.unread = true;
      else if (!(t === "Active now" || (t.startsWith("Active ") && t.endsWith(" ago")))) texts.push(t);
    }
    c.last = withoutName(texts, c.name).join(" ");
    out.push(c);
  }
  return out;
}

/// The chat list's rows across a page's parts, each once: a row cut between two parts shows in both, each missing what
/// the other has.
function chatsIn(parts) {
  const out = [];
  const at = new Map();
  for (const part of parts) {
    for (const c of chatsOf(part)) {
      const i = at.get(c.name);
      if (i === undefined) {
        at.set(c.name, out.length);
        out.push(c);
        continue;
      }
      const link = out[i].link || c.link;
      if (!out[i].time && c.time) out[i] = c;
      out[i].link = link;
    }
  }
  return out;
}

/// A name whatever its spacing and emoji variants.
const squash = (s) => s.replace(/[  ️]/g, "");

/// Drops the leading texts that spell the chat's name: a group's emoji come as images between its words.
function withoutName(texts, name) {
  const want = squash(name);
  let got = "";
  for (let i = 0; i < texts.length; i++) {
    if (got === want) return texts.slice(i);
    const next = got + squash(texts[i]);
    if (!want.startsWith(next)) break;
    got = next;
  }
  if (got === want) return [];
  // The name didn't line up: it's the first text.
  return texts.length ? texts.slice(1) : texts;
}

/// The open conversation's messages: { sender, time, text }, "You" the account's own.
function messagesOf(nodes) {
  return nodes.filter((n) => n.role === "button").map((n) => message(n.label)).filter(Boolean)
    .map(([time, sender, text]) => ({ time, sender, text: text.trim() ? text.trim() : "(attachment)" }));
}

/// A message's button: `Enter, Message sent <time> by <sender>[: <text>]`, its time the shortest that fits.
function message(label) {
  const start = "Enter, Message sent ";
  if (!label.startsWith(start)) return null;
  const rest = label.slice(start.length);
  for (let i = rest.indexOf(" by "); i >= 0; i = rest.indexOf(" by ", i + 1)) {
    if (i === 0) continue;
    const after = rest.slice(i + 4);
    const k = after.indexOf(":");
    if (k < 0) {
      if (after) return [rest.slice(0, i), after, ""];
    } else if (k > 0 && after.slice(k).startsWith(": ")) {
      return [rest.slice(0, i), after.slice(0, k), after.slice(k + 2)];
    }
  }
  return null;
}

/// The chats a search found: { chat, matches (when it has several), text (when it has one), time }.
function matchesOf(nodes) {
  const out = [];
  for (const r of rows(nodes)) {
    if (!r.texts.length || r.label.includes(MENU)) continue;
    const rest = r.texts.slice(1).join(" ");
    const count = (rest.endsWith(" matched messages") ? rest.slice(0, -" matched messages".length) : rest.endsWith(" matched message") ? rest.slice(0, -" matched message".length) : null);
    const counted = count && /^[0-9]+$/.test(count);
    out.push({ chat: r.texts[0], matches: counted ? Number(count) : 0, text: counted ? "" : rest, time: r.time });
  }
  return out;
}

/// No recent chat has the name asked for.
class NoChat extends Error {}

/// The chat called `want`: by its whole name, or by a part of it only one chat has.
function pick(chats, want) {
  const w = want.trim().toLowerCase();
  const partial = [];
  for (const c of chats) {
    const n = c.name.toLowerCase();
    if (n === w) return c;
    if (n.includes(w)) partial.push(c);
  }
  const names = (cs) => cs.map((c) => c.name).join("; ");
  if (partial.length === 1) return partial[0];
  if (!partial.length) throw new NoChat(`no recent chat called ${JSON.stringify(want)}; recent chats: ${names(chats)}`);
  throw new Error(`${JSON.stringify(want)} matches several chats: ${names(partial)}. Give more of the name`);
}

/// A browser profile Messenger reads isn't signed in to it.
class NotSignedIn extends Error {
  constructor() {
    super("Messenger isn't signed in on that browser profile. Ask the person to sign in again: messenger_sign_in with the profile gives them a link, where they sign in at messenger.com (with Keep me signed in), then save the profile");
  }
}

/// A tool's structured answer: Agent Devices gives it as its content's last text.
function structured(got) {
  for (const item of [...(got?.content || [])].reverse()) {
    try {
      const v = JSON.parse(item.text);
      if (v && typeof v === "object" && !Array.isArray(v)) return v;
    } catch {}
  }
  return {};
}

const text = (v) => (typeof v === "string" ? v : "");

/// A page as the browser tools read it: its address, its outline, in parts (a row cut between two shows in both), and
/// its refs, by role and name.
class Page {
  constructor() {
    this.url = "";
    this.nodes = [];
    this.parts = [];
    this.refs = [];
  }
  find(role, name) {
    return this.refs.find((r) => r.role === role && r.name === name)?.id;
  }
  /// Whether the page is Messenger's sign-in page.
  signedOut() {
    return !!this.find("button", "Log In") || !!this.find("textbox", "Password") || this.url.includes("/login");
  }
  chats() {
    return chatsIn(this.parts);
  }
}

/// The browser in a device borrowed from Agent Devices, starting with a copy of a browser profile signed in to
/// messenger.com. `row` is what the database keeps of it from the last call.
class Browser {
  constructor(ctx, profile, row) {
    this.ctx = ctx;
    this.profile = profile;
    this.device = row?.device || "";
    /// The device's tools keep a browser in a session.
    this.session = row?.session || "";
    this.target = row?.target || "";
    this.tab = row?.tab || "";
    /// Off the chat list: a conversation, or search results.
    this.dirty = !!row?.dirty;
    this.borrowed = false;
  }

  /// Keeps it for the next call, or forgets it when its device was given back.
  save() {
    if (!this.device) return this.ctx.user.exec("delete from browsers where profile = ?", [this.profile]);
    this.ctx.user.exec("insert or replace into browsers values (?, ?, ?, ?, ?, ?, ?)", [this.profile, this.device, this.session, this.target, this.tab, Date.now(), this.dirty ? 1 : 0]);
  }

  /// Takes a device with the profile, unless one is borrowed: one with no screen, since nobody watches it read.
  async borrow() {
    if (this.device) return;
    const got = await use(this.ctx, "agent-devices", "borrow", { for: "Reading Messenger", by: "messenger", profile: this.profile, screen: false });
    this.device = text(got?.device);
    [this.target, this.tab] = ["", ""];
    this.session = `messenger-${Date.now()}-${Math.floor(Math.random() * 1e9)}`;
    if (!this.device) throw new Error("couldn't borrow a device");
    this.borrowed = true;
  }

  async giveBack() {
    if (!this.device) return;
    await use(this.ctx, "agent-devices", "give_back", { device: this.device }).catch(() => {});
    this.device = "";
  }

  /// Runs one of the device's tools and returns its structured answer, borrowing a device again if the last one was
  /// given back.
  async tool(name, input) {
    try {
      return await this.tryTool(name, input);
    } catch (err) {
      // The device was given back, as after 15 minutes without a call.
      if (!this.device || !String(err.message).includes(this.device)) throw err;
      this.device = "";
      if (name !== "browser_prepare") await this.open();
      return await this.tryTool(name, input);
    }
  }

  async tryTool(name, input) {
    await this.borrow();
    input = { ...input, session: this.session };
    let got;
    if (name === "list_windows") {
      got = await use(this.ctx, "computer", "use", { device: this.device, tool: name, args: input });
    } else {
      input.device = this.device;
      if (this.target && name !== "browser_prepare") [input.target_id, input.tab_id] = [this.target, this.tab];
      got = await use(this.ctx, "computer", name, input);
    }
    const data = structured(got);
    if (data.status === "refused") throw new Error(`${name} refused: ${text(data.message)}`);
    return data;
  }

  /// Starts the device's browser on the profile and attaches to its tab.
  async open() {
    [this.target, this.tab] = ["", ""];
    let prepared;
    try {
      prepared = await this.tool("browser_prepare", { allow_launch: true, profile: { mode: "isolated_named", name: this.profile } });
    } catch (err) {
      throw new Error(`starting the browser: ${err.message}`);
    }
    const pid = Math.trunc(Number(prepared.prepared_pid) || 0);
    const window = (await this.tool("list_windows", { pid })).windows?.[0];
    if (!window) throw new Error("the browser has no window");
    const bound = await this.tool("get_browser_state", { pid, window_id: window.window_id });
    this.target = text(bound.target_id);
    // The active tab, or else the first.
    for (const t of bound.tabs || []) {
      if (t.active === true || !this.tab) this.tab = text(t.tab_id);
    }
    if (!this.target || !this.tab) throw new Error("couldn't attach to the browser");
    this.dirty = true;
  }

  /// Reads the page, or the parts matching `query`; it attaches again once if the browser closed (it does after a few
  /// idle minutes).
  async snapshot(query) {
    let failed = null;
    for (let attempt = 0; attempt < 2; attempt++) {
      if (!this.target || attempt > 0) await this.open();
      const input = { snapshot_format: "semantic_v2" };
      if (query) input.query = query;
      const page = new Page();
      let all = "";
      failed = null;
      for (let n = 0; n < 6; n++) {
        let part;
        try {
          part = await this.tool("get_browser_state", input);
        } catch (err) {
          failed = err;
          break;
        }
        const t = text(part.outline);
        all += t + "\n";
        page.parts.push(outline(t));
        page.refs.push(...(part.refs || []).map((r) => ({ id: text(r?.ref), role: text(r?.role), name: text(r?.name) })));
        page.url = text(part.page?.url) || text(part.page?.URL);
        const more = text(part.snapshot?.continuation);
        if (more) input.continuation = more;
        else break;
      }
      if (!failed) {
        page.nodes = outline(all);
        return page;
      }
    }
    throw failed || new Error("couldn't read the page");
  }

  /// Reads the chat list, loading it unless it's showing. Anything else is loaded over rather than read: the browser
  /// tools drop a page whose DOM nests too deeply, as some conversations' do, and are slow to say so.
  async list() {
    if (this.target && !this.dirty) {
      const p = await this.snapshot("").catch(() => null);
      if (p && p.url === HOME && p.chats().length) return p;
    }
    // If the list doesn't show, load it once more.
    for (let round = 0; round < 2; round++) {
      await this.navigate(HOME);
      this.dirty = false;
      // A browser that just started can take a while.
      for (let n = 0; n < 30; n++) {
        await sleep(500);
        const p = await this.snapshot("");
        if (p.signedOut()) {
          // Its copy of the profile stays signed out: borrow afresh next time, after the person signed in.
          await this.giveBack();
          throw new NotSignedIn();
        }
        if (p.chats().length) return p;
      }
    }
    throw new Error("Messenger's chat list didn't show");
  }

  /// Loads `url`, attaching again once if the browser closed.
  async navigate(url) {
    if (!this.target) await this.open();
    try {
      await this.tool("browser_navigate", { url });
      return;
    } catch {}
    await this.open();
    await this.tool("browser_navigate", { url });
  }

  async click(p, role, name) {
    const ref = p.find(role, name);
    if (!ref) throw new Error(`${role} ${JSON.stringify(name)} isn't on the page`);
    await this.tool("browser_click", { ref, input_route: "dom_event" });
  }

  /// Marks a chat unread again from its menu in the chat list.
  async markUnread(chat) {
    const menu = MENU + chat;
    await this.click(await this.snapshot(menu), "button", menu);
    for (let n = 0; n < 10; n++) {
      await sleep(300);
      const p = await this.snapshot("Mark as unread");
      if (p.find("menuitem", "Mark as unread")) return this.click(p, "menuitem", "Mark as unread");
    }
    throw new Error("its menu had no Mark as unread");
  }

  async chats(unread) {
    const lines = (await this.list()).chats().filter((c) => !unread || c.unread).map((c) => `${c.name}${c.unread ? " (unread)" : ""}, ${c.time}: ${c.last}`);
    return lines.length ? lines.join("\n") : "No unread chats.";
  }

  async read(chat) {
    if (!chat.trim()) throw new Error("which chat?");
    const p = await this.list();
    const c = pick(p.chats(), chat);
    this.dirty = true;
    await this.click(p, "link", c.link);
    let msgs = [];
    const deadline = Date.now() + 10_000;
    while (!msgs.length && Date.now() < deadline) {
      await sleep(500);
      msgs = messagesOf((await this.snapshot("Message sent")).nodes);
    }
    let note = "";
    if (c.unread) {
      try {
        await this.list();
        await this.markUnread(c.name);
        note = "\n(Marked unread again.)";
      } catch (err) {
        note = `\n(Couldn't mark it unread again: ${err.message})`;
      }
    }
    if (!msgs.length) throw new Error(`no messages showed in ${c.name}${note}`);
    return msgs.map((m) => `[${m.time}] ${m.sender}: ${m.text}`).join("\n") + note;
  }

  async search(query) {
    if (!query.trim()) throw new Error("what to search for?");
    const p = await this.list();
    // The list shows results until it's loaded again.
    this.dirty = true;
    const box = p.find("combobox", "Search Messenger");
    if (!box) throw new Error("no search box on the page");
    await this.tool("browser_type", { ref: box, text: query, replace: true });
    let link;
    for (let n = 0; n < 10 && !link; n++) {
      link = (await this.snapshot("Search messages for")).find("link", `Search messages for ${query}`);
      if (!link) await sleep(300);
    }
    if (!link) throw new Error("Messenger didn't offer to search messages");
    await this.tool("browser_click", { ref: link, input_route: "dom_event" });
    let found = [];
    for (let n = 0; n < 10 && !found.length; n++) {
      await sleep(500);
      found = matchesOf((await this.snapshot("")).nodes);
    }
    if (!found.length) return "Nothing found.";
    return found.map((m) => (m.matches > 0 ? `${m.chat}: ${m.matches} matching messages` : `${m.chat}, ${m.time}: ${m.text}`)).join("\n");
  }

  async send(chat, message) {
    if (!message.trim() || !chat.trim()) throw new Error("chat and text are required");
    const name = await this.openChat(chat);
    let box;
    // The conversation loads; on a narrow page its details may cover it.
    for (let n = 0; n < 16 && !box; n++) {
      const p = await this.snapshot("Write to");
      box = p.refs.findLast((r) => r.role === "textbox" && r.name.startsWith("Write to"))?.id;
      if (box) break;
      if (p.find("button", "Back")) await this.click(p, "button", "Back").catch(() => {});
      await sleep(500);
    }
    if (!box) throw new Error(`no message box in the chat with ${name}`);
    await this.tool("browser_type", { ref: box, text: message });
    await this.tool("browser_type", { ref: box, text: "\n", mode: "keystrokes" });
    // Confirm it from the chat list, which is quick to read where the conversation may not be.
    const words = (s) => s.split(/\s+/).filter(Boolean).join(" ");
    const want = `You: ${words(message)}`;
    await sleep(1000);
    for (let n = 0; n < 10; n++) {
      const p = await this.list();
      let c = null;
      try { c = pick(p.chats(), name); } catch {}
      if (c && words(c.last) === want) return `Sent to ${c.name}.`;
      await sleep(500);
      this.dirty = true;
    }
    throw new Error(`typed the message to ${name}, but it didn't show as sent: look at the Agent Devices page`);
  }

  /// Opens a recent chat, or else the one person Messenger suggests whose name is exactly `want`, and says its name.
  /// Anything less certain is refused, so a message never goes to the wrong person.
  async openChat(want) {
    const p = await this.list();
    this.dirty = true;
    try {
      const c = pick(p.chats(), want);
      await this.click(p, "link", c.link);
      return c.name;
    } catch (err) {
      if (!(err instanceof NoChat)) throw err;
    }
    const box = p.find("combobox", "Search Messenger");
    if (!box) throw new Error("no search box on the page");
    await this.tool("browser_type", { ref: box, text: want, replace: true });
    let found = [];
    for (let n = 0; n < 10; n++) {
      await sleep(500);
      const page = await this.snapshot(want);
      const people = page.refs.filter((r) => r.role === "link" && !r.name.startsWith("Search messages for"));
      found = people.map((r) => r.name);
      const exact = people.filter((r) => r.name.toLowerCase() === want.trim().toLowerCase());
      if (exact.length === 1) {
        await this.tool("browser_click", { ref: exact[0].id, input_route: "dom_event" });
        return want.trim();
      }
      if (exact.length > 1) throw new Error(`several people are called ${JSON.stringify(want)}: send from the agent device yourself`);
    }
    const how = ": messenger_chats lists recent chats; anyone else by the name Messenger shows for them";
    if (!found.length) throw new Error(`no chat or person called ${JSON.stringify(want)}${how}`);
    throw new Error(`no chat or person called exactly ${JSON.stringify(want)}; Messenger suggests: ${found.join("; ")}${how}`);
  }
}

// What the app keeps, in the person's own scope: the browser profiles linked, the default first, and why one needs
// signing in again, as its last read found; and the device each profile has borrowed, with its browser. An install that
// linked none yet has the browser profile called default.
const SCHEMA = `
  create table accounts (profile text primary key, position integer not null, why text);
  create table settings (key text primary key, value text not null);
  create table browsers (profile text primary key, device text not null, session text not null, target text not null, tab text not null, used integer not null, dirty integer not null);`;

async function load(ctx) {
  if (!ctx.user) throw new Error("This device doesn't keep your data: use one of your own devices");
  const [accounts, seeded, browsers] = await Promise.all([
    ctx.user.query("select profile, why from accounts order by position"),
    ctx.user.query("select 1 as seeded from settings where key = 'linked'"),
    ctx.user.query("select * from browsers"),
  ]);
  return {
    linked: seeded.length ? accounts.map((a) => a.profile) : ["default"],
    why: Object.fromEntries(accounts.filter((a) => a.why).map((a) => [a.profile, a.why])),
    browsers: Object.fromEntries(browsers.map((b) => [b.profile, b])),
  };
}

/// Keeps the profiles linked, and why some need signing in again.
function saveAccounts(ctx, s) {
  ctx.user.exec("delete from accounts");
  s.linked.forEach((profile, i) => ctx.user.exec("insert into accounts values (?, ?, ?)", [profile, i, s.why[profile] || null]));
  ctx.user.exec("insert or replace into settings values ('linked', '1')");
}

/// Keeps whether a read found the profile signed in.
function found(ctx, s, profile, why) {
  if ((s.why[profile] || null) === why) return;
  if (why) s.why[profile] = why;
  else delete s.why[profile];
  saveAccounts(ctx, s);
}

/// The linked profile an account names, or the first when none is named.
function profileOf(s, account) {
  const [first] = s.linked;
  if (!first) throw new Error("Messenger isn't linked to a browser profile yet: messenger_sign_in links one");
  if (!account) return first;
  const profile = s.linked.find((p) => p.toLowerCase() === String(account).toLowerCase());
  if (!profile) throw new Error(`Messenger isn't linked to a profile ${JSON.stringify(account)}; it's linked to ${s.linked.join(", ")}`);
  return profile;
}

/// Runs one of the browser's reads (or the send) on the account's profile, keeping its device for the next call. A profile
/// Messenger signed out is kept as one that needs signing in: its read says so as its answer, since a call that fails
/// keeps nothing; a send fails.
async function browse(ctx, input, op, run) {
  const s = await load(ctx);
  const profile = profileOf(s, input.account);
  const b = new Browser(ctx, profile, s.browsers[profile]);
  let result;
  try {
    result = await run(b);
    if (op !== "send") found(ctx, s, profile, null);
  } catch (err) {
    // A device borrowed for this call isn't kept when it fails: give it back.
    if (!(err instanceof NotSignedIn) || op === "send") {
      if (b.borrowed) await b.giveBack();
      throw err;
    }
    found(ctx, s, profile, "Messenger signed this profile out: sign in again");
    result = err.message;
  }
  b.save();
  return result;
}

// Each account is a Vault item too, so the person's Vault lists it: the browser profile, plain, with no secret, since the
// sign-in itself stays in the profile.
const TYPE = "Messenger account";
const VAULT = {
  account: { title: TYPE, type: TYPE, domain: "messenger.com", accounts: true, fields: { profile: { label: "Browser profile" } } },
};

/// Makes an item for each linked profile that has none, and lets go of those whose profile isn't linked.
async function keep(ctx, linked) {
  if (!ctx.user) return;
  const items = (await ctx.vault.items()).filter((i) => i.type === TYPE && i.scope === ctx.user.id);
  for (const profile of linked) {
    if (!items.some((i) => i.fields.profile?.value === profile)) await ctx.vault.create({ title: profile, type: TYPE, domain: "messenger.com", hosts: [], same: "profile", fields: { profile: { value: profile } } });
  }
  for (const item of items) if (!linked.includes(item.fields.profile?.value)) ctx.vault.release(item.id);
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
  uses: { apps: ["agent-devices", "computer"] },
  vault: VAULT,
  schema: { user: SCHEMA },

  // A device borrowed for a read goes back after 5 idle minutes; Agent Devices takes one back after 15 without a call. Runs on
  // the device that runs Messenger's schedules (one devices run_schedules messenger on <it>).
  schedules: { idle: { op: "_idle", every: "2m", description: "Gives back the devices Messenger borrowed and isn't using" } },

  tools: {
    accounts: {
      reads: true,
      description: "The browser profiles Messenger reads, the default first: each { account, ok }, account the profile's name, with why when ok is false and it needs signing in again. Give account to the others to pick one.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const s = await load(ctx);
        await keep(ctx, s.linked).catch(() => {}); // a call that only reads lists them all the same
        return s.linked.map((account) => (s.why[account] ? { account, ok: false, why: s.why[account] } : { account, ok: true }));
      },
    },
    chats: {
      reads: true,
      description: "The recent Messenger chats messenger.com shows, newest first, to see what's new or find a chat to read: one a line, \"Sam (unread), <time>: last message\", the time as Messenger shows it. Older chats aren't reached.",
      input: { type: "object", properties: { account: ACCOUNT, unread: { type: "boolean", description: "only chats with unread messages" } } },
      run: (ctx, input) => browse(ctx, input, "chats", (b) => b.chats(input.unread === true)),
    },
    read: {
      reads: false,
      description: "A recent chat's latest messages, as messenger.com shows them when it opens, newest last: one a line, \"[time] Sam: text\", the time as Messenger shows it. Opening a chat marks it read, so one that was unread is marked unread again afterwards, but the sender may see it as seen.",
      input: { type: "object", required: ["chat"], properties: { account: ACCOUNT, chat: { type: "string", description: CHAT } } },
      run: (ctx, input) => browse(ctx, input, "read", (b) => b.read(text(input.chat))),
    },
    search: {
      reads: true,
      description: "Messages containing some words, across every chat, as Messenger's search finds them: to find something said when you don't know which chat. One a line: the chat and the message when it has one match (\"Sam, <time>: text\"), or how many (\"Family: 3 matching messages\"), for messenger_read.",
      input: { type: "object", required: ["query"], properties: { account: ACCOUNT, query: { type: "string", description: "words to find, as you'd type them in Messenger's search" } } },
      run: (ctx, input) => browse(ctx, input, "search", (b) => b.search(text(input.query))),
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
      run: (ctx, input) => browse(ctx, input, "send", (b) => b.send(text(input.chat), text(input.text))),
    },
    sign_in: {
      reads: false,
      description: "For the person: links Messenger to a browser profile (made if new) and gives a link to messenger.com in it, for them to sign in there themselves; never sign in for them. Signs the default profile in again when none is given.",
      input: { type: "object", properties: { profile: { type: "string", description: "the browser profile's name, like Personal or Work; agent-devices_profiles lists them" } } },
      async run(ctx, input) {
        const s = await load(ctx);
        const profile = text(input.profile).trim() || text(input.account).trim() || s.linked[0] || "default";
        const ask = () => use(ctx, "agent-devices", "sign_in", { profile, url: HOME });
        let got;
        try {
          got = await ask();
        } catch (err) {
          // A new one: added to the Vault first.
          if (!String(err.message).includes("No browser profile")) throw err;
          await use(ctx, "agent-devices", "add_profile", { profile });
          got = await ask();
        }
        const url = text(got.url);
        if (!s.linked.includes(profile)) s.linked.push(profile);
        delete s.why[profile];
        saveAccounts(ctx, s);
        await keep(ctx, s.linked);
        return {
          profile, url, device: got.device, on: got.on,
          text: `Give the person this link: ${url}\nThey sign in to Messenger there, in the ${profile} browser profile (tick Keep me signed in), then press Done on Messenger's settings, or Done Signing In on the profile in the Vault; it's saved by itself after 30 minutes otherwise.`,
        };
      },
    },
    sign_out: {
      reads: false,
      description: "Stop reading a browser profile's Messenger. The profile, and its sign-ins to messenger.com and other sites, stay, so messenger_sign_in links it again.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "the browser profile's name, as messenger_accounts lists them" } } },
      async run(ctx, input) {
        const s = await load(ctx);
        const account = text(input.account);
        const i = s.linked.findIndex((p) => p.toLowerCase() === account.toLowerCase());
        if (i < 0) throw new Error(`Messenger isn't linked to a profile ${JSON.stringify(account)}`);
        const [profile] = s.linked.splice(i, 1);
        delete s.why[profile];
        const row = s.browsers[profile];
        if (row) {
          await use(ctx, "agent-devices", "give_back", { device: row.device }).catch(() => {});
          ctx.user.exec("delete from browsers where profile = ?", [profile]);
        }
        saveAccounts(ctx, s);
        await keep(ctx, s.linked);
        return `Messenger no longer reads the ${profile} profile. The profile and its other sign-ins stay, on the Agent Devices page.`;
      },
    },
    _profiles: {
      reads: true,
      description: "The browser profiles, for the settings screen to pick one.",
      input: { type: "object", properties: {} },
      run: (ctx) => use(ctx, "agent-devices", "profiles", {}),
    },
    _save: {
      reads: false,
      description: "The settings screen's Done: saves the browser profile signed in.",
      input: { type: "object", properties: { profile: { type: "string" } } },
      async run(ctx, input) {
        const profile = text(input.profile);
        await use(ctx, "agent-devices", "save_profile", { profile });
        found(ctx, await load(ctx), profile, null);
        return `Saved the ${profile} profile.`;
      },
    },
    _running: {
      reads: true,
      description: "Which of your devices run Agent Devices, for the screens to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ devices: (await ctx.devices()).filter((d) => d.online && (d.services || []).includes("agent-devices")).map((d) => d.name) }),
    },
    _idle: {
      reads: false,
      description: "The schedule's: gives back the devices left idle for 5 minutes.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const idle = await ctx.user.query("select profile, device from browsers where used < ?", [Date.now() - IDLE]);
        for (const b of idle) {
          await use(ctx, "agent-devices", "give_back", { device: b.device }).catch(() => {});
          ctx.user.exec("delete from browsers where profile = ?", [b.profile]);
        }
        return { gave_back: idle.length };
      },
    },
  },

  screens: {
    settings: { title: "Messenger", html: settings },
  },
};
