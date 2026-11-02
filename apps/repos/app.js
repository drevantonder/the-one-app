// Repos: git hosting as an ordinary app. Installed in global, so every repo is visible to everyone. Local git builds
// packs and `git-remote-one` keeps them as blobs; this app only moves branches: a push moves a branch iff its base is
// the branch's head. No git runs here: pages read the packs themselves (browse.js, git.js).
//
// A repo has an owner (user:<id>, group:<id> or admins). Only someone acting for it moves bare branches (`main`);
// anyone pushes `<branch>@<their name>`, a suggestion, which then belongs to them. Its owners delete it, as apps are:
// it waits in Recently Deleted, read by no one, until they restore it or erase it, and its packs, for good.

import * as browse from "./browse.js";
import css from "./screens/repos.css" with { type: "text" };
import common from "./screens/common.js" with { type: "text" };
import mainScreen from "./screens/main.html" with { type: "text" };
import repoScreen from "./screens/repo.html" with { type: "text" };
import fileScreen from "./screens/file.html" with { type: "text" };
import historyScreen from "./screens/history.html" with { type: "text" };
import commitScreen from "./screens/commit.html" with { type: "text" };
import branchesScreen from "./screens/branches.html" with { type: "text" };

/// A screen: Repos' styles and shared script, then its own HTML.
const page = (title, html) => ({ title, html: `<style>${css}</style><script>${common}</script>${html}` });

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

