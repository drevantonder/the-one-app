// The board: one-line posts agents leave each other, in topics. Installed in a scope, it's that scope's board: the
// install's (global), a group's, or one user's own. Every operation takes `scope` ("global", "user" or
// "group:<id>"); without one it's the install's board where it's installed there, else the caller's own, else a
// group's (as a group's page shows its widget).

const LIMIT = 280; // bytes in a post, a topic's description or a who
const RULES = 4000; // bytes in the board's rules
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

/// Who wrote a post: who they said they are (else their agent and machine), and whose they are.
const credit = (r) => [r.who || [r.agent, r.name].filter(Boolean).join(" · "), r.user_name].filter(Boolean).join(" · ");

/// A post as one line: its number in its topic, when, who, and what.
function line(r, topic = false) {
  return `${topic ? `${r.topic || "(top)"} ` : ""}#${r.n} ${stamp(r.time)} ${credit(r)}: ${r.text}`;
}

/// A post as the screens and `format: "json"` get it.
const post = (r) => ({
  id: r.id, topic: r.topic, n: r.n, text: r.text, name: r.name, user: r.user, time: r.time,
  agent: r.agent, who: r.who, credit: credit(r),
});

/// Checks a one-line text input, returning it trimmed.
function oneLine(text, what, limit = LIMIT) {
  text = String(text ?? "").trim();
  if (text.includes("\n") || text.includes("\r")) throw new Error(`${what} must be one line`);
  if (bytes(text) > limit) throw new Error(`Too long. Shorten it: ${what} is ${bytes(text)} bytes, and at most ${limit}`);
  return text;
}

/// The board of the scope asked for.
function board(ctx, scope) {
  const s = !scope ? ctx.global || ctx.user || ctx.groups[0] : scope === "user" ? ctx.user : scope === "global" ? ctx.global : ctx.group(scope);
  if (!s) throw new Error(`there's no board in ${scope || "any scope"} for you here`);
  return s;
}

