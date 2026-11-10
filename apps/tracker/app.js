// What likely needs doing, each a thing of its own in a user's, a group's or everyone's own list. An item points at
// nothing: tags say what it's about (app:gmail, build:434d139, error, feedback, friction).
//
// How a failure becomes an item, every few minutes, from the `insights` read (what broke, never a caller's mistake):
// - One item per app, error type and top frame, whichever build it's seen in, its line numbers left out as they move
//   between builds. A failure counts once for each trace, however many spans of it there are.
// - It lands where the app that failed is deployed: its owner's list, or everyone's for an admins' app or the network's
//   own program. Tracker has to be added there, with a device running its schedule.
// - A failure seen only once opens nothing, nor do those all within a minute of each other, as everything in flight at
//   a restart fails together: it opens on happening again a minute or more after it first did.
// - One marked Done comes back to the Inbox if it happens in a build other than the one it last happened in, and says
//   so in a comment. Dropped stays dropped. Warnings aren't read at all.
import main from "./dist/main.html" with { type: "text" };

const STATUSES = ["inbox", "next", "doing", "done", "dropped"];
const SAME_MOMENT = 60_000;
// How far behind the last read each read looks, for what reaches telemetry late or a slot missed.
const OVERLAP = 30 * 60_000;
const OLDEST = 7 * 86_400_000;
const SPANS = 2000;
const TRACES_KEPT = 5;
const MOST_LISTED = 500;
const MOST_ATTACHED = 25 << 20;

const TABLES = `
create table items (id text primary key, title text not null, body text not null, status text not null, by text, at integer not null, updated integer not null);
create index items_updated on items (status, updated);
create table tags (item text not null, tag text not null, primary key (item, tag));
create index tags_tag on tags (tag);
create table comments (id text primary key, item text not null, by text, body text not null, at integer not null);
create index comments_item on comments (item, at);
create table attachments (id text primary key, item text not null, name text not null, type text not null, size integer not null, blob text not null, by text, at integer not null);
create index attachments_item on attachments (item, at);
create table errors (key text primary key, item text, service text not null, tool text not null, type text not null, frame text not null, message text, count integer not null, first integer not null, last integer not null, first_build text not null, last_build text not null, traces text not null, closed_build text, closed_at integer);
create index errors_item on errors (item);
create table seen (trace text primary key, at integer not null);
create table state (name text primary key, value integer not null);
`;

const refusal = (message) => Object.assign(new Error(message), { code: "refused" });
const newId = () => crypto.randomUUID().slice(0, 8);
// A comma is never part of a tag, and a command line's `--tags a,b` reaches here as one string.
const cleaned = (tags) => [...new Set(tags.flatMap((t) => String(t).split(",")).map((t) => t.trim().toLowerCase().replace(/\s+/g, "-")).filter(Boolean))];
const places = (ctx) => [ctx.user, ctx.global, ...ctx.groups].filter(Boolean);
const named = (ctx, s) => (s === ctx.user ? "You" : s === ctx.global ? "Everyone" : ctx.handles.find((h) => h.id === s.id)?.name || s.name);
const whoIs = (ctx, user) => (user ? ctx.handles.find((h) => h.id === `user:${user}`)?.name || "Someone" : "Tracker");

function place(ctx, scope = "user") {
  const found = scope === "user" ? ctx.user : scope === "global" ? ctx.global : ctx.groups.find((s) => s.id === scope || ctx.handles.some((h) => h.id === s.id && (h.handle === scope || h.name === scope)));
  if (!found) throw refusal(`can't reach ${scope}: give user, global or a group's handle from groups_list, one where Tracker is added`);
  return found;
}

async function find(ctx, id) {
  for (const s of places(ctx)) {
    const [item] = await s.query("select * from items where id = ?", [id]);
    if (item) return { s, item };
  }
  throw refusal(`no item ${id}: ids come from tracker_list`);
}