const randomId = () => [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");

/// Writes a new repo, or a fork of `from` with its branches (not suggestions) and history as they are now.
function newRepo(ctx, g, { name, owner, from = null }) {
  g.exec(`insert into repos (name, owner, "from", created, user) values (?, ?, ?, ?, ?)`, [name, owner, from, ctx.now(), ctx.me.user]);
  if (from) {
    g.exec(`insert into branches select ?, branch, head, ?, ?, ? from branches where repo = ? and branch not like '%@%'`, [name, ctx.me.user, ctx.me.name, ctx.now(), from]);
    g.exec("insert or ignore into packs select ?, hash, time from packs where repo = ?", [name, from]);
  }
}

/// What to call an owner, as far as the caller can tell: you, a group of theirs, admins.
function ownerName(ctx, owner) {
  if (owner === `user:${ctx.me.user}`) return ctx.user?.name || "you";
  if (owner === "admins") return "admins";
  const group = ctx.groups.find((g) => g.id === owner);
  if (group) return group.name;
  return owner.startsWith("group:") ? "a group" : "someone else";
}

const repoOf = async (g, name) => (await g.query("select * from repos where name = ?", [name]))[0] || null;

/// A repo that's there to read and push to: not deleted.
async function live(g, name) {
  const r = await repoOf(g, name);
  if (r?.deleted) throw new Error(`${name} was deleted: its owners restore it from Recently Deleted`);
  return r;
}

/// A repo the caller acts for the owner of, to delete, restore or erase.
async function owned(ctx, name) {
  const r = await repoOf(global(ctx), name);
  if (!r) throw new Error(`no repo ${name}`);
  if (!actsFor(asWriter(ctx), r.owner)) throw new Error(`only ${ownerName(ctx, r.owner)} deletes, restores or erases ${name}`);
  return r;
}

/// When a deleted repo was deleted, and by whom.
const deletedOf = (r) => (r.deleted ? { time: r.deleted, by: r.deleted_by } : null);

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
  color: "#5856d6",
  description: "Git hosting in the install: clone, push and suggest changes",
  instructions: `Repos is git hosting in this install: \`git clone one://<repo>\`, \`git push\` and \`git fetch\` work on any computer in this install. Only a repo's owner moves main; push your own branch as main@<your name> to suggest changes.`,

  schema: {
    global: `
      create table repos (name text primary key, owner text not null, "from" text, created integer not null, user text not null,
                          deleted integer, deleted_by text);
      create table branches (repo text not null, branch text not null, head text not null, user text not null, name text not null,
                             time integer not null, primary key (repo, branch));
      create table packs (repo text not null, hash text not null, time integer not null, primary key (repo, hash));
      create table pushes (id text primary key, repo text not null, branch text not null, base text, head text,
                           user text not null, name text not null, time integer not null);
      create index pushes_repo on pushes (repo, time);`,
  },

  ops: {
    list: {
      reads: true,
      description: "Every repo: its owner, where it was forked from, its default branch (main, or else its first), how many branches and suggestions it has, and when it last moved. Deleted ones say when, and wait in Recently Deleted.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const repos = await global(ctx).query(
          `select r.name, r.owner, r."from", r.created, r.deleted, r.deleted_by,
                  coalesce((select branch from branches b where b.repo = r.name and b.branch = 'main'),
                           (select min(branch) from branches b where b.repo = r.name and b.branch not like '%@%')) as branch,
                  (select count(*) from branches b where b.repo = r.name and b.branch not like '%@%') as branches,
                  (select count(*) from branches b where b.repo = r.name and b.branch like '%@%') as suggestions,
                  (select max(time) from branches b where b.repo = r.name) as updated
             from repos r order by coalesce(updated, r.created) desc`,
        );
        return repos.map((r) => ({ ...r, deleted: deletedOf(r), deleted_by: undefined, owner_name: ownerName(ctx, r.owner), yours: actsFor(asWriter(ctx), r.owner) }));
      },
    },
    get: {
      reads: true,
      description: "A repo: its owner, branches and suggestions with their heads, forks, and the packs that hold its objects, oldest first.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await live(g, repo);
        if (!r) throw new Error(`no repo ${repo}`);
        const branches = await g.query("select branch, head, user, name, time from branches where repo = ? order by branch", [repo]);
        const packs = (await g.query("select hash from packs where repo = ? order by time, hash", [repo])).map((p) => p.hash);
        const forks = (await g.query(`select name from repos where "from" = ? order by name`, [repo])).map((f) => f.name);
        const pushes = await g.query("select * from pushes where repo = ? order by time desc limit 30", [repo]);
        return { ...r, owner_name: ownerName(ctx, r.owner), branches, packs, forks, pushes };
      },
    },
    tree: {
      reads: true,
      description: "A folder of a repo at a branch or commit (main if none): its files and folders, its README, and the latest commit.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" }, ref: { type: "string" }, path: { type: "string" } } },
      async run(ctx, input) {
        const r = await repoOf(global(ctx), input.repo);
        const yours = !!r && actsFor(asWriter(ctx), r.owner);
        // A deleted one shows only that it was, for its owners to restore or erase.
        if (r?.deleted) return { repo: r.name, owner: r.owner, owner_name: ownerName(ctx, r.owner), yours, deleted: deletedOf(r), branches: [], entries: [] };
        const t = await browse.tree(ctx, global(ctx), input);
        return { ...t, owner_name: ownerName(ctx, t.owner), yours };
      },
    },
    file: {
      reads: true,
      description: "A file of a repo at a branch or commit: its text, or whether it's binary or too big to show.",
      input: { type: "object", required: ["repo", "path"], properties: { repo: { type: "string" }, ref: { type: "string" }, path: { type: "string" } } },
      run: (ctx, input) => browse.file(ctx, global(ctx), input),
    },
    log: {
      reads: true,
      description: "A repo's history, newest first, 25 commits a page (page 0 is the newest), of the whole repo or of one path.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" }, ref: { type: "string" }, path: { type: "string" }, page: { type: "integer" } } },
      run: (ctx, input) => browse.log(ctx, global(ctx), input),
    },
    commit: {
      reads: true,
      description: "A commit: its message, author, parents, and each file it changed with +/- counts and diff hunks (big diffs are left for diff).",
      input: { type: "object", required: ["repo", "sha"], properties: { repo: { type: "string" }, sha: { type: "string" } } },
      run: (ctx, input) => browse.commit(ctx, global(ctx), input),
    },
    diff: {
      reads: true,
      description: "One file's whole diff in a commit.",
      input: { type: "object", required: ["repo", "sha", "path"], properties: { repo: { type: "string" }, sha: { type: "string" }, path: { type: "string" } } },
      run: (ctx, input) => browse.diff(ctx, global(ctx), input),
    },
    latest: {
      reads: true,
      description: "The last commit that changed each entry of a folder, by name.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" }, ref: { type: "string" }, path: { type: "string" } } },
      run: (ctx, input) => browse.latest(ctx, global(ctx), input),
    },
    branches: {
      reads: true,
      description: "A repo's branches and suggestions, each with its head commit's subject and author.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      run: (ctx, input) => browse.branches(ctx, global(ctx), input),
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
        newRepo(ctx, g, { name, owner });
        return `Created ${name}.`;
      },
    },
    fork: {
      description: "Fork a repo as a new one of yours: its branches (not suggestions) as they are now, and its history.",
      input: { type: "object", required: ["repo", "name"], properties: { repo: { type: "string" }, name: { type: "string" }, owner: { type: "string" } } },
      async run(ctx, { repo, name, owner }) {
        const g = global(ctx);
        owner ||= `user:${ctx.me.user}`;
        if (!(await live(g, repo))) throw new Error(`no repo ${repo}`);
        if (!NAME.test(name || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
        if (await repoOf(g, name)) throw new Error(`${name} already exists`);
        if (!actsFor(asWriter(ctx), owner)) throw new Error(`you don't act for ${owner}`);
        newRepo(ctx, g, { name, owner, from: repo });
        return `Forked ${repo} as ${name}.`;
      },
    },
    delete: {
      confirm: true,
      description: "Delete a repo you own: no one can read, clone or push to it, and it waits in Recently Deleted, where its owners restore it or erase it for good.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (r.deleted) throw new Error(`${repo} is already in Recently Deleted`);
        global(ctx).exec("update repos set deleted = ?, deleted_by = ? where name = ? and deleted is null", [ctx.now(), ctx.me.name, repo]);
        return `Deleted ${repo}: it's in Recently Deleted.`;
      },
    },
    restore: {
      description: "Restore a repo you own from Recently Deleted, as it was.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (!r.deleted) throw new Error(`${repo} isn't deleted`);
        global(ctx).exec("update repos set deleted = null, deleted_by = null where name = ?", [repo]);
        return `Restored ${repo}.`;
      },
    },
    erase: {
      confirm: true,
      description: "Erase a deleted repo you own for good: its branches, history and the packs no other repo shares are gone, and its name is free again.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string" } } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await owned(ctx, repo);
        if (!r.deleted) throw new Error(`delete ${repo} before erasing it`);
        const packs = await g.query("select hash from packs where repo = ? and hash not in (select hash from packs where repo != ?)", [repo, repo]);
        // Erased, it's gone, and its name is free again; its forks keep their code, no longer forked from it.
        for (const table of ["branches", "packs", "pushes"]) g.exec(`delete from ${table} where repo = ?`, [repo]);
        g.exec("delete from repos where name = ?", [repo]);
        g.exec(`update repos set "from" = null where "from" = ?`, [repo]);
        for (const p of packs) g.forget(p.hash);
        return `Erased ${repo}.`;
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
        let r = await live(g, repo);
        if (!r) {
          if (!NAME.test(repo || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
          r = { name: repo, owner: `user:${ctx.me.user}` };
          newRepo(ctx, g, { name: repo, owner: r.owner });
        }
        if ((base && !SHA.test(base)) || (head && !SHA.test(head))) throw new Error("base and head are commit hashes");
        const row = (await g.query("select * from branches where repo = ? and branch = ?", [repo, branch]))[0];
        const why = refusal(r, row, asWriter(ctx), branch);
        if (why) throw new Error(why);
        const { by } = split(branch);
        if (by && !row && by !== slug(ctx.user?.name) && by !== ctx.me.user) throw new Error(`suggestions are yours: push ${split(branch).base}@${slug(ctx.user?.name) || ctx.me.user}`);
        if ((row?.head ?? null) !== base) throw new Error(`${branch} has moved (it's at ${row?.head ?? "nothing"}): fetch, then push again`);
        g.check("select (select head from branches where repo = ? and branch = ?) is ?", [repo, branch, base], `${branch} has moved: fetch, then push again`);
        if (head) g.exec("insert or replace into branches values (?, ?, ?, ?, ?, ?)", [repo, branch, head, row?.user ?? ctx.me.user, ctx.me.name, ctx.now()]);
        else g.exec("delete from branches where repo = ? and branch = ?", [repo, branch]);
        if (pack) g.exec("insert or ignore into packs values (?, ?, ?)", [repo, pack, ctx.now()], [pack]);
        g.exec("insert into pushes values (?, ?, ?, ?, ?, ?, ?, ?)", [randomId(), repo, branch, base, head, ctx.me.user, ctx.me.name, ctx.now()]);
        return head ? `${repo} ${branch} is at ${head.slice(0, 12)}.` : `Deleted ${branch} of ${repo}.`;
      },
    },
  },


  screens: {
    main: page("Repos", mainScreen),
    repo: page("Repo", repoScreen),
    file: page("File", fileScreen),
    history: page("History", historyScreen),
    commit: page("Commit", commitScreen),
    branches: page("Branches", branchesScreen),
  },

  widgets: {
    latest: {
      title: "Repos",
      description: "The newest commits, across repos or in one.",
      size: "small",
      sizes: ["small", "medium", "large", "xl"],
      options: {
        repo: {
          title: "Repo",
          default: "",
          choices: async (ctx) => [{ value: "", title: "All Repos" }, ...(ctx.global ? await ctx.global.query("select name from repos where deleted is null order by name") : []).map((r) => r.name)],
        },
      },
      async render(ctx) {
        if (!ctx.global) return { t: "col", c: [] };
        const many = { small: 1, medium: 3, large: 6, xl: 6 }[ctx.size] || 3;
        const commits = await recent(ctx, many, ctx.options.repo);
        if (!commits.length) return { t: "text", text: "No repos yet", size: "sm", tone: "muted" };
        const link = (c) => ({ screen: "commit", params: { repo: c.repo, sha: c.sha } });
        if (ctx.size === "small") {
          const c = commits[0];
          return {
            t: "col", gap: 3, link: link(c),
            c: [
              { t: "row", gap: 4, c: [{ t: "text", text: c.repo, size: "xs", weight: "medium", tone: "accent", lines: 1 }, { t: "time", at: c.time, size: "xs", tone: "muted" }] },
              { t: "text", text: c.subject, size: "sm", lines: 4 },
              { t: "col", grow: true, c: [] },
              { t: "text", text: `${c.author} · ${c.branch}`, size: "xs", tone: "muted", lines: 1 },
            ],
          };
        }
        return {
          // Spread out to fill the widget.
          t: "col", gap: 4,
          c: commits.flatMap((c, i) => [...(i ? [{ t: "col", grow: true, c: [] }] : []), {
            t: "col", gap: 2, link: link(c),
            c: [
              { t: "row", gap: 5, c: [
                { t: "text", text: c.repo, size: "xs", weight: "medium", lines: 1 },
                { t: "text", text: c.author, size: "xs", tone: "accent", lines: 1 },
                { t: "col", grow: true, c: [] },
                { t: "time", at: c.time, size: "xs", tone: "muted" },
              ] },
              { t: "text", text: c.subject, size: "sm", lines: 1 },
            ],
          }]),
        };
      },
    },
  },
};

/// The newest `many` commits across repos' main branches, each with its repo and branch: the branches pushed to last,
/// and their history, newest first. A repo whose packs can't be read is left out.
async function recent(ctx, many, repo = "") {
  const heads = await ctx.global.query(
    "select repo, branch from branches where branch not like '%@%' and (? = '' or repo = ?) and repo not in (select name from repos where deleted is not null) order by time desc limit ?",
    [repo || "", repo || "", many],
  );
  const all = [];
  for (const { repo, branch } of heads) {
    try {
      const { commits } = await browse.log(ctx, ctx.global, { repo, ref: branch });
      all.push(...commits.slice(0, many).map((c) => ({ ...c, repo, branch })));
    } catch {}
  }
  // Repos that share history (a fork, or one split out of another) list a commit once.
  const seen = new Set();
  const key = (c) => `${c.time} ${c.subject}`;
  return all.sort((a, b) => b.time - a.time).filter((c) => !seen.has(key(c)) && seen.add(key(c))).slice(0, many);
}
