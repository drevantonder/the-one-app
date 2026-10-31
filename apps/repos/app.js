// Repos: git hosting as an ordinary app. Installed in global, so every repo is visible to everyone. Local git builds
// packs and `git-remote-one` keeps them as blobs; this app only moves branches: a push moves a branch iff its base is
// the branch's head. No git runs here.
//
// A repo has an owner (user:<id>, group:<id> or admins). Only someone acting for it moves bare branches (`main`);
// anyone pushes `<branch>@<their name>`, a suggestion, which then belongs to them.

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const BRANCH = /^[A-Za-z0-9._\/-]{1,200}(@[a-z0-9._-]{1,64})?$/;
const SHA = /^[0-9a-f]{40}([0-9a-f]{24})?$/;

/// Whether the writer of `e` acts for `owner`: themselves, a group they're an admin of, or admins.
function actsFor(e, owner) {
  if (owner === `user:${e.user}`) return true;
  if (owner === "admins") return e.role === "admin";
  if (owner.startsWith("group:")) return e.groups?.[owner.slice(6)] === "admin";
  return false;
}

/// A branch's name, and the user it's a suggestion of (`main@bob`), if it is one.
const split = (branch) => {
  const at = branch.lastIndexOf("@");
  return at < 0 ? { base: branch, by: null } : { base: branch.slice(0, at), by: branch.slice(at + 1) };
};

/// What a user's name looks like after `@`.
const slug = (name) => String(name || "").toLowerCase().replace(/[^a-z0-9._-]+/g, "-").replace(/^-+|-+$/g, "");

/// Why `e` may not push to `branch` of `repo` (rows from the store), or null if it may.
function refusal(repo, row, e, branch) {
  if (!BRANCH.test(branch)) return `${branch} isn't a branch name`;
  const { by } = split(branch);
  if (!by) return actsFor(e, repo.owner) ? null : `only ${repo.owner} moves ${branch}; push your own, ${branch}@<your name>`;
  if (row && row.user !== e.user) return `${branch} is someone else's suggestion`;
  return null;
}

/// What to call an owner, as far as the caller can tell: you, a group of theirs, admins.
function ownerName(ctx, owner) {
  if (owner === `user:${ctx.me.user}`) return "you";
  if (owner === "admins") return "admins";
  const group = ctx.groups.find((g) => g.id === owner);
  if (group) return group.name;
  return owner.startsWith("group:") ? "a group" : "someone else";
}

const repoOf = async (g, name) => (await g.query("select * from repos where name = ?", [name]))[0] || null;

function global(ctx) {
  if (!ctx.global) throw new Error("repos lives in global: install it there");
  return ctx.global;
}

/// The caller as an entry would show them, to check a push before writing it.
const asWriter = (ctx) => ({
  user: ctx.me.user,
  role: ctx.global?.role,
  groups: Object.fromEntries(ctx.groups.map((g) => [g.id.slice(6), g.role])),
});