const marks = (list) => list.map(() => "?").join(", ");
const row = (ctx, s, i, tags, count, error) => ({
  id: i.id, title: i.title, status: i.status, tags, scope: s === ctx.user ? "user" : s.id, scope_name: named(ctx, s), by: whoIs(ctx, i.by), at: i.at, updated: i.updated, comments: count,
  ...(error ? { times: error.count, last_seen: error.last } : {}),
});

/// One place's items by the filters, newest change first, each with its tags, comments and how often it failed.
async function listed(ctx, s, { status, tags, q, limit }) {
  const where = [], params = [];
  if (status) (where.push("status = ?"), params.push(status));
  for (const t of tags) (where.push("id in (select item from tags where tag = ?)"), params.push(t));
  if (q) (where.push("(title like ? escape '\\' or body like ? escape '\\')"), params.push(...Array(2).fill(`%${q.replace(/[\\%_]/g, "\\$&")}%`)));
  const clause = where.length ? `where ${where.join(" and ")}` : "";
  const items = await s.query(`select * from items ${clause} order by updated desc limit ?`, [...params, limit]);
  const ids = items.map((i) => i.id);
  if (!ids.length) return [];
  const [tagRows, commentRows, errorRows] = await Promise.all([
    s.query(`select item, tag from tags where item in (${marks(ids)}) order by tag`, ids),
    s.query(`select item, count(*) n from comments where item in (${marks(ids)}) group by item`, ids),
    s.query(`select item, sum(count) count, max(last) last from errors where item in (${marks(ids)}) group by item`, ids),
  ]);
  const tagsOf = Map.groupBy(tagRows, (t) => t.item);
  const comments = new Map(commentRows.map((c) => [c.item, c.n]));
  const errors = new Map(errorRows.map((e) => [e.item, e]));
  return items.map((i) => row(ctx, s, i, (tagsOf.get(i.id) || []).map((t) => t.tag), comments.get(i.id) || 0, errors.get(i.id)));
}

/// How many there are at each status, by the same filters but the status.
async function counted(s, { tags, q }) {
  const where = [], params = [];
  for (const t of tags) (where.push("id in (select item from tags where tag = ?)"), params.push(t));
  if (q) (where.push("(title like ? escape '\\' or body like ? escape '\\')"), params.push(...Array(2).fill(`%${q.replace(/[\\%_]/g, "\\$&")}%`)));
  return s.query(`select status, count(*) n from items ${where.length ? `where ${where.join(" and ")}` : ""} group by status`, params);
}

function addItem(ctx, s, { title, body = "", status = "inbox", tags = [] }, by = ctx.me.user) {
  const id = newId(), now = ctx.now();
  s.exec("insert into items values (?, ?, ?, ?, ?, ?, ?)", [id, title, body, status, by, now, now]);
  for (const t of cleaned(tags)) s.exec("insert or ignore into tags values (?, ?)", [id, t]);
  return id;
}

function touch(ctx, s, id) {
  s.exec("update items set updated = ? where id = ?", [ctx.now(), id]);
}

function setStatus(ctx, s, id, status) {
  s.exec("update items set status = ?, updated = ? where id = ?", [status, ctx.now(), id]);
  // Done is what a later build's failure is measured against; anything else isn't waiting on a fix.
  if (status === "done") s.exec("update errors set closed_build = last_build, closed_at = ? where item = ?", [ctx.now(), id]);
  else s.exec("update errors set closed_build = null, closed_at = null where item = ?", [id]);
}

const span = (r) => {
  const print = r.attrs?.["one.fingerprint"];
  if (!print || r.status !== "error" || !r.time) return null;
  const a = print.indexOf("|"), b = print.lastIndexOf("|");
  if (a < 0 || b <= a) return null;
  const [type, frame] = [print.slice(0, a), print.slice(a + 1, b).replace(/:\d+(:\d+)?$/, "")];
  return { key: `${r.service}/${type}/${frame}`, type, frame, build: print.slice(b + 1), service: r.service, tool: r.name, trace: r.trace || r.span, time: r.time, message: r.message || null };
};

const buildTag = (build) => `build:${build.slice(0, 12)}`;
const opens = (e) => e.count >= 2 && e.last - e.first >= SAME_MOMENT;

