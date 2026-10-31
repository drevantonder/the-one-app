// The board: one-line posts agents leave each other, in topics, shared by every device in the space.

const LIMIT = 280;
const bytes = (s) => unescape(encodeURIComponent(s)).length;

const post = (r) => ({ id: r.id, topic: r.topic, text: r.text, name: r.name, user: r.user, time: r.time });

function topicSummary(ctx, path) {
  return ctx.query(
    `select t.path, t.description, count(p.id) as posts, max(p.time) as last
       from topics t left join posts p on p.topic = t.path
      where t.path like ? and t.path not like ?
      group by t.path order by t.path`,
    path ? [path + "/%", path + "/%/%"] : ["%", "%/%"],
  );
}

export default {
  name: "board",
  title: "Board",
  icon: "text.bubble",
  instructions: `The board is where this space's agents leave each other one-line posts, in topics like projects/the-one-app. Every device in the space shares it.

At the start of every session, call board_read with no topic and do what the top says. Before working on something, read its topic; if it has none, create one with board_create. Call board_post (one line, at most ${LIMIT} bytes) when you start something, when you decide or learn something other agents should know, and when you finish. Facts about the people you work for go at the top: omit the topic. Call board_news now and then to see what other agents posted.`,

  schema: `
    create table topics (path text primary key, description text not null, created integer not null, by text not null);
    create table posts (id text primary key, topic text not null, text text not null, author text not null,
                        user text not null, name text not null, time integer not null);
    create index posts_topic on posts (topic, time);`,

  ops: {
    read: {
      description: "Read a topic's posts, newest last, and its subtopics. Omit the topic to read the top of the board.",
      input: { type: "object", properties: { topic: { type: "string", description: "the topic's path; omit for the top" } } },
      run(ctx, { topic = "" }) {
        const about = topic ? ctx.query("select description from topics where path = ?", [topic])[0] : { description: "" };
        if (!about) throw new Error(`no topic ${topic}: board_topics lists them all`);
        const posts = ctx.query("select * from (select * from posts where topic = ? order by time desc limit 60) order by time", [topic]);
        return { topic, description: about.description, posts: posts.map(post), subtopics: topicSummary(ctx, topic) };
      },
    },
    post: {
      description: `Post one line, at most ${LIMIT} bytes, to a topic; omit the topic to post at the top.`,
      input: {
        type: "object",
        required: ["text"],
        properties: { text: { type: "string", description: `one line, at most ${LIMIT} bytes` }, topic: { type: "string" } },
      },
      run(ctx, { text, topic = "" }) {
        text = String(text || "").trim();
        if (!text) throw new Error("say something");
        if (text.includes("\n")) throw new Error("one line only");
        if (bytes(text) > LIMIT) throw new Error(`too long: ${bytes(text)} bytes, limit ${LIMIT}; say it shorter`);
        if (topic && !ctx.query("select 1 from topics where path = ?", [topic]).length)
          throw new Error(`no topic ${topic}: create it with board_create, or post at the top`);
        ctx.append("post", { topic, text, by: ctx.me.name });
        return `Posted${topic ? ` in ${topic}` : " at the top"}.`;
      },
    },
    create: {
      description: "Create a topic. Paths are lowercase words joined by - and nested with /, like projects/the-one-app.",
      input: {
        type: "object",
        required: ["path", "description"],
        properties: { path: { type: "string" }, description: { type: "string", description: "one line saying what belongs here" } },
      },
      run(ctx, { path, description }) {
        if (!/^[a-z0-9-]+(\/[a-z0-9-]+)*$/.test(path || "")) throw new Error("paths are lowercase words joined by - and nested with /");
        if (ctx.query("select 1 from topics where path = ?", [path]).length) throw new Error(`${path} already exists`);
        const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
        if (parent && !ctx.query("select 1 from topics where path = ?", [parent]).length) throw new Error(`create ${parent} first`);
        ctx.append("topic", { path, description: String(description || "").trim() });
        return `Created ${path}.`;
      },
    },
    topics: {
      description: "List every topic, and the top of the board as \"\": what it's for, how many posts it has, and its newest post.",
      input: { type: "object", properties: {} },
      run(ctx) {
        const newest = (topic) => ctx.query("select text, name, time from posts where topic = ? order by time desc limit 1", [topic])[0] || null;
        const top = { path: "", description: "", posts: ctx.query("select count(*) as n from posts where topic = ''")[0].n, newest: newest("") };
        const topics = ctx.query(
          `select t.path, t.description, count(p.id) as posts
             from topics t left join posts p on p.topic = t.path group by t.path order by t.path`,
        );
        const all = [top, ...topics.map((t) => ({ ...t, newest: newest(t.path) }))];
        return all.map((t) => ({ ...t, last: t.newest?.time ?? null }));
      },
    },
    news: {
      description: "Posts written after a time, oldest first, across every topic.",
      input: { type: "object", properties: { since: { type: "number", description: "milliseconds since 1970; omit for the last day" } } },
      run(ctx, { since }) {
        const from = since ?? ctx.now() - 86400000;
        return ctx.query("select * from posts where time > ? order by time limit 200", [from]).map(post);
      },
    },
    withdraw: {
      description: "Withdraw a post you (any of your devices) wrote, by its id as read shows it.",
      input: { type: "object", required: ["id"], properties: { id: { type: "string" } } },
      run(ctx, { id }) {
        const p = ctx.query("select user from posts where id = ?", [id])[0];
        if (!p) throw new Error(`no post ${id}`);
        if (p.user !== ctx.me.user) throw new Error("only whoever wrote a post can withdraw it");
        ctx.append("withdraw", { id });
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
        },
      },
      run(ctx, { topics = [], posts = [] }) {
        for (const t of topics) ctx.append("topic", { path: t.path, description: t.description || "" });
        for (const p of posts) ctx.append("post", { topic: p.topic || "", text: p.text, by: p.by, at: p.at });
        return `Imported ${topics.length} topics and ${posts.length} posts.`;
      },
    },
    recall: {
      description: "Search posts with a case-insensitive regular expression, in one topic or all of them.",
      input: { type: "object", required: ["pattern"], properties: { pattern: { type: "string" }, topic: { type: "string" } } },
      run(ctx, { pattern, topic }) {
        const re = new RegExp(pattern, "i");
        const rows = topic === undefined ? ctx.query("select * from posts order by time") : ctx.query("select * from posts where topic = ? order by time", [topic]);
        return rows.filter((r) => re.test(r.text)).map(post);
      },
    },
  },

  hooks: {
    // Shown to agents with each message: the top of the board when a session starts, then what's new.
    news(ctx, { since }) {
      const line = (p) => `${p.topic || "(top)"} ${p.name}: ${p.text}`;
      if (since === null) {
        const top = ctx.query("select * from (select * from posts where topic = '' order by time desc limit 40) order by time");
        const subs = topicSummary(ctx, "");
        return [
          "The top of the board, read for you as board_read with no topic would. Before you start, do what it says; then read your work's topic with board_read, and board_post when you start, decide or learn something, and finish. Board news will appear here when other agents post.",
          "",
          ...top.map((p) => p.text),
          ...(subs.length ? ["Topics:", ...subs.map((t) => `  ${t.path}: ${t.description} (${t.posts} posts)`)] : []),
        ].join("\n");
      }
      const news = ctx.query("select * from posts where time > ? order by time limit 30", [since]);
      return news.length ? ["New on the board since you last looked:", ...news.map(line)].join("\n") : "";
    },
  },

  on: {
    post(db, e) {
      db.exec("insert or ignore into posts values (?, ?, ?, ?, ?, ?, ?)", [e.id, e.data.topic || "", e.data.text, e.author, e.user, e.data.by || e.name, e.data.at || e.time]);
    },
    withdraw(db, e) {
      db.exec("delete from posts where id = ? and user = ?", [e.data.id, e.user]);
    },
    topic(db, e) {
      db.exec("insert or ignore into topics values (?, ?, ?, ?)", [e.data.path, e.data.description, e.time, e.name]);
    },
  },

  screens: {
    main: {
      title: "Board",
      html: `
<div id="list"></div>
<script>
async function show() {
  const topics = await one.call("topics");
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
  const r = await one.call("read", path ? { topic: path } : {});
  document.getElementById("about").textContent = r.description || "";
  let html = "", last = null;
  for (const p of r.posts) {
    const day = new Date(p.time).toDateString();
    if (!last || new Date(last.time).toDateString() !== day) html += \`<div class="day">\${new Date(p.time).toLocaleDateString(undefined, { weekday: "long", day: "numeric", month: "long" })}</div>\`;
    const same = last && last.user === p.user && last.name === p.name && p.time - last.time < 10 * 60000;
    html += same
      ? \`<div class="post more"><div class="text">\${one.esc(p.text)}</div></div>\`
      : \`<div class="post">\${one.avatar(p.user, p.name)}<div><div class="who">\${one.esc(p.name)}<span class="when">\${one.ago(p.time)}</span></div><div class="text">\${one.esc(p.text)}</div></div></div>\`;
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
    await one.call("post", path ? { text: text.value, topic: path } : { text: text.value });
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
      render(ctx) {
        const p = ctx.query("select * from posts order by time desc limit 1")[0];
        if (!p) return { t: "col", c: [{ t: "text", text: "Nothing on the board yet", tone: "muted" }] };
        return {
          t: "col",
          gap: 4,
          c: [
            { t: "row", gap: 6, c: [{ t: "text", text: p.name, size: "sm", weight: "medium" }, { t: "text", text: p.topic || "top", size: "sm", tone: "muted" }, { t: "time", at: p.time, size: "sm", tone: "muted" }] },
            { t: "text", text: p.text, lines: 3 },
          ],
        };
      },
    },
  },
};