export default {
  name: "repos",
  title: "Repos",
  icon: "arrow.triangle.branch",
  instructions: `Repos is git hosting in this install: \`git clone one://<repo>\`, \`git push\` and \`git fetch\` work on any machine with a node. Only a repo's owner moves main; push your own branch as main@<your name> to suggest changes.`,

  schema: {
    global: `
      create table repos (name text primary key, owner text not null, "from" text, created integer not null, user text not null);
      create table branches (repo text not null, branch text not null, head text not null, user text not null, name text not null,
                             time integer not null, primary key (repo, branch));
      create table packs (repo text not null, hash text not null, time integer not null, primary key (repo, hash));
      create table pushes (id text primary key, repo text not null, branch text not null, base text, head text,
                           user text not null, name text not null, time integer not null);
      create index pushes_repo on pushes (repo, time);`,
  },

  ops: {
    list: {
      description: "Every repo: its owner, where it was forked from, its branches and suggestions, and when it last moved.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        return global(ctx).query(
          `select r.name, r.owner, r."from", r.created,
                  (select count(*) from branches b where b.repo = r.name and b.branch not like '%@%') as branches,
                  (select count(*) from branches b where b.repo = r.name and b.branch like '%@%') as suggestions,
                  (select max(time) from branches b where b.repo = r.name) as updated
             from repos r order by coalesce(updated, r.created) desc`,
        );
      },
    },
    get: {
      description: "A repo: its owner, branches and suggestions with their heads, forks, and the packs that hold its objects, oldest first.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await repoOf(g, repo);
        if (!r) throw new Error(`no repo ${repo}`);
        const branches = await g.query("select branch, head, user, name, time from branches where repo = ? order by branch", [repo]);
        const packs = (await g.query("select hash from packs where repo = ? order by time, hash", [repo])).map((p) => p.hash);
        const forks = (await g.query(`select name from repos where "from" = ? order by name`, [repo])).map((f) => f.name);
        const pushes = await g.query("select * from pushes where repo = ? order by time desc limit 30", [repo]);
        return { ...r, owner_name: ownerName(ctx, r.owner), branches, packs, forks, pushes };
      },
    },
    create: {
      description: "Create a repo, owned by you (the default), a group you're an admin of (group:<id>), or admins.",
      input: { type: "object", required: ["name"], properties: { name: { type: "string" }, owner: { type: "string" } } },
      async run(ctx, { name, owner }) {
        const g = global(ctx);
        owner ||= `user:${ctx.me.user}`;
        if (!NAME.test(name || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
        if (await repoOf(g, name)) throw new Error(`${name} already exists`);
        if (!actsFor(asWriter(ctx), owner)) throw new Error(`you don't act for ${owner}`);
        g.append("repo", { name, owner });
        return `Created ${name}.`;
      },
    },
    fork: {
      description: "Fork a repo as a new one of yours: its branches (not suggestions) as they are now, and its history.",
      input: { type: "object", required: ["repo", "name"], properties: { repo: { type: "string" }, name: { type: "string" }, owner: { type: "string" } } },
      async run(ctx, { repo, name, owner }) {
        const g = global(ctx);
        owner ||= `user:${ctx.me.user}`;
        if (!(await repoOf(g, repo))) throw new Error(`no repo ${repo}`);
        if (!NAME.test(name || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
        if (await repoOf(g, name)) throw new Error(`${name} already exists`);
        if (!actsFor(asWriter(ctx), owner)) throw new Error(`you don't act for ${owner}`);
        g.append("repo", { name, owner, from: repo });
        return `Forked ${repo} as ${name}.`;
      },
    },
    push: {
      description: "Move a branch from base (its head now, or null for a new branch) to head (null deletes it), with the pack that holds the new objects. git-remote-one calls this; a new repo is made, yours.",
      input: {
        type: "object",
        required: ["repo", "branch"],
        properties: { repo: { type: "string" }, branch: { type: "string" }, base: { type: ["string", "null"] }, head: { type: ["string", "null"] }, pack: { type: ["string", "null"] } },
      },
      async run(ctx, { repo, branch, base = null, head = null, pack = null }) {
        const g = global(ctx);
        let r = await repoOf(g, repo);
        if (!r) {
          if (!NAME.test(repo || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
          r = { name: repo, owner: `user:${ctx.me.user}` };
          g.append("repo", { name: repo, owner: r.owner });
        }
        if ((base && !SHA.test(base)) || (head && !SHA.test(head))) throw new Error("base and head are commit hashes");
        const row = (await g.query("select * from branches where repo = ? and branch = ?", [repo, branch]))[0];
        const why = refusal(r, row, asWriter(ctx), branch);
        if (why) throw new Error(why);
        const { by } = split(branch);
        if (by && !row && by !== slug(ctx.user?.name) && by !== ctx.me.user) throw new Error(`suggestions are yours: push ${split(branch).base}@${slug(ctx.user?.name) || ctx.me.user}`);
        if ((row?.head ?? null) !== base) throw new Error(`${branch} has moved (it's at ${row?.head ?? "nothing"}): fetch, then push again`);
        g.append("push", { repo, branch, base, head }, pack ? [pack] : []);
        return head ? `${repo} ${branch} is at ${head.slice(0, 12)}.` : `Deleted ${branch} of ${repo}.`;
      },
    },
  },

  on: {
    repo(db, e) {
      const { name, owner, from } = e.data;
      if (!NAME.test(name || "") || db.query("select 1 from repos where name = ?", [name]).length) return;
      if (!actsFor(e, owner || "")) return;
      if (from && !db.query("select 1 from repos where name = ?", [from]).length) return;
      db.exec(`insert into repos values (?, ?, ?, ?, ?)`, [name, owner, from || null, e.time, e.user]);
      if (from) {
        db.exec(`insert into branches select ?, branch, head, ?, ?, ? from branches where repo = ? and branch not like '%@%'`, [name, e.user, e.name, e.time, from]);
        db.exec(`insert or ignore into packs select ?, hash, time from packs where repo = ?`, [name, from]);
      }
    },
    push(db, e) {
      const { repo, branch, base = null, head = null } = e.data;
      const r = db.query("select * from repos where name = ?", [repo])[0];
      if (!r) return;
      const row = db.query("select * from branches where repo = ? and branch = ?", [repo, branch])[0];
      if (refusal(r, row, e, branch) || (row?.head ?? null) !== base) return;
      if (head) db.exec("insert or replace into branches values (?, ?, ?, ?, ?, ?)", [repo, branch, head, row?.user ?? e.user, e.name, e.time]);
      else db.exec("delete from branches where repo = ? and branch = ?", [repo, branch]);
      for (const hash of e.blobs) db.exec("insert or ignore into packs values (?, ?, ?)", [repo, hash, e.time]);
      db.exec("insert into pushes values (?, ?, ?, ?, ?, ?, ?, ?)", [e.id, repo, branch, base, head, e.user, e.name, e.time]);
    },
  },

  screens: {
    main: {
      title: "Repos",
      html: `
<div id="list"></div>
<script>
async function show() {
  const repos = await one.call("list");
  const row = (r) => \`<a class="row" onclick='one.open("repo", \${JSON.stringify({ repo: r.name })})'>
    <div class="icon\${r.from ? " grey" : ""}">\${one.esc(r.name[0].toUpperCase())}</div>
    <div class="main">
      <div class="title"><span>\${one.esc(r.name)}</span><span class="when">\${r.updated ? one.ago(r.updated) : ""}</span></div>
      <div class="sub">\${r.branches} branch\${r.branches === 1 ? "" : "es"}\${r.suggestions ? \` · \${r.suggestions} suggestion\${r.suggestions === 1 ? "" : "s"}\` : ""}\${r.from ? " · fork of " + one.esc(r.from) : ""}</div>
    </div>
    <div class="chev">›</div>
  </a>\`;
  document.getElementById("list").innerHTML = repos.length
    ? \`<div class="list">\${repos.map(row).join("")}</div>\`
    : '<div class="empty">No repos yet.<br><span class="small">Push one: git push one://&lt;name&gt; main</span></div>';
}
one.on("changed", show);
show();
</script>`,
    },
    repo: {
      title: "Repo",
      html: `
<div id="repo"></div>
<script>
const name = one.params.repo;
one.title(name);
async function show() {
  const r = await one.call("get", { repo: name });
  const branch = (b) => \`<div class="row plain"><div class="main">
      <div class="title"><span>\${one.esc(b.branch)}</span><span class="when">\${one.ago(b.time)}</span></div>
      <div class="sub"><code>\${b.head.slice(0, 12)}</code> · \${one.esc(b.name)}</div>
    </div></div>\`;
  const bare = r.branches.filter((b) => !b.branch.includes("@")), suggested = r.branches.filter((b) => b.branch.includes("@"));
  const push = (p) => \`<div class="row plain"><div class="main">
      <div class="title"><span>\${one.esc(p.branch)}</span><span class="when">\${one.ago(p.time)}</span></div>
      <div class="sub">\${p.head ? "<code>" + (p.base ? p.base.slice(0, 7) + "…" : "") + p.head.slice(0, 7) + "</code>" : "deleted"} · \${one.esc(p.name)}</div>
    </div></div>\`;
  const fork = (f) => \`<a class="row plain" onclick='one.open("repo", \${JSON.stringify({ repo: f })})'><div class="main"><div class="title"><span>\${one.esc(f)}</span></div></div><div class="chev">›</div></a>\`;
  document.getElementById("repo").innerHTML = \`
    <p class="muted small">Owned by \${one.esc(r.owner_name)}\${r.from ? " · fork of " + one.esc(r.from) : ""}</p>
    <div class="list"><div class="row plain"><div class="main"><div class="sub">git clone one://\${one.esc(r.name)}</div></div></div></div>
    <h2 class="section">Branches</h2>
    <div class="list">\${bare.map(branch).join("") || '<div class="empty">No branches yet</div>'}</div>
    \${suggested.length ? '<h2 class="section">Suggestions</h2><div class="list">' + suggested.map(branch).join("") + "</div>" : ""}
    \${r.forks.length ? '<h2 class="section">Forks</h2><div class="list">' + r.forks.map(fork).join("") + "</div>" : ""}
    \${r.pushes.length ? '<h2 class="section">Pushes</h2><div class="list">' + r.pushes.map(push).join("") + "</div>" : ""}\`;
}
one.on("changed", show);
show();
</script>`,
    },
  },

  widgets: {
    latest: {
      title: "Repos",
      async render(ctx) {
        if (!ctx.global) return { t: "col", c: [] };
        const b = (await ctx.global.query("select repo, branch, name, time from branches order by time desc limit 1"))[0];
        if (!b) return { t: "col", c: [{ t: "text", text: "No repos yet", tone: "muted" }] };
        return {
          t: "col",
          gap: 4,
          c: [
            { t: "row", gap: 6, c: [{ t: "text", text: b.repo, size: "sm", weight: "medium" }, { t: "text", text: b.branch, size: "sm", tone: "muted" }, { t: "time", at: b.time, size: "sm", tone: "muted" }] },
            { t: "text", text: `${b.name} pushed`, lines: 1 },
          ],
        };
      },
    },
  },
};
