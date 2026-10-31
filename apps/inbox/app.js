// The inbox: notifications to a person, a group or the admins, from apps (ctx.apps.inbox.notify), agents and the CLI,
// each with a title, a body and a link into an app's screen. A notification to yourself stays in your own scope; one
// to a group goes in the group's scope; one to someone else, or to the admins, goes in global, and only its recipients
// see it here. What you've read is kept in your own scope.
// Phones register here for push; notify sends through the push driver (APNs, on a server that holds its key) and says
// whether a push was accepted or there was no device to send to. Never whether anyone saw it.

const LIMIT = 200;

const notices = `create table notices (id text primary key, recipient text not null, title text not null, body text not null,
                   link text, app text, user text not null, sender text not null, machine text not null, time integer not null);
                 create index notices_time on notices (time)`;

const scopeOf = (ctx, s) => (s.startsWith("group:") ? ctx.group(s) : s === "global" ? ctx.global : ctx.user);

/// Where a notification to `to` goes, and whom it's for there.
function place(ctx, to) {
  const me = `user:${ctx.me.user}`;
  if (!to || to === "me" || to === me) {
    if (!ctx.user) throw new Error("your own inbox isn't here");
    return { scope: ctx.user, recipient: me };
  }
  if (/^[0-9a-f]{16}$/.test(to)) to = `group:${to}`;
  // A group you're in keeps it in its own scope; one you're not in gets it through global.
  const g = to.startsWith("group:") && ctx.group(to);
  if (g) return { scope: g, recipient: g.id };
  if (!ctx.global) throw new Error("only notifications to yourself or your groups work until an admin adds the inbox for everyone");
  if (to === "everyone" && ctx.me.role !== "admin") throw new Error("only admins notify everyone");
  if (to === "admins" || to === "everyone" || /^(user|group):[0-9a-f]{16}$/.test(to)) return { scope: ctx.global, recipient: to };
  throw new Error(`to is me, user:<id>, group:<id>, admins or everyone, not ${to}`);
}

/// The devices of the people a notification is for, as their phones registered them.
async function devicesFor(ctx, recipient) {
  if (!ctx.global) return recipient === `user:${ctx.me.user}` && ctx.user ? ctx.user.query("select * from devices where gone = 0") : [];
  const all = await ctx.global.query("select * from devices where gone = 0");
  if (recipient.startsWith("user:")) return all.filter((d) => d.user === recipient.slice(5));
  if (recipient === "admins") return all.filter((d) => d.role === "admin");
  if (recipient === "everyone") return all;
  return all.filter((d) => JSON.parse(d.groups || "{}")[recipient.slice(6)]);
}

/// Every notification for the caller, newest first, with whether they've read it.
async function mine(ctx, limit = LIMIT) {
  const me = `user:${ctx.me.user}`;
  const reads = ctx.user ? await ctx.user.query("select id from reads") : [];
  const before = ctx.user ? (await ctx.user.query("select value from marks where key = 'read_before'"))[0]?.value ?? 0 : 0;
  const read = new Set(reads.map((r) => r.id));
  const out = [];
  if (ctx.user) out.push(...(await ctx.user.query("select * from notices order by time desc limit ?", [limit])));
  for (const g of ctx.groups) out.push(...(await g.query("select * from notices order by time desc limit ?", [limit])));
  if (ctx.global) {
    const to = [me, "everyone", ...ctx.groups.map((g) => g.id), ...(ctx.me.role === "admin" ? ["admins"] : [])];
    out.push(...(await ctx.global.query(`select * from notices where recipient in (${to.map(() => "?").join(",")}) order by time desc limit ?`, [...to, limit])));
  }
  out.sort((a, b) => b.time - a.time);
  return out.slice(0, limit).map((n) => ({
    id: n.id, title: n.title, body: n.body, link: n.link ? JSON.parse(n.link) : null, app: n.app, to: n.recipient,
    from: n.sender, machine: n.machine, time: n.time, read: read.has(n.id) || n.time <= before,
  }));
}