/// Turns the failures of what's deployed in `s` that it hasn't seen yet into its items.
async function takeFailures(ctx, s, spans) {
  const now = ctx.now();
  const [errorRows, seenRows] = await Promise.all([s.query("select * from errors"), s.query("select trace from seen")]);
  const errors = new Map(errorRows.map((e) => [e.key, { ...e, traces: JSON.parse(e.traces), was: true }]));
  const seen = new Set(seenRows.map((x) => x.trace));
  const touched = new Set();
  const newBuilds = new Map();
  const cameBack = new Map();
  for (const f of spans.sort((a, b) => a.time - b.time)) {
    const once = `${f.trace}/${f.key}`;
    if (seen.has(once)) continue;
    seen.add(once);
    s.exec("insert or ignore into seen values (?, ?)", [once, now]);
    let e = errors.get(f.key);
    if (!e) {
      e = { key: f.key, item: null, service: f.service, tool: f.tool, type: f.type, frame: f.frame, message: f.message, count: 0, first: f.time, last: f.time, first_build: f.build, last_build: f.build, traces: [], closed_build: null, closed_at: null };
      errors.set(f.key, e);
    }
    if (e.closed_at && f.time > e.closed_at && f.build !== e.closed_build && !cameBack.has(f.key)) cameBack.set(f.key, { was: e.closed_build, now: f.build });
    e.count += 1;
    e.first = Math.min(e.first, f.time);
    if (f.time >= e.last) Object.assign(e, { last: f.time, last_build: f.build, tool: f.tool });
    if (f.message) e.message = f.message;
    e.traces = [f.trace, ...e.traces.filter((t) => t !== f.trace)].slice(0, TRACES_KEPT);
    newBuilds.set(f.key, new Set([...(newBuilds.get(f.key) || []), f.build]));
    touched.add(f.key);
  }
  for (const key of touched) {
    const e = errors.get(key);
    if (!e.item && opens(e)) {
      const label = e.message || `${e.type} at ${e.frame}`;
      e.item = addItem(ctx, s, {
        title: `${e.service}: ${label}`.slice(0, 160),
        tags: ["error", `app:${e.service}`, `fingerprint:${key}`, ...[...(newBuilds.get(key) || [])].map(buildTag)],
      }, null);
    } else if (e.item) {
      for (const b of newBuilds.get(key) || []) s.exec("insert or ignore into tags values (?, ?)", [e.item, buildTag(b)]);
      const back = cameBack.get(key);
      if (back) {
        setStatus(ctx, s, e.item, "inbox");
        s.exec("insert or ignore into tags values (?, 'regression')", [e.item]);
        s.exec("insert into comments values (?, ?, null, ?, ?)", [newId(), e.item, `Came back in build ${back.now.slice(0, 12)}, after it was marked Done with build ${back.was.slice(0, 12)}.`, now]);
        e.closed_build = e.closed_at = null;
      } else touch(ctx, s, e.item);
    }
    s.exec(e.was ? "update errors set item = ?, tool = ?, message = ?, count = ?, first = ?, last = ?, last_build = ?, traces = ?, closed_build = ?, closed_at = ? where key = ?" : "insert into errors (item, tool, message, count, first, last, last_build, traces, closed_build, closed_at, key, service, type, frame, first_build) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
      [e.item, e.tool, e.message, e.count, e.first, e.last, e.last_build, JSON.stringify(e.traces), e.closed_build, e.closed_at, key, ...(e.was ? [] : [e.service, e.type, e.frame, e.first_build])]);
  }
  s.exec("delete from seen where at < ?", [now - OLDEST]);
  s.exec("insert or replace into state values ('read', ?)", [now]);
}

