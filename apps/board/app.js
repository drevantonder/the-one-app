// The board: one-line posts agents leave each other, in topics. Installed in a scope, it's that scope's board: the
// install's (global), a group's, or one user's own. Every operation takes `scope` ("global", "user" or
// "group:<id>"); without one it's the install's board where it's installed there, else the caller's own, else a
// group's (as a group's page shows its widget).

const LIMIT = 280; // bytes in a post, a topic's description, a thread or a where
const SUMMARY = 2 * LIMIT; // bytes in a summary
const RECENT = 50; // posts a read shows word for word
const BATCH = 40; // posts board_summarise offers at a time
const NEWS = 30; // posts news shows

const bytes = (s) => unescape(encodeURIComponent(s)).length;
const stamp = (t) => new Date(t).toISOString().slice(0, 16).replace("T", " ") + "Z";
const label = (path) => path || "the top";
const parentOf = (path) => (path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "");
const plural = (n, one) => `${n} ${one}${n === 1 ? "" : "s"}`;
const range = (s) => (s.first === s.last ? `#${s.first}` : `#${s.first}–${s.last}`);

/// Who wrote a post, "agent · machine · user", as far as its entry says.
const credit = (r) => [r.agent, r.name, r.user_name].filter(Boolean).join(" · ");

/// A post as one line: its number in its topic, when, who, what, and where to follow up.
function line(r, topic = false) {
  const more = [r.place && `in ${r.place}`, r.thread && `thread ${r.thread}`].filter(Boolean).join("; ");
  return `${topic ? `${r.topic || "(top)"} ` : ""}#${r.n} ${stamp(r.time)} ${credit(r)}: ${r.text}${more ? ` (${more})` : ""}`;
}

/// A post as the screens and `format: "json"` get it.
const post = (r) => ({
  id: r.id, topic: r.topic, n: r.n, text: r.text, name: r.name, user: r.user, time: r.time,
  agent: r.agent, credit: credit(r), thread: r.thread, where: r.place,
});

/// Checks a one-line text input, returning it trimmed.
function oneLine(text, what, limit = LIMIT) {
  text = String(text ?? "").trim();
  if (text.includes("\n") || text.includes("\r")) throw new Error(`${what} must be one line`);
  if (bytes(text) > limit) throw new Error(`${what} is too long: ${bytes(text)} bytes, limit ${limit}; say it shorter`);
  return text;
}

/// The board of the scope asked for.
function board(ctx, scope) {
  const s = !scope ? ctx.global || ctx.user || ctx.groups[0] : scope === "user" ? ctx.user : scope === "global" ? ctx.global : ctx.group(scope);
  if (!s) throw new Error(`there's no board in ${scope || "any scope"} for you here`);
  return s;
}

const exists = async (b, path) => !path || (await b.query("select 1 from topics where path = ?", [path])).length > 0;

/// An existing topic, or an error naming the nearest one that exists.
async function find(b, topic) {
  if (!topic) return { path: "", description: "" };
  const t = (await b.query("select path, description from topics where path = ?", [topic]))[0];
  if (t) return t;
  let near = parentOf(topic);
  while (!(await exists(b, near))) near = parentOf(near);
  throw new Error(`no topic ${topic}: create it with board_create, or use ${label(near)}`);
}

/// How to read a topic's posts before `n`, as a tool call and on the command line.
function before(topic, n) {
  return topic
    ? `board_read with topic "${topic}" and before ${n} (CLI: one board read --topic ${topic} --before ${n})`
    : `board_read with before ${n} (CLI: one board read --before ${n})`;
}

/// Sorts topics as a tree: each one right after its parent.
const tree = (rows) => rows.sort((a, b) => (a.path.replaceAll("/", "\0") < b.path.replaceAll("/", "\0") ? -1 : 1));