export default {
  name: "inbox",
  title: "Inbox",
  icon: "bell",
  color: "#ff3b30",
  description: "Notifications from your apps, agents and people",
  instructions: "inbox_notify tells a person, a group or the admins something (title, body, and a link into an app's screen); it lands in their Inbox, and on their phone if it can push. It says whether a push was accepted or there was no device, never whether anyone saw it. inbox_list reads your own.",

  uses: { drivers: ["push"] },

  schema: {
    user: `${notices};
           create table reads (id text primary key);
           create table marks (key text primary key, value integer);
           create table devices (token text primary key, user text, role text, groups text, platform text, env text, name text, time integer, gone integer)`,
    group: notices,
    global: `${notices};
             create table devices (token text primary key, user text, role text, groups text, platform text, env text, name text, time integer, gone integer)`,
  },

  ops: {
    notify: {
      description: "Notify someone: to is me (the default), user:<id>, group:<id>, admins or everyone (admins only). link opens an app's screen: { app, screen, params }. Returns delivery: accepted (a push service took it for at least one device) or no_device.",
      input: {
        type: "object",
        required: ["title"],
        properties: {
          title: { type: "string" },
          body: { type: "string" },
          to: { type: "string", description: "me, user:<id>, group:<id>, admins or everyone" },
          link: { type: "object", description: "{ app, screen, params }: the screen it opens" },
        },
      },
      async run(ctx, { title, body = "", to, link }) {
        title = String(title || "").trim();
        if (!title) throw new Error("give it a title");
        if (link && (typeof link !== "object" || !link.app)) throw new Error("link is { app, screen, params }");
        const { scope, recipient } = place(ctx, to);
        scope.append("notice", { recipient, title: title.slice(0, 200), body: String(body).slice(0, 2000), link: link || null, app: ctx.from || null });
        const devices = await devicesFor(ctx, recipient);
        if (!devices.length) return { to: recipient, delivery: "no_device", why: "nobody it's for has a phone registered for push" };
        try {
          const sent = await ctx.drivers.push.send({ devices: devices.map((d) => ({ token: d.token, platform: d.platform, env: d.env })), title, body, link: link || null, app: ctx.from || null });
          // Tokens the push service says are no longer valid (the app was deleted from the phone).
          for (const token of sent.gone || []) (ctx.global || ctx.user).append("device", { token, gone: true });
          return { to: recipient, delivery: sent.accepted > 0 ? "accepted" : "no_device", devices: devices.length, accepted: sent.accepted, ...(sent.why ? { why: sent.why } : {}) };
        } catch (err) {
          return { to: recipient, delivery: "no_device", why: `no push: ${err.message}` };
        }
      },
    },
    list: {
      description: "Your notifications, newest first, with whether you've read each, and how many are unread.",
      input: { type: "object", properties: { unread: { type: "boolean", description: "only the unread ones" }, limit: { type: "number" } } },
      reads: true,
      async run(ctx, { unread, limit = 50 } = {}) {
        const all = await mine(ctx);
        const items = (unread ? all.filter((n) => !n.read) : all).slice(0, Math.min(limit, LIMIT));
        return { unread: all.filter((n) => !n.read).length, items };
      },
    },
    unread: {
      description: "How many of your notifications are unread.",
      input: { type: "object", properties: {} },
      reads: true,
      async run(ctx) {
        return { unread: (await mine(ctx)).filter((n) => !n.read).length };
      },
    },
    read: {
      description: "Mark a notification read, by id; without one, all of them.",
      input: { type: "object", properties: { id: { type: "string" } } },
      async run(ctx, { id }) {
        if (!ctx.user) throw new Error("your own inbox isn't here");
        if (id) ctx.user.append("read", { id });
        else ctx.user.append("read", { before: ctx.now() });
        return id ? "Read." : "All read.";
      },
    },
    register: {
      description: "A phone registers for push: its APNs device token (hex), platform ios, and env sandbox or production.",
      input: {
        type: "object",
        required: ["token"],
        properties: { token: { type: "string" }, platform: { type: "string" }, env: { type: "string" }, name: { type: "string" } },
      },
      async run(ctx, { token, platform = "ios", env = "production", name = "" }) {
        if (!/^[0-9a-f]{16,200}$/i.test(token || "")) throw new Error("a device token is hex");
        (ctx.global || ctx.user).append("device", { token: token.toLowerCase(), platform, env, name: name || ctx.me.name });
        return "Registered for push.";
      },
    },
    unregister: {
      description: "A phone stops getting pushes.",
      input: { type: "object", required: ["token"], properties: { token: { type: "string" } } },
      async run(ctx, { token }) {
        (ctx.global || ctx.user).append("device", { token: String(token).toLowerCase(), gone: true });
        return "Unregistered.";
      },
    },
    devices: {
      description: "Your devices registered for push.",
      input: { type: "object", properties: {} },
      reads: true,
      async run(ctx) {
        const s = ctx.global || ctx.user;
        return (await s.query("select token, platform, env, name, time from devices where user = ? and gone = 0", [ctx.me.user])).map((d) => ({ ...d, token: d.token.slice(0, 8) + "…" }));
      },
    },
  },

  on: {
    notice(db, e) {
      const d = e.data;
      db.exec("insert or ignore into notices values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", [
        e.id, d.recipient, d.title, d.body || "", d.link ? JSON.stringify(d.link) : null, d.app || null, e.user, e.user_name || "", e.name, e.time,
      ]);
    },
    read(db, e) {
      if (e.data.id) db.exec("insert or ignore into reads values (?)", [e.data.id]);
      if (e.data.before) db.exec("insert into marks values ('read_before', ?) on conflict (key) do update set value = max(value, excluded.value)", [e.data.before]);
    },
    device(db, e) {
      const d = e.data;
      if (d.gone) return db.exec("update devices set gone = 1 where token = ?", [d.token]);
      db.exec("insert or replace into devices values (?, ?, ?, ?, ?, ?, ?, ?, 0)", [d.token, e.user, e.role, JSON.stringify(e.groups || {}), d.platform, d.env, d.name, e.time]);
    },
  },

  widgets: {
    unread: {
      title: "Inbox",
      async render(ctx) {
        const all = await mine(ctx, 20);
        const unread = all.filter((n) => !n.read);
        if (!all.length) return { t: "col", c: [{ t: "text", text: "Nothing in your inbox", tone: "muted" }] };
        const n = unread[0] || all[0];
        return {
          t: "col",
          gap: 4,
          c: [
            { t: "row", gap: 6, c: [{ t: "text", text: unread.length ? `${unread.length} unread` : "All read", size: "sm", weight: "medium" }, { t: "time", at: n.time, size: "sm", tone: "muted" }] },
            { t: "text", text: n.title, weight: unread.length ? "bold" : undefined, lines: 2 },
          ],
        };
      },
    },
  },

  screens: {
    main: {
      title: "Inbox",
      html: `
<style>
  .dot { width: 9px; height: 9px; border-radius: 50%; background: var(--accent); flex: none; margin-top: 7px; }
  .dot.read { background: transparent; }
  .row.notice { align-items: flex-start; gap: 10px; }
  .row.notice.unread .title span:first-child { font-weight: 600; }
  .bar { display: flex; justify-content: flex-end; margin: 4px 4px 0; }
  .bar button { font: inherit; font-size: 15px; color: var(--accent); background: none; border: 0; padding: 4px 0; }
  .bar button:disabled { color: var(--muted); }
</style>
<div class="bar"><button id="all" disabled onclick="readAll()">Mark All as Read</button></div>
<div id="list"></div>
<script>
one.title("Inbox");
let items = [];
async function show() {
  const r = await one.call("list", { limit: 100 });
  items = r.items;
  document.getElementById("all").disabled = !r.unread;
  const row = (n, i) => \`<a class="row notice\${n.read ? "" : " unread"}" onclick="tap(\${i})">
      <div class="dot\${n.read ? " read" : ""}"></div>
      <div class="main">
        <div class="title"><span>\${one.esc(n.title)}</span><span class="when">\${one.ago(n.time)}</span></div>
        \${n.body ? '<div class="sub">' + one.esc(n.body) + "</div>" : ""}
        <div class="sub small">\${one.esc([n.app, n.from].filter(Boolean).join(" · "))}</div>
      </div>
      \${n.link ? '<div class="chev">›</div>' : ""}
    </a>\`;
  document.getElementById("list").innerHTML = items.length
    ? '<div class="list">' + items.map(row).join("") + "</div>"
    : '<div class="empty">No notifications<div class="small" style="margin-top:6px">Apps, agents and people you work with notify you here.</div></div>';
}
async function tap(i) {
  const n = items[i];
  if (!n.read) one.call("read", { id: n.id }).then(show);
  if (n.link) one.open(n.link.screen || "main", n.link.params || {}, n.link.app);
}
async function readAll() {
  await one.call("read", {});
  show();
}
one.on("changed", show);
show();
</script>`,
    },
  },
};