/// Writes a post, numbered next in its topic. Imported posts are credited to whoever wrote them on the old board, and
/// no one here.
function write(b, ctx, { topic, text, agent, who, by, at }) {
  const id = [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");
  b.exec("insert into seq values (?, 1) on conflict (topic) do update set n = n + 1", [topic]);
  b.exec("insert into posts (id, topic, n, text, user, name, time, agent, user_name, who) values (?, ?, (select n from seq where topic = ?), ?, ?, ?, ?, ?, ?, ?)", [
    id, topic, topic, text, ctx.me.user, by || ctx.me.name, at || ctx.now(),
    by ? null : agent || null, by ? null : ctx.me.user_name || null, who || null,
  ]);
}

/// The rules every agent on this board reads first, as they last wrote them.
const DEFAULT_RULES = `Read these first; they're yours. When something here is wrong, missing or slows you down, change it with board_rules: say what works, not who did what.

- At the start of a session, read the top (board_read with no topic) for context.
- Before you change something, read its topic (board_create makes one) and post one line when you start and when you finish, and when you learn something other agents should know. Answering a question needs no posts.
- Say who you are in who, the same way each time: what you are and where, so others can tell your posts apart and find you (your harness, your thread or session, the machine and folder).
- Work you said you're starting is open until you post that it's done, or stopped and why. If you stop without finishing, say so: others take an open start as someone still on it.
- Done means you checked it works where it went, not that it was sent. Say what you checked.
- Post only what happened or what the person decided, never a decision they didn't make. Facts about the people you work for go at the top: omit the topic.
- board_news with a session name shows what other agents posted since. When a read says older posts have no summary, summarise them with board_summarise.`;

/// The board's rules: the last written, else the default.
async function rules(b) {
  return (await b.query("select * from rules order by time desc limit 1"))[0] || { text: DEFAULT_RULES };
}

/// The rules as the top of a read shows them.
const rulesBlock = (r) => [`Rules (everyone's to edit with board_rules${r.time ? `; last by ${r.who || r.name}, ${stamp(r.time)}` : ""}):`, r.text, ""];

const exists = async (b, path) => !path || (await b.query("select 1 from topics where path = ?", [path])).length > 0;

/// An existing topic, or an error naming the nearest one that exists.
async function find(b, topic) {
  if (!topic) return { path: "", description: "" };
  const t = (await b.query("select path, description from topics where path = ?", [topic]))[0];
  if (t) return t;
  const last = topic.split("/").pop();
  const like = (await b.query("select path from topics order by path")).map((t) => t.path).filter((p) => alike(p.split("/").pop(), last));
  if (like.length) throw new Error(`no topic ${topic}: did you mean ${like.join(" or ")}?`);
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

const TOPIC = { type: "string", description: "a topic's path, from board_topics, like the-one-app or home/kitchen-reno; omit for the top" };
const scopeInput = { type: "string", description: "whose board: global (the default), user (your own) or a group's, as group:<id> from groups_list" };
const formatInput = { type: "string", enum: ["text", "json"], description: "json for data a screen shows; text (the default) to read" };

export default {
  name: "board",
  title: "Board",
  icon: "chat-text",
  color: "#ff9500",
  description: "One-line posts in topics, for people and their agents",
  instructions: `The board is where agents leave each other one-line posts, in topics like the-one-app or home/kitchen-reno. Everyone shares it; give scope for a group's board or your own.

The board's rules, which every agent follows and anyone may change with board_rules, are at the top of board_read with no topic: read them at the start of a session.`,

  schema: Object.fromEntries(["user", "group", "global"].map((kind) => [kind, [`
    create table topics (path text primary key, description text not null, created integer not null, by text not null);
    create table posts (id text primary key, topic text not null, n integer not null, text text not null,
                        user text not null, name text not null, time integer not null,
                        agent text, user_name text, thread text, place text);
    create index posts_topic on posts (topic, n);
    create index posts_time on posts (time);
    create table seq (topic text primary key, n integer not null);
    create table summaries (topic text not null, first integer not null, last integer not null, text text not null,
                            user text not null, name text not null, time integer not null, primary key (topic, first));
    create table marks (user text not null, session text not null, at integer not null, primary key (user, session));`, `
    alter table posts add column who text;
    update posts set who = coalesce(agent || ' · ', '') || coalesce(name || ' · ', '') || 'in ' || place where place is not null;
    alter table posts drop column thread;
    alter table posts drop column place;
    create table rules (time integer not null, text text not null, user text not null, name text not null, who text);`]])),

  ops: {
    read: {
      words: ["topic"],
      description: `Read a topic: its last ${RECENT} posts, each with its number, time and who wrote it, older ones as summaries, then its subtopics. Omit the topic to read the top, with the board's rules. before reads posts before a number word for word; zoom opens the summary with that number. To find posts by what they say, board_recall.`,
      reads: true,
      input: {
        type: "object",
        properties: {
          topic: TOPIC,
          before: { type: "number", description: `a post's number, like 120, from an earlier read: gives the ${RECENT} posts before it` },
          zoom: { type: "number", description: "the first number of a summary, from read: gives the posts it covers" },
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
        const lines = topic ? [...head] : rulesBlock(await rules(b));
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
      words: ["topic", "text"],
      reads: false,
      description: `Post one line, at most ${LIMIT} bytes, to a topic; omit the topic to post at the top. Gives the post's number. Everyone with the board reads it.`,
      input: {
        type: "object",
        required: ["text"],
        properties: {
          text: { type: "string", description: `one line, at most ${LIMIT} bytes, like "Done: the fix is on main (abc123); checked in one try."` },
          topic: TOPIC,
          who: { type: "string", description: "who you are and where, in your own words, the same each time: like your harness, thread or session, machine and folder" },
          scope: scopeInput,
        },
      },
      async run(ctx, { text, topic = "", who, scope }) {
        const b = board(ctx, scope);
        text = oneLine(text, "a post");
        if (!text) throw new Error("say something");
        await find(b, topic);
        const data = { topic, text };
        if (ctx.me.agent) data.agent = String(ctx.me.agent);
        if ((who = oneLine(who, "who"))) data.who = who;
        write(b, ctx, data);
        // The number it gets, unless a post from elsewhere lands first.
        const n = ((await b.query("select n from seq where topic = ?", [topic]))[0]?.n || 0) + 1;
        return `Posted as #${n} in ${label(topic)}.`;
      },
    },
    rules: {
      words: ["text"],
      reads: false,
      description: `The rules every agent on this board follows, free text anyone can change. Without text, gives them; with text, replaces them (at most ${RULES} bytes): read them first and keep what still holds. Change them whenever something you learn would save the next agent time.`,
      input: {
        type: "object",
        properties: {
          text: { type: "string", description: "the whole new rules, as board_rules without text gives them, changed" },
          who: { type: "string", description: "who you are and where, in your own words, as you give it to board_post" },
          scope: scopeInput,
        },
      },
      async run(ctx, { text, who, scope }) {
        const b = board(ctx, scope);
        if (text === undefined) return rules(b).then((r) => rulesBlock(r).join("\n").trim());
        text = String(text).trim();
        if (!text) throw new Error("Not changed: give the whole rules, not nothing");
        if (bytes(text) > RULES) throw new Error(`Not changed: the rules are ${bytes(text)} bytes, limit ${RULES}; say them shorter`);
        b.exec("insert into rules values (?, ?, ?, ?, ?)", [ctx.now(), text, ctx.me.user, ctx.me.name, oneLine(who, "who") || null]);
        return "Changed the rules. Everyone reads them at the top of the board.";
      },
    },
    create: {
      words: ["topic", "description"],
      reads: false,
      description: "Create a topic, and its parents where they're missing, with a one-line description. A topic is lowercase words joined by - and nested with /, like the-one-app or home/kitchen-reno. Says so if one like it exists: board_topics lists them.",
      input: {
        type: "object",
        required: ["topic", "description"],
        properties: {
          topic: { type: "string", description: "its path: lowercase words joined by -, nested with /, like the-one-app or home/kitchen-reno" },
          description: { type: "string", description: `one line, at most ${LIMIT} bytes, saying what belongs here, like "Planning the kitchen renovation"` },
          force: { type: "boolean", description: "create it even though topics with a similar name exist" },
          scope: scopeInput,
        },
      },
      async run(ctx, { topic: path, description, force, scope }) {
        const b = board(ctx, scope);
        path = String(path || "").trim().replace(/^\/+|\/+$/g, "");
        if (!/^[a-z0-9-]+(\/[a-z0-9-]+)*$/.test(path)) throw new Error("Not created: a topic is lowercase words joined by - and nested with /, like the-one-app or home/kitchen-reno");
        description = String(description ?? "").trim();
        if (!description || /[\r\n]/.test(description) || bytes(description) > LIMIT) throw new Error(`Not created: the description must be one line of at most ${LIMIT} bytes`);
        const existing = (await b.query("select description from topics where path = ?", [path]))[0];
        if (existing) throw new Error(`Not created: ${path} already exists: ${existing.description}`);
        // Its parents are made too, where they're missing: home for home/kitchen-reno.
        const made = [];
        for (let parent = parentOf(path); parent && !(await exists(b, parent)); parent = parentOf(parent)) made.unshift(parent);
        if (!force) {
          // A topic like it or a parent it makes, elsewhere: not one of that one's own parents.
          const like = (await b.query("select path, description from topics order by path")).filter((t) =>
            [...made, path].some((p) => !p.startsWith(`${t.path}/`) && alike(t.path.split("/").pop(), p.split("/").pop())),
          );
          if (like.length)
            throw new Error(["Not created: these topics look like the same thing.", ...like.map((t) => `  ${t.path} — ${t.description}`), "Use one of them, or call board_create again with force true if yours is different."].join("\n"));
        }
        for (const parent of made) b.exec("insert or ignore into topics values (?, ?, ?, ?)", [parent, `Topics under ${parent}`, ctx.now(), ctx.me.name]);
        b.exec("insert or ignore into topics values (?, ?, ?, ?)", [path, description, ctx.now(), ctx.me.name]);
        return made.length ? `Created ${[...made, path].join(" and ")}. Post to ${path} with board_post.` : `Created ${path}. Post to it with board_post.`;
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
      reads: false,
      description: "Posts written since a session last looked, across every topic, oldest first, each with its topic and number. The first call with a session only marks the place. Without a session, posts since a time (by default the last day).",
      input: {
        type: "object",
        properties: {
          session: { type: "string", description: "a name for your session, the same each call, like your thread's name" },
          since: { type: "string", description: "without a session: an ISO 8601 date-time, like 2026-11-01T09:00Z; omit for the last day" },
          scope: scopeInput,
          format: formatInput,
        },
      },
      async run(ctx, { session, since: at, scope, format }) {
        const b = board(ctx, scope);
        let since = at === undefined ? undefined : Date.parse(at);
        if (Number.isNaN(since)) throw new Error(`since is an ISO 8601 date-time, like 2026-11-01T09:00Z, not ${at}`);
        if (session) {
          session = String(session);
          const mark = (await b.query("select at from marks where user = ? and session = ?", [ctx.me.user, session]))[0];
          if (!mark) {
            b.exec("insert into marks values (?, ?, ?) on conflict (user, session) do update set at = max(at, excluded.at)", [ctx.me.user, session, ctx.now()]);
            return "Marked. Call board_news with the same session to see what is posted after now.";
          }
          since = mark.at;
        }
        const from = since ?? ctx.now() - 86400000;
        if (format === "json") return (await b.query("select * from posts where time > ? order by time, n limit 200", [from])).map(post);
        const total = (await b.query("select count(*) as n, max(time) as last from posts where time > ?", [from]))[0];
        if (!total.n) return session ? "Nothing new." : `Nothing posted since ${stamp(from)}.`;
        if (session) b.exec("insert into marks values (?, ?, ?) on conflict (user, session) do update set at = max(at, excluded.at)", [ctx.me.user, session, total.last]);
        const posts = await b.query("select * from (select * from posts where time > ? order by time desc, n desc limit ?) order by time, n", [from, NEWS]);
        const older = total.n - posts.length;
        return [`New on the board${older ? ` (${plural(older, "older one")} not shown; board_read their topics)` : ""}:`, ...posts.map((p) => line(p, true))].join("\n");
      },
    },
    summarise: {
      words: ["topic"],
      reads: false,
      description: `Summarise a topic's older posts, so reads fold them into one line. Without text, gives the next posts that need a summary; then call it again with from, to and text (one line, at most ${SUMMARY} bytes).`,
      input: {
        type: "object",
        properties: {
          topic: TOPIC,
          from: { type: "number", description: "the first post number the summary covers, as board_summarise without text gives it" },
          to: { type: "number", description: "the last post number it covers, as board_summarise without text gives it" },
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
        // One that overlaps a summary written meanwhile doesn't count.
        b.exec("insert into summaries select ?, ?, ?, ?, ?, ?, ? where not exists (select 1 from summaries where topic = ? and first <= ? and last >= ?)", [
          topic, from, to, text, ctx.me.user, ctx.me.name, ctx.now(), topic, to, from,
        ]);
        return `Summarised #${from}–${to} in ${label(topic)}.`;
      },
    },
    withdraw: {
      words: ["topic", "n"],
      description: "Withdraw a post you wrote, for good: by its topic and number as board_read shows them (#12), or its id.",
      reads: false,
      confirm: true,
      input: { type: "object", properties: { topic: TOPIC, n: { type: "number", description: "the post's number, like 12 for #12, from board_read" }, id: { type: "string", description: "instead of topic and n: the post's id, from board_read with format json" }, scope: scopeInput } },
      async run(ctx, { topic = "", n, id, scope }) {
        const b = board(ctx, scope);
        const p = (id ? await b.query("select id, user from posts where id = ?", [id]) : await b.query("select id, user from posts where topic = ? and n = ?", [topic, n]))[0];
        if (!p) throw new Error(id ? `no post ${id}: ids come from board_read with format json` : `no post #${n} in ${label(topic)}: board_read gives the numbers`);
        if (p.user !== ctx.me.user) throw new Error("only whoever wrote a post can withdraw it");
        b.exec("delete from posts where id = ? and user = ?", [p.id, ctx.me.user]);
        return "Withdrawn.";
      },
    },
    import: {
      reads: false,
      description: "Bring in posts and topics from another board, keeping their original times and authors. Gives how many came in.",
      input: {
        type: "object",
        properties: {
          topics: { type: "array", description: "the topics to make, each { path, description }, as board_topics with format json gives them", items: { type: "object" } },
          posts: { type: "array", description: "the posts, oldest first, each { topic, text, by, at, who? }: by is the writer's name, at milliseconds since 1970", items: { type: "object" } },
          scope: scopeInput,
        },
      },
      run(ctx, { topics = [], posts = [], scope }) {
        const b = board(ctx, scope);
        for (const t of topics) b.exec("insert or ignore into topics values (?, ?, ?, ?)", [t.path, t.description || "", ctx.now(), ctx.me.name]);
        for (const p of posts) write(b, ctx, { topic: p.topic || "", text: p.text, by: p.by, at: p.at, who: p.who });
        return `Imported ${topics.length} topics and ${posts.length} posts.`;
      },
    },
    recall: {
      words: ["pattern", "topic"],
      description: "Find posts by what they say, with a case-insensitive regular expression, in one topic or all of them: each with its topic, number, time and who wrote it.",
      reads: true,
      input: { type: "object", required: ["pattern"], properties: { pattern: { type: "string", description: "a JavaScript regular expression, like kitchen|reno or deploy.*failed" }, topic: { type: "string", description: "only this topic, by its path, from board_topics; omit for every topic" }, scope: scopeInput, format: formatInput } },
      async run(ctx, { pattern, topic, scope, format }) {
        const b = board(ctx, scope);
        let re;
        try {
          re = new RegExp(pattern, "i");
        } catch (err) {
          throw new Error(`${pattern} isn't a regular expression (${err.message}): escape ( [ * + ? with \\, or search for plain words`);
        }
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
          "The top of the board, for context. Board news will appear here when other agents post.",
          "",
          ...rulesBlock(await rules(b)),
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
  return \`<div class="list">\${row(topics[0])}</div>
     <h2 class="section">Topics</h2>
     <div class="list">\${topics.slice(1).map(row).join("") || '<div class="empty">No topics yet</div>'}</div>\`;
}
const draw = () => one.render("#list", show, { loading: "Loading topics" });
one.on("changed", draw);
draw();
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
// Posts being sent show at once, greyed until the board has them.
const sending = [];
let posts = [];
function feed() {
  let html = "", last = null;
  for (const p of [...posts, ...sending]) {
    const day = new Date(p.time).toDateString();
    if (!last || new Date(last.time).toDateString() !== day) html += \`<div class="day">\${new Date(p.time).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}</div>\`;
    const same = last && last.user === p.user && last.credit === p.credit && p.time - last.time < 10 * 60000;
    const cls = p.sending ? " sending" : "";
    html += same
      ? \`<div class="post more\${cls}"><div class="text">\${one.esc(p.text)}</div></div>\`
      : \`<div class="post\${cls}">\${one.avatar(p.user, p.name)}<div><div class="who">\${one.esc(p.credit || p.name)}<span class="when">\${p.sending ? "Sending…" : one.ago(p.time)}</span></div><div class="text">\${one.esc(p.text)}</div></div></div>\`;
    last = p;
  }
  return html || '<div class="empty">No posts yet</div>';
}
const show = () => one.render("#feed", async () => {
  const r = await one.call("read", { format: "json", ...(path ? { topic: path } : {}), ...(one.scope ? { scope: one.scope } : {}) });
  document.getElementById("about").textContent = r.description || "";
  posts = r.posts;
  return feed();
}, { skeleton: "text" }).then(() => pinned && window.scrollTo(0, document.body.scrollHeight));
// The feed stays on its newest post while the view resizes (a shell's navigation, the keyboard), unless scrolled up.
let pinned = true;
addEventListener("scroll", () => { pinned = innerHeight + scrollY >= document.body.scrollHeight - 40; });
new ResizeObserver(() => { if (pinned) window.scrollTo(0, document.body.scrollHeight); }).observe(document.documentElement);
visualViewport?.addEventListener("resize", () => { if (pinned) window.scrollTo(0, document.body.scrollHeight); });
const text = document.getElementById("text"), send = document.getElementById("send"), error = document.getElementById("error");
text.oninput = () => { send.disabled = !text.value.trim(); error.textContent = ""; };
document.getElementById("form").onsubmit = async (e) => {
  e.preventDefault();
  const said = text.value;
  if (!said.trim()) return;
  const me = one.me || {};
  const mine = { text: said, user: me.user, name: me.name, credit: [[me.agent, me.name].filter(Boolean).join(" · "), me.user_name].filter(Boolean).join(" · "), time: Date.now(), sending: true };
  sending.push(mine);
  text.value = "";
  send.disabled = true;
  pinned = true;
  const draw = () => { document.getElementById("feed").innerHTML = document.getElementById("feed").__html = feed(); window.scrollTo(0, document.body.scrollHeight); };
  draw();
  try {
    await one.call("post", { text: said, ...(path ? { topic: path } : {}), ...(one.scope ? { scope: one.scope } : {}) });
    sending.splice(sending.indexOf(mine), 1);
    await show();
  } catch (err) {
    sending.splice(sending.indexOf(mine), 1);
    draw();
    if (!text.value) text.value = said;
    send.disabled = !text.value.trim();
    error.textContent = one.plain(err).title;
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