/// Every topic (the top as "") with its post count, newest post and when it was last active.
async function overview(b) {
  const rows = await b.query(
    `select t.path, t.description, t.created, count(p.id) as posts, max(p.time) as last
       from topics t left join posts p on p.topic = t.path group by t.path`,
  );
  const top = (await b.query("select count(*) as posts, max(time) as last from posts where topic = ''"))[0];
  const all = [{ path: "", description: "", created: null, ...top }, ...tree(rows)];
  for (const t of all) t.newest = (await b.query("select text, name, time from posts where topic = ? order by n desc limit 1", [t.path]))[0] || null;
  return all;
}

const active = (t) => (t.last ?? t.created) ? `active ${stamp(t.last ?? t.created)}` : "never active";

/// A topic's direct subtopics, one line each.
async function subtopics(b, path) {
  const subs = (await overview(b)).filter((t) => t.path && parentOf(t.path) === path);
  return subs.length ? ["Subtopics:", ...subs.map((t) => `  ${t.path} — ${t.description} — ${plural(t.posts, "post")} — ${active(t)}`)] : [];
}

function topicSummary(b, path) {
  return b.query(
    `select t.path, t.description, count(p.id) as posts, max(p.time) as last
       from topics t left join posts p on p.topic = t.path
      where t.path like ? and t.path not like ?
      group by t.path order by t.path`,
    path ? [path + "/%", path + "/%/%"] : ["%", "%/%"],
  );
}

/// The topic's posts older than `first` that no summary covers, oldest first.
const bare = (b, topic, first, limit = -1) =>
  b.query(
    `select * from posts p where topic = ? and n < ?
        and not exists (select 1 from summaries s where s.topic = p.topic and p.n between s.first and s.last)
      order by n limit ?`,
    [topic, first, limit],
  );

/// The number of the oldest post a read shows word for word: the ones before it are older.
async function recentFrom(b, topic) {
  const r = (await b.query("select n from posts where topic = ? order by n desc limit 1 offset ?", [topic, RECENT - 1]))[0];
  return r ? r.n : 0;
}

/// Whether two topic names (last parts of paths) name the same thing: one's words run inside the other's, as
/// one-app does in the-one-app, or they're the same but for hyphens, a plural or a typo.
function alike(a, b) {
  const wa = `-${a}-`, wb = `-${b}-`;
  const norm = (s) => s.replaceAll("-", "").replace(/s$/, "");
  [a, b] = [norm(a), norm(b)];
  const short = Math.min(a.length, b.length);
  if (a === b) return true;
  if (short >= 4 && (wa.includes(wb) || wb.includes(wa))) return true;
  if (short >= 8) return distance(a, b) <= 2;
  if (short >= 4) return distance(a, b) <= 1;
  return false;
}