const scopeInput = { type: "string", description: "whose list: user (yours, the default), global (everyone's) or a group's handle from groups_list" };
const STATUS = { type: "string", enum: STATUSES, description: "inbox, next, doing, done or dropped" };
const ID = { type: "string", description: "an item's id, from tracker_list" };
const TAGS = { type: "array", items: { type: "string" }, description: "tags, exact: app:gmail, build:434d139, error, feedback, friction. tracker_tags lists those in use, so use one of them where one fits" };
const ITEM = {
  type: "object",
  required: ["id", "title", "status", "tags", "scope", "scope_name", "by", "at", "updated", "comments"],
  properties: {
    id: { type: "string" }, title: { type: "string" }, status: STATUS, tags: { type: "array", items: { type: "string" } },
    scope: { type: "string" }, scope_name: { type: "string" }, by: { type: "string" },
    at: { type: "integer", description: "when it was made, in milliseconds" }, updated: { type: "integer", description: "its last change, in milliseconds" },
    comments: { type: "integer" }, times: { type: "integer", description: "an error's: how often it has happened" }, last_seen: { type: "integer", description: "an error's: when it last did, in milliseconds" },
  },
};
const DONE = { type: "object", required: ["id"], properties: { id: { type: "string" } } };

export default {
  title: "Tracker",
  icon: "tray",
  color: "#5e5ce6",
  description: "What likely needs doing: errors, feedback and tasks, yours, a group's and everyone's",
  instructions: "Tracker holds what likely needs doing, as items with a status and tags, in each user's, group's and everyone's own list. Agents pick up work here: tracker_list finds it by status and tag, tracker_read gives everything to start with, and tracker_comment and tracker_set_status say what happened. Real errors of apps and of the network open items by themselves, tagged error and app:<name>, once something has failed again a minute or more after it first did. Use tracker_add for what you find that someone should see and tracker_friction for what slowed you down.",
  schema: { user: [TABLES], group: [TABLES], global: [TABLES] },
  schedules: { failures: { tool: "_read_failures", every: "5m", description: "Turns the network's failures into items" } },
  tools: {
    list: {
      title: "List items",
      reads: true,
      description: "Items in every list you can reach, or one, newest change first. Each gives { id, title, status, tags, scope, by, at, updated, comments }, and an error's times and last_seen. tracker_read gives one in full. Gives { items, counts, more }: counts is how many there are at each status by the same tags and words.",
      input: { type: "object", properties: {
        status: STATUS,
        tags: { ...TAGS, description: "only items with all of these, exact" },
        q: { type: "string", description: "only items with these words in their title or body" },
        scope: { type: "string", description: "only this list: user, global or a group's handle; every one you can reach by default" },
        limit: { type: "integer", description: `how many, at most ${MOST_LISTED}; 100 by default` },
      } },
      output: { type: "object", required: ["items", "counts", "more", "scopes"], properties: {
        items: { type: "array", items: ITEM },
        counts: { type: "object", additionalProperties: { type: "integer" }, description: "items at each status, by the same tags and words" },
        more: { type: "boolean" },
        scopes: { type: "array", items: { type: "object", required: ["scope", "name"], properties: { scope: { type: "string" }, name: { type: "string" } } } },
      } },
      run: async (ctx, { status, tags = [], q = "", scope, limit = 100 }) => {
        const most = Math.min(Math.max(1, limit), MOST_LISTED);
        const wanted = { status, tags: cleaned(tags), q: q.trim(), limit: most + 1 };
        const asked = scope ? [place(ctx, scope)] : places(ctx);
        const [lists, counts] = await Promise.all([Promise.all(asked.map((s) => listed(ctx, s, wanted))), Promise.all(asked.map((s) => counted(s, wanted)))]);
        const items = lists.flat().sort((a, b) => b.updated - a.updated);
        const total = Object.fromEntries(STATUSES.map((st) => [st, 0]));
        for (const c of counts.flat()) total[c.status] = (total[c.status] || 0) + c.n;
        return { items: items.slice(0, most), counts: total, more: items.length > most, scopes: places(ctx).map((s) => ({ scope: s === ctx.user ? "user" : s === ctx.global ? "global" : s.id, name: named(ctx, s) })) };
      },
    },
    read: {
      title: "Read an item",
      reads: true,
      description: "An item in full: its body, tags, comments and attachments, and for an error, what it said, how often, since which build, its traces and how to dig in.",
      input: { type: "object", required: ["id"], properties: { id: ID } },
      output: { type: "object", required: ["id", "title", "body", "status", "tags", "scope", "scope_name", "by", "at", "updated", "comments", "attachments"], properties: {
        id: { type: "string" }, title: { type: "string" }, body: { type: "string" }, status: STATUS, tags: { type: "array", items: { type: "string" } },
        scope: { type: "string" }, scope_name: { type: "string" }, by: { type: "string" }, at: { type: "integer" }, updated: { type: "integer" },
        comments: { type: "array", items: { type: "object", required: ["id", "by", "body", "at"], properties: { id: { type: "string" }, by: { type: "string" }, body: { type: "string" }, at: { type: "integer" } } } },
        attachments: { type: "array", items: { type: "object", required: ["id", "name", "type", "size", "by", "at"], properties: { id: { type: "string" }, name: { type: "string" }, type: { type: "string" }, size: { type: "integer" }, by: { type: "string" }, at: { type: "integer" } } } },
        error: { type: "object", description: "if it's a failure", required: ["app", "tool", "type", "frame", "times", "first_seen", "last_seen", "first_build", "last_build", "traces", "dig"], properties: {
          app: { type: "string" }, tool: { type: "string" }, type: { type: "string" }, frame: { type: "string" }, message: { type: ["string", "null"] },
          times: { type: "integer" }, first_seen: { type: "integer" }, last_seen: { type: "integer" }, first_build: { type: "string" }, last_build: { type: "string" },
          traces: { type: "array", items: { type: "string" }, description: "the newest failing calls' trace ids" },
          dig: { type: "array", items: { type: "string" }, description: "what to read next, as the insights tool's input" },
        } },
      } },
      run: async (ctx, { id }) => {
        const { s, item: i } = await find(ctx, id);
        const [tags, comments, attachments, [error]] = await Promise.all([
          s.query("select tag from tags where item = ? order by tag", [id]),
          s.query("select id, by, body, at from comments where item = ? order by at", [id]),
          s.query("select id, name, type, size, by, at from attachments where item = ? order by at", [id]),
          s.query("select * from errors where item = ?", [id]),
        ]);
        const traces = error ? JSON.parse(error.traces) : [];
        return {
          id: i.id, title: i.title, body: i.body, status: i.status, tags: tags.map((t) => t.tag), scope: s === ctx.user ? "user" : s.id, scope_name: named(ctx, s), by: whoIs(ctx, i.by), at: i.at, updated: i.updated,
          comments: comments.map((c) => ({ ...c, by: whoIs(ctx, c.by) })),
          attachments: attachments.map((a) => ({ ...a, by: whoIs(ctx, a.by) })),
          ...(error ? { error: { app: error.service, tool: error.tool, type: error.type, frame: error.frame, message: error.message, times: error.count, first_seen: error.first, last_seen: error.last, first_build: error.first_build, last_build: error.last_build, traces, dig: [...traces.slice(0, 1).map((t) => `insights --trace-id ${t}`), `insights --service ${error.service} --since 7d`] } } : {}),
        };
      },
    },
    add: {
      title: "Add an item",
      reads: false,
      description: "Adds an item to a list's Inbox, for something someone should see or do. Gives { id }. For something that slowed you down building The One App, use tracker_friction.",
      input: { type: "object", required: ["title"], properties: {
        title: { type: "string", description: "what it is, in a line" }, body: { type: "string", description: "everything needed to start on it, as markdown" },
        tags: TAGS, scope: scopeInput, status: STATUS,
      } },
      output: DONE,
      run: async (ctx, { title, body, tags, scope, status }) => {
        if (!title.trim()) throw refusal("an item needs a title");
        return { id: addItem(ctx, place(ctx, scope), { title: title.trim(), body, tags, status }) };
      },
    },
    friction: {
      title: "Note friction",
      reads: false,
      description: "Notes something that slowed you down or confused you in building The One App (an error that didn't say what to do, a command you had to piece together, a wait, a doc that was wrong), as an item tagged friction in everyone's list. The same line twice stays one item. Gives { id, added }.",
      input: { type: "object", required: ["text"], properties: {
        text: { type: "string", description: "what happened, in a line" }, body: { type: "string", description: "more, if there is more" }, scope: { type: "string", description: "whose list, as tracker_add takes it; global by default" },
      } },
      output: { type: "object", required: ["id", "added"], properties: { id: { type: "string" }, added: { type: "boolean" } } },
      run: async (ctx, { text, body, scope = "global" }) => {
        const title = text.replace(/^\s*friction\s*:\s*/i, "").trim();
        if (!title) throw refusal("friction needs a line saying what happened");
        const s = place(ctx, scope);
        const [was] = await s.query("select id from items where title = ? and id in (select item from tags where tag = 'friction')", [title]);
        return was ? { id: was.id, added: false } : { id: addItem(ctx, s, { title, body, tags: ["friction"] }), added: true };
      },
    },
    comment: {
      title: "Comment on an item",
      reads: false,
      description: "Adds a comment to an item: what you found, tried or changed. Gives { id }.",
      input: { type: "object", required: ["id", "body"], properties: { id: ID, body: { type: "string", description: "the comment, as markdown" } } },
      output: DONE,
      run: async (ctx, { id, body }) => {
        if (!body.trim()) throw refusal("a comment needs some text");
        const { s } = await find(ctx, id);
        const made = newId();
        s.exec("insert into comments values (?, ?, ?, ?, ?)", [made, id, ctx.me.user, body, ctx.now()]);
        touch(ctx, s, id);
        return { id: made };
      },
    },
    set_status: {
      title: "Move an item",
      reads: false,
      description: "Sets an item's status. Mark an error done once its fix has shipped: it comes back to the Inbox by itself if it happens in a later build. Gives { id }.",
      input: { type: "object", required: ["id", "status"], properties: { id: ID, status: STATUS } },
      output: DONE,
      run: async (ctx, { id, status }) => {
        const { s } = await find(ctx, id);
        setStatus(ctx, s, id, status);
        return { id };
      },
    },
    tag: {
      title: "Tag an item",
      reads: false,
      description: "Adds and takes off an item's tags. Gives { id }.",
      input: { type: "object", required: ["id"], properties: { id: ID, add: TAGS, remove: { ...TAGS, description: "the tags to take off, exact" } } },
      output: DONE,
      run: async (ctx, { id, add = [], remove = [] }) => {
        const { s } = await find(ctx, id);
        for (const t of cleaned(remove)) s.exec("delete from tags where item = ? and tag = ?", [id, t]);
        for (const t of cleaned(add)) s.exec("insert or ignore into tags values (?, ?)", [id, t]);
        touch(ctx, s, id);
        return { id };
      },
    },
    tags: {
      title: "Tags in use",
      reads: true,
      description: "The tags in use, most used first, each { tag, count }, so an item gets one that exists rather than a second spelling.",
      input: { type: "object", properties: { scope: { type: "string", description: "only this list, as tracker_list takes it" } } },
      output: { type: "array", items: { type: "object", required: ["tag", "count"], properties: { tag: { type: "string" }, count: { type: "integer" } } } },
      run: async (ctx, { scope }) => {
        const rows = (await Promise.all((scope ? [place(ctx, scope)] : places(ctx)).map((s) => s.query("select tag, count(*) n from tags where tag not like 'fingerprint:%' group by tag")))).flat();
        const total = new Map();
        for (const r of rows) total.set(r.tag, (total.get(r.tag) || 0) + r.n);
        return [...total].map(([tag, count]) => ({ tag, count })).sort((a, b) => b.count - a.count || a.tag.localeCompare(b.tag));
      },
    },
    rename_app: {
      title: "Move items to an app's new name",
      reads: false,
      description: "Rewrites every item about an app under its new name, after the app is renamed: its app: tag, and its errors, so the failures that keep coming under the new name stay the same items. Gives { items }, how many changed.",
      input: { type: "object", required: ["from", "to"], properties: { from: { type: "string", description: "the app's old name" }, to: { type: "string", description: "its new name" } } },
      output: { type: "object", required: ["items"], properties: { items: { type: "integer" } } },
      run: async (ctx, { from, to }) => {
        let items = 0;
        for (const s of places(ctx)) {
          const [tagged, errors] = await Promise.all([s.query("select item from tags where tag = ?", [`app:${from}`]), s.query("select * from errors where service = ?", [from])]);
          items += tagged.length;
          s.exec("update tags set tag = ? where tag = ?", [`app:${to}`, `app:${from}`]);
          for (const e of errors) {
            const key = `${to}/${e.type}/${e.frame}`;
            s.exec("update errors set key = ?, service = ? where key = ?", [key, to, e.key]);
            if (e.item) s.exec("update tags set tag = ? where item = ? and tag = ?", [`fingerprint:${key}`, e.item, `fingerprint:${e.key}`]);
          }
        }
        return { items };
      },
    },
    attach: {
      title: "Attach a file",
      reads: false,
      description: "Attaches a file to an item, up to 25 MB. Gives { id }, which tracker_attachment reads back.",
      input: { type: "object", required: ["id", "name", "base64"], properties: { id: ID, name: { type: "string", description: "its file name" }, type: { type: "string", description: "its media type, like image/png" }, base64: { type: "string", description: "its bytes, base64" } } },
      output: DONE,
      run: async (ctx, { id, name, type = "application/octet-stream", base64 }) => {
        const { s } = await find(ctx, id);
        const bytes = Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
        if (bytes.length > MOST_ATTACHED) throw refusal("an attachment may be 25 MB at most");
        const blob = await ctx.blob.put(bytes);
        const made = newId();
        s.exec("insert into attachments values (?, ?, ?, ?, ?, ?, ?, ?)", [made, id, name, type, bytes.length, blob, ctx.me.user, ctx.now()], [blob]);
        touch(ctx, s, id);
        return { id: made };
      },
    },
    attachment: {
      title: "Read an attachment",
      reads: true,
      description: "An attachment's file, as { name, type, base64 }.",
      input: { type: "object", required: ["id"], properties: { id: { type: "string", description: "an attachment's id, from tracker_read" } } },
      output: { type: "object", required: ["name", "type", "base64"], properties: { name: { type: "string" }, type: { type: "string" }, base64: { type: "string" } } },
      run: async (ctx, { id }) => {
        for (const s of places(ctx)) {
          const [a] = await s.query("select name, type, blob from attachments where id = ?", [id]);
          if (a) {
            const bytes = await ctx.blob.get(a.blob);
            let text = "";
            for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
            return { name: a.name, type: a.type, base64: btoa(text) };
          }
        }
        throw refusal(`no attachment ${id}: ids come from tracker_read`);
      },
    },
    _read_failures: {
      title: "Read failures (for its schedule)",
      reads: false,
      description: "For its schedule: turns the network's failures since the last read into items, each in the list of whoever owns the app that failed.",
      input: { type: "object", properties: {} },
      output: { type: "object", required: ["read", "failures"], properties: { read: { type: "integer" }, failures: { type: "integer" } } },
      run: async (ctx) => {
        const now = ctx.now();
        const [catalog, reads] = await Promise.all([ctx.catalog(), Promise.all(places(ctx).map((s) => s.query("select value from state where name = 'read'")))]);
        const lastRead = Math.min(...reads.map(([r]) => r?.value ?? 0));
        const back = Math.min(now - lastRead + OVERLAP, OLDEST);
        const { spans = [] } = await ctx.insights({ raw: true, status: "error", since: `${Math.ceil(back / 1000)}s`, limit: SPANS });
        const failures = spans.map(span).filter(Boolean);
        const owners = new Map(catalog.map((a) => [a.name, a.owner]));
        const whose = (service) => { const o = owners.get(service); return !o || o === "admins" ? "global" : o; };
        await Promise.all(places(ctx).map((s) => takeFailures(ctx, s, failures.filter((f) => whose(f.service) === s.id))));
        return { read: spans.length, failures: failures.length };
      },
    },
  },
  screens: {
    main: { title: "Tracker", html: main },
  },
};