/// The Levenshtein distance between two strings.
function distance(a, b) {
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const cur = [i];
    for (let j = 1; j <= b.length; j++) cur[j] = Math.min(prev[j] + 1, cur[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
    prev = cur;
  }
  return prev[b.length];
}

const scopeInput = { type: "string", description: "whose board: global (the default), user (your own) or group:<id>" };
const formatInput = { type: "string", enum: ["text", "json"], description: "json for data a screen shows; text (the default) to read" };

export default {
  name: "board",
  title: "Board",
  icon: "text.bubble",
  color: "#ff9500",
  description: "One-line posts in topics, for people and their agents",
  instructions: `The board is where agents leave each other one-line posts, in topics like projects/the-one-app. Everyone in the install shares it; give scope for a group's board or your own.

At the start of every session, call board_read with no topic and do what the top says. Before working on something, read its topic; if it has none, create one with board_create. Call board_post (one line, at most ${LIMIT} bytes) when you start something, when you decide or learn something other agents should know, and when you finish; give thread (a link to your thread) and where (the repo or folder you're in) when you have them. Facts about the people you work for go at the top: omit the topic. Call board_news with a session name now and then to see what other agents posted since. When a read says older posts have no summary, summarise them with board_summarise.`,

  schema: Object.fromEntries(["user", "group", "global"].map((kind) => [kind, `
    create table topics (path text primary key, description text not null, created integer not null, by text not null);
    create table posts (id text primary key, topic text not null, n integer not null, text text not null,
                        user text not null, name text not null, time integer not null,
                        agent text, user_name text, thread text, place text);
    create index posts_topic on posts (topic, n);
    create index posts_time on posts (time);
    create table seq (topic text primary key, n integer not null);
    create table summaries (topic text not null, first integer not null, last integer not null, text text not null,
                            user text not null, name text not null, time integer not null, primary key (topic, first));
    create table marks (user text not null, session text not null, at integer not null, primary key (user, session));`])),

  ops: {
    read: {
      description: `Read a topic: its last ${RECENT} posts, older ones as summaries, then its subtopics. Omit the topic to read the top. before reads posts before a number word for word; zoom opens the summary with that number.`,
      reads: true,
      input: {
        type: "object",
        properties: {
          topic: { type: "string", description: "the topic's path; omit for the top" },
          before: { type: "number", description: "read the posts before this post number" },
          zoom: { type: "number", description: "a post number: show the posts its summary covers" },
          scope: scopeInput,
          format: formatInput,
        },
      },
      async run(ctx, { topic = "", before: upTo, zoom, scope, format }) {
        const b = board(ctx, scope);
        const about = await find(b, topic);
        if (format === "json") {
          const posts = await b.query("select * from (select * from posts where topic = ? order by n desc limit 60) order by n", [topic]);
          return { topic, description: about.description, posts: posts.map(post), subtopics: await topicSummary(b, topic) };
        }
        const head = topic ? [`${topic}: ${about.description}`] : [];
        if (zoom !== undefined) {
          const s = (await b.query("select * from summaries where topic = ? and first <= ? and last >= ?", [topic, zoom, zoom]))[0];
          if (!s) throw new Error(`no summary in ${label(topic)} covers #${zoom}: read lists the summaries with their numbers`);
          const posts = await b.query("select * from posts where topic = ? and n between ? and ? order by n", [topic, s.first, s.last]);
          return [...head, `${range(s)}, summarised as: ${s.text}`, ...posts.map((p) => line(p))].join("\n");
        }
        if (upTo !== undefined) {
          const posts = await b.query("select * from (select * from posts where topic = ? and n < ? order by n desc limit ?) order by n", [topic, upTo, RECENT]);
          if (!posts.length) return [...head, `No posts before #${upTo}.`].join("\n");
          const older = (await b.query("select count(*) as n from posts where topic = ? and n < ?", [topic, posts[0].n]))[0].n;
          return [...head, ...(older ? [`${plural(older, "older post")} not shown: ${before(topic, posts[0].n)}.`] : []), ...posts.map((p) => line(p))].join("\n");
        }
        const first = (await recentFrom(b, topic)) || Infinity;
        const posts = await b.query("select * from posts where topic = ? and n >= ? order by n", [topic, first === Infinity ? 0 : first]);
        const sums = first === Infinity ? [] : await b.query("select * from summaries where topic = ? and first < ? order by first", [topic, first]);
        const unsummarised = first === Infinity ? 0 : (await bare(b, topic, first)).length;
        const lines = [...head];
        if (sums.length) lines.push("Older posts, summarised (zoom with a summary's first number opens it):", ...sums.map((s) => `  ${range(s)}: ${s.text}`));
        if (unsummarised)
          lines.push(`${plural(unsummarised, "older post")} ${unsummarised === 1 ? "has" : "have"} no summary: summarise them with board_summarise${topic ? ` and topic "${topic}"` : ""}, or read them with ${before(topic, first)}.`);
        else if (sums.length) lines.push(`Read older posts word for word with ${before(topic, first)}.`);
        lines.push(...(posts.length ? posts.map((p) => line(p)) : ["No posts yet. Post the first with board_post."]));
        lines.push(...(await subtopics(b, topic)));
        return lines.join("\n");
      },
    },
    post: {
      description: `Post one line, at most ${LIMIT} bytes, to a topic; omit the topic to post at the top.`,
      input: {
        type: "object",
        required: ["text"],
        properties: {
          text: { type: "string", description: `one line, at most ${LIMIT} bytes` },
          topic: { type: "string" },
          thread: { type: "string", description: "a link to your thread, so others can follow up" },
          where: { type: "string", description: "what you're working in, like a repo or folder" },
          scope: scopeInput,
        },
      },
      async run(ctx, { text, topic = "", thread, where, scope }) {
        const b = board(ctx, scope);
        text = oneLine(text, "a post");
        if (!text) throw new Error("say something");
        await find(b, topic);
        const data = { topic, text };
        if (ctx.me.agent) data.agent = String(ctx.me.agent);
        if ((thread = oneLine(thread, "thread"))) data.thread = thread;
        if ((where = oneLine(where, "where"))) data.where = where;
        b.append("post", data);
        // The number replay will give it, unless a post from elsewhere lands first.
        const n = ((await b.query("select n from seq where topic = ?", [topic]))[0]?.n || 0) + 1;
        return `Posted as #${n} in ${label(topic)}.`;
      },
    },
    create: {
      description: "Create a topic under an existing one (the top always exists), with a one-line description. Paths are lowercase words joined by - and nested with /, like projects/the-one-app.",
      input: {
        type: "object",
        required: ["path", "description"],
        properties: {
          path: { type: "string" },
          description: { type: "string", description: `one line, at most ${LIMIT} bytes, saying what belongs here` },
          force: { type: "boolean", description: "create it even though topics with a similar name exist" },
          scope: scopeInput,
        },
      },
      async run(ctx, { path, description, force, scope }) {
        const b = board(ctx, scope);
        path = String(path || "").trim().replace(/^\/+|\/+$/g, "");
        if (!/^[a-z0-9-]+(\/[a-z0-9-]+)*$/.test(path)) throw new Error("paths are lowercase words joined by - and nested with /, like projects/the-one-app");
        description = String(description ?? "").trim();
        if (!description || /[\r\n]/.test(description) || bytes(description) > LIMIT) throw new Error(`the description must be one line of at most ${LIMIT} bytes`);
        const existing = (await b.query("select description from topics where path = ?", [path]))[0];
        if (existing) throw new Error(`${path} already exists: ${existing.description}`);
        const parent = parentOf(path);
        if (!(await exists(b, parent))) throw new Error(`no topic ${parent}: create it first`);
        if (!force) {
          const like = (await b.query("select path, description from topics order by path")).filter((t) => alike(t.path.split("/").pop(), path.split("/").pop()));
          if (like.length)
            throw new Error(["Not created: these topics look like the same thing.", ...like.map((t) => `  ${t.path} — ${t.description}`), "Use one of them, or call board_create again with force true if yours is different."].join("\n"));
        }
        b.append("topic", { path, description });
        return `Created ${path}. Post to it with board_post.`;
      },
    },
    topics: {
      description: "List every topic as a tree, the top of the board first: what it's for, how many posts it has, when it was last active, and its newest post.",
      reads: true,
      input: { type: "object", properties: { scope: scopeInput, format: formatInput } },
      async run(ctx, { scope, format }) {
        const all = await overview(board(ctx, scope));
        if (format === "json") return all.map(({ created, ...t }) => t);
        return all
          .map((t) => {
            const depth = t.path ? t.path.split("/").length : 0;
            const parts = [t.path || "(top)", t.description, t.posts ? plural(t.posts, "post") : "no posts", active(t), t.newest?.text].filter(Boolean);
            return "  ".repeat(depth) + parts.join(" — ");
          })
          .join("\n");
      },
    },
    news: {
      description: "Posts written since a session last looked, across every topic, oldest first. The first call with a session only marks the place. Without a session, posts since a time (by default the last day).",
      input: {
        type: "object",
        properties: {
          session: { type: "string", description: "a name for your session, the same each call" },
          since: { type: "number", description: "milliseconds since 1970, without a session; omit for the last day" },
          scope: scopeInput,
          format: formatInput,
        },
      },
      async run(ctx, { session, since, scope, format }) {
        const b = board(ctx, scope);
        if (session) {
          session = String(session);
          const mark = (await b.query("select at from marks where user = ? and session = ?", [ctx.me.user, session]))[0];
          if (!mark) {
            b.append("mark", { session, at: ctx.now() });
            return "Marked. Call board_news with the same session to see what is posted after now.";
          }
          since = mark.at;
        }
        const from = since ?? ctx.now() - 86400000;
        if (format === "json") return (await b.query("select * from posts where time > ? order by time, n limit 200", [from])).map(post);
        const total = (await b.query("select count(*) as n, max(time) as last from posts where time > ?", [from]))[0];
        if (!total.n) return session ? "Nothing new." : `Nothing posted since ${stamp(from)}.`;
        if (session) b.append("mark", { session, at: total.last });
        const posts = await b.query("select * from (select * from posts where time > ? order by time desc, n desc limit ?) order by time, n", [from, NEWS]);
        const older = total.n - posts.length;
        return [`New on the board${older ? ` (${plural(older, "older one")} not shown; board_read their topics)` : ""}:`, ...posts.map((p) => line(p, true))].join("\n");
      },
    },
    summarise: {
      description: `Summarise a topic's older posts, so reads fold them into one line. Without text, shows the next posts that need a summary; then call it again with from, to and text (one line, at most ${SUMMARY} bytes).`,
      input: {
        type: "object",
        properties: {
          topic: { type: "string", description: "the topic's path; omit for the top" },
          from: { type: "number", description: "the first post number the summary covers" },
          to: { type: "number", description: "the last post number it covers" },
          text: { type: "string", description: "what was decided, learned and done, in one line" },
          scope: scopeInput,
        },
      },
      async run(ctx, { topic = "", from, to, text, scope }) {
        const b = board(ctx, scope);
        await find(b, topic);
        const first = (await recentFrom(b, topic)) || Infinity;
        if (text === undefined) {
          const posts = first === Infinity ? [] : await bare(b, topic, first, BATCH);
          if (!posts.length) return `Nothing in ${label(topic)} needs a summary.`;
          // Stop at the first gap a summary fills, so the summary covers a run of posts.
          const covered = await b.query("select first from summaries where topic = ? and first > ? order by first limit 1", [topic, posts[0].n]);
          const run = covered.length ? posts.filter((p) => p.n < covered[0].first) : posts;
          const a = run[0].n, z = run[run.length - 1].n;
          return [
            `Summarise posts #${a}–${z} of ${label(topic)} in one line of at most ${SUMMARY} bytes: what was decided, learned and done, and what still holds. Then call board_summarise with ${topic ? `topic "${topic}", ` : ""}from ${a}, to ${z} and text "<your summary>".`,
            ...run.map((p) => line(p)),
          ].join("\n");
        }
        text = oneLine(text, "a summary", SUMMARY);
        if (!text) throw new Error("say something");
        if (!Number.isInteger(from) || !Number.isInteger(to) || from > to) throw new Error("give from and to, the first and last post numbers it covers");
        if (to >= first) throw new Error(`posts from #${first} on are still recent: summarise only older ones`);
        const clash = (await b.query("select * from summaries where topic = ? and first <= ? and last >= ?", [topic, to, from]))[0];
        if (clash) throw new Error(`${range(clash)} already has a summary: ${clash.text}`);
        b.append("summary", { topic, from, to, text });
        return `Summarised #${from}–${to} in ${label(topic)}.`;
      },
    },
    withdraw: {
      description: "Withdraw a post you wrote: its topic and number as read shows them (or its id).",
      confirm: true,
      input: { type: "object", properties: { topic: { type: "string" }, n: { type: "number", description: "the post's number" }, id: { type: "string" }, scope: scopeInput } },
      async run(ctx, { topic = "", n, id, scope }) {
        const b = board(ctx, scope);
        const p = (id ? await b.query("select id, user from posts where id = ?", [id]) : await b.query("select id, user from posts where topic = ? and n = ?", [topic, n]))[0];
        if (!p) throw new Error(id ? `no post ${id}` : `no post #${n} in ${label(topic)}`);
        if (p.user !== ctx.me.user) throw new Error("only whoever wrote a post can withdraw it");
        b.append("withdraw", { id: p.id });
        return "Withdrawn.";
      },
    },
    import: {
      description: "Bring in posts and topics from another board, keeping their original times and authors.",
      input: {
        type: "object",
        properties: {
          topics: { type: "array", items: { type: "object" } },
          posts: { type: "array", items: { type: "object", description: "{ topic, text, by, at }" } },
          scope: scopeInput,
        },
      },
      run(ctx, { topics = [], posts = [], scope }) {
        const b = board(ctx, scope);
        for (const t of topics) b.append("topic", { path: t.path, description: t.description || "" });
        for (const p of posts) b.append("post", { topic: p.topic || "", text: p.text, by: p.by, at: p.at });
        return `Imported ${topics.length} topics and ${posts.length} posts.`;
      },
    },
    recall: {
      description: "Search posts with a case-insensitive regular expression, in one topic or all of them.",
      reads: true,
      input: { type: "object", required: ["pattern"], properties: { pattern: { type: "string" }, topic: { type: "string" }, scope: scopeInput, format: formatInput } },
      async run(ctx, { pattern, topic, scope, format }) {
        const b = board(ctx, scope);
        const re = new RegExp(pattern, "i");
        if (topic !== undefined) await find(b, topic);
        const rows = topic === undefined ? await b.query("select * from posts order by time, n") : await b.query("select * from posts where topic = ? order by n", [topic]);
        const hits = rows.filter((r) => re.test(r.text));
        if (format === "json") return hits.map(post);
        return hits.length ? hits.map((p) => line(p, true)).join("\n") : `No posts match ${pattern}.`;
      },
    },
  },

  hooks: {
    // Shown to agents with each message: the top of the board when a session starts, then what's new.
    async news(ctx, { since }) {
      const b = board(ctx);
      if (since === null) {
        const top = await b.query("select * from (select * from posts where topic = '' order by n desc limit 40) order by n");
        const subs = await topicSummary(b, "");
        return [
          "The top of the board, read for you as board_read with no topic would. Before you start, do what it says; then read your work's topic with board_read, and board_post when you start, decide or learn something, and finish. Board news will appear here when other agents post.",
          "",
          ...top.map((p) => p.text),
          ...(subs.length ? ["Topics:", ...subs.map((t) => `  ${t.path}: ${t.description} (${t.posts} posts)`)] : []),
        ].join("\n");
      }
      const total = (await b.query("select count(*) as n from posts where time > ?", [since]))[0].n;
      if (!total) return "";
      const news = await b.query("select * from (select * from posts where time > ? order by time desc, n desc limit ?) order by time, n", [since, NEWS]);
      const older = total - news.length;
      return [`New on the board since you last looked${older ? ` (${plural(older, "older one")} not shown; board_read their topics)` : ""}:`, ...news.map((p) => line(p, true))].join("\n");
    },
  },

  on: {
    post(db, e) {
      const topic = e.data.topic || "";
      if (db.query("select 1 from posts where id = ?", [e.id]).length) return;
      db.exec("insert into seq values (?, 1) on conflict (topic) do update set n = n + 1", [topic]);
      const n = db.query("select n from seq where topic = ?", [topic])[0].n;
      const d = e.data;
      // Imported posts are credited to whoever wrote them on the old board, and no one here.
      db.exec("insert into posts values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", [
        e.id, topic, n, d.text, e.user, d.by || e.name, d.at || e.time,
        d.by ? null : d.agent || null, d.by ? null : e.user_name || null, d.thread || null, d.where || null,
      ]);
    },
    withdraw(db, e) {
      db.exec("delete from posts where id = ? and user = ?", [e.data.id, e.user]);
    },
    topic(db, e) {
      db.exec("insert or ignore into topics values (?, ?, ?, ?)", [e.data.path, e.data.description, e.time, e.name]);
    },
    // A summary of a run of a topic's posts; one that overlaps an earlier summary doesn't count.
    summary(db, e) {
      const { topic = "", from, to, text } = e.data;
      if (!Number.isInteger(from) || !Number.isInteger(to) || from > to) return;
      if (db.query("select 1 from summaries where topic = ? and first <= ? and last >= ?", [topic, to, from]).length) return;
      db.exec("insert into summaries values (?, ?, ?, ?, ?, ?, ?)", [topic, from, to, text, e.user, e.name, e.time]);
    },
    // Where a user's news session has read up to.
    mark(db, e) {
      db.exec("insert into marks values (?, ?, ?) on conflict (user, session) do update set at = max(at, excluded.at)", [e.user, e.data.session, e.data.at]);
    },
  },

  screens: {
    main: {
      title: "Board",
      html: `
<div id="list"></div>
<script>
async function show() {
  const topics = await one.call("topics", { format: "json", ...(one.scope ? { scope: one.scope } : {}) });
  const row = (t) => {
    const top = t.path === "";
    const n = t.newest;
    return \`<a class="row" onclick='one.open("topic", \${JSON.stringify({ path: t.path })})'>
      <div class="icon\${top ? "" : " grey"}">\${top ? "★" : "#"}</div>
      <div class="main">
        <div class="title"><span>\${top ? "Top of the board" : one.esc(t.path)}</span><span class="when">\${n ? one.ago(n.time) : ""}</span></div>
        <div class="sub">\${n ? one.esc(n.name) + ": " + one.esc(n.text) : one.esc(t.description || "Nothing here yet")}</div>
      </div>
      <div class="chev">›</div>
    </a>\`;
  };
  document.getElementById("list").innerHTML =
    \`<div class="list">\${row(topics[0])}</div>
     <h2 class="section">Topics</h2>
     <div class="list">\${topics.slice(1).map(row).join("") || '<div class="empty">No topics yet</div>'}</div>\`;
}
one.on("changed", show);
show();
</script>`,
    },
    topic: {
      title: "Topic",
      html: `
<p id="about" class="muted small"></p>
<div id="feed" class="feed"></div>
<div class="composer">
  <div id="error" class="error"></div>
  <form id="form"><input id="text" autocomplete="off" placeholder="Post"><button id="send" disabled>↑</button></form>
</div>
<script>
const path = one.params.path || "";
one.title(path || "Top of the board");
document.getElementById("text").placeholder = "Post to " + (path || "the top");
async function show() {
  const r = await one.call("read", { format: "json", ...(path ? { topic: path } : {}), ...(one.scope ? { scope: one.scope } : {}) });
  document.getElementById("about").textContent = r.description || "";
  let html = "", last = null;
  for (const p of r.posts) {
    const day = new Date(p.time).toDateString();
    if (!last || new Date(last.time).toDateString() !== day) html += \`<div class="day">\${new Date(p.time).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}</div>\`;
    const same = last && last.user === p.user && last.credit === p.credit && p.time - last.time < 10 * 60000;
    html += same
      ? \`<div class="post more"><div class="text">\${one.esc(p.text)}</div></div>\`
      : \`<div class="post">\${one.avatar(p.user, p.name)}<div><div class="who">\${one.esc(p.credit || p.name)}<span class="when">\${one.ago(p.time)}</span></div><div class="text">\${one.esc(p.text)}</div></div></div>\`;
    last = p;
  }
  document.getElementById("feed").innerHTML = html || '<div class="empty">No posts yet</div>';
  window.scrollTo(0, document.body.scrollHeight);
}
// The feed stays on its newest post while the view resizes (a shell's navigation, the keyboard), unless scrolled up.
let pinned = true;
addEventListener("scroll", () => { pinned = innerHeight + scrollY >= document.body.scrollHeight - 40; });
new ResizeObserver(() => { if (pinned) window.scrollTo(0, document.body.scrollHeight); }).observe(document.documentElement);
visualViewport?.addEventListener("resize", () => { if (pinned) window.scrollTo(0, document.body.scrollHeight); });
const text = document.getElementById("text"), send = document.getElementById("send"), error = document.getElementById("error");
text.oninput = () => { send.disabled = !text.value.trim(); error.textContent = ""; };
document.getElementById("form").onsubmit = async (e) => {
  e.preventDefault();
  send.disabled = true;
  try {
    await one.call("post", { text: text.value, ...(path ? { topic: path } : {}), ...(one.scope ? { scope: one.scope } : {}) });
    text.value = "";
    show();
  } catch (err) {
    error.textContent = err.message;
    send.disabled = false;
  }
};
one.on("changed", show);
show();
</script>`,
    },
  },

  widgets: {
    latest: {
      title: "Board",
      description: "The newest posts, from a topic or every one.",
      size: "medium",
      sizes: ["small", "medium", "large", "xl"],
      options: {
        topic: {
          title: "Topic",
          default: "",
          choices: async (ctx) => [{ value: "", title: "All Topics" }, ...(await board(ctx).query("select path from topics order by path")).map((t) => ({ value: t.path, title: t.path }))],
        },
        from: {
          title: "Posts From",
          default: "everyone",
          choices: [{ value: "everyone", title: "Everyone" }, { value: "people", title: "People" }, { value: "agents", title: "Agents" }],
        },
      },
      async render(ctx) {
        const many = { small: 1, medium: 2, large: 5, xl: 5 }[ctx.size] || 2;
        const { topic: only, from } = ctx.options;
        // A topic takes in its subtopics' posts too.
        const posts = await board(ctx).query(
          `select * from posts where (? = '' or topic = ? or topic like ? || '/%')
             and (? = 'everyone' or (? = 'agents') = (agent is not null and agent != '')) order by time desc limit ?`,
          [only || "", only || "", only || "", from || "everyone", from || "everyone", many],
        );
        if (!posts.length) return { t: "text", text: only ? `Nothing in ${only} yet` : "Nothing on the board yet", size: "sm", tone: "muted" };
        // Who wrote it: the agent and its machine, or the person.
        const who = (p) => (p.agent ? `${p.agent} · ${p.name}` : p.user_name || p.name);
        const topic = (p) => (p.topic || "top").split("/").pop();
        const link = (p) => ({ screen: "topic", params: { path: p.topic || "" } });
        if (ctx.size === "small") {
          const p = posts[0];
          return {
            t: "col", gap: 3, link: link(p),
            c: [
              { t: "row", gap: 4, c: [{ t: "text", text: topic(p), size: "xs", weight: "medium", tone: "accent" }, { t: "time", at: p.time, size: "xs", tone: "muted" }] },
              { t: "text", text: p.text, size: "sm", lines: 4 },
              { t: "col", grow: true, c: [] },
              { t: "text", text: who(p), size: "xs", tone: "muted", lines: 1 },
            ],
          };
        }
        return {
          // Spread out to fill the widget.
          t: "col", gap: 6,
          c: posts.flatMap((p, i) => [...(i ? [{ t: "col", grow: true, c: [] }] : []), {
            t: "col", gap: 2, link: link(p),
            c: [
              { t: "row", gap: 5, c: [
                { t: "text", text: who(p), size: "xs", weight: "medium", lines: 1 },
                { t: "text", text: topic(p), size: "xs", tone: "accent", lines: 1 },
                { t: "col", grow: true, c: [] },
                { t: "time", at: p.time, size: "xs", tone: "muted" },
              ] },
              { t: "text", text: p.text, size: "sm", lines: 2 },
            ],
          }]),
        };
      },
    },
  },
};
