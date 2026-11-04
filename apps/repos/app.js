// Repos: git hosting as an ordinary app. Installed in global, so every repo is visible to everyone. Stock git clones and
// pushes over HTTP through the node's local API (http.js), and a repo's packs are blobs. No git runs here: pages read
// the packs themselves (browse.js, git.js).
//
// A repo has an owner (user:<id>, group:<id> or admins), and only someone acting for it pushes; anyone else forks it
// and pushes to their fork. Its owners delete it, as apps are: it waits in Recently Deleted, read by no one, until they
// restore it or erase it, and its packs, for good.

import * as browse from "./browse.js";
import * as http from "./http.js";
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
const BRANCH = /^(?!\/)(?!.*(\.\.|\/\/|\/$|\.lock$|@\{))[A-Za-z0-9._\/@+-]{1,200}$/;

/// Whether the writer of `e` acts for `owner`: themselves, a group they're an admin of, or admins.
function actsFor(e, owner) {
  if (owner === `user:${e.user}`) return true;
  if (owner === "admins") return e.role === "admin";
  if (owner.startsWith("group:")) return e.groups?.[owner.slice(6)] === "admin";
  return false;
}

/// Times in what an operation gives, ms since 1970 here, as ISO 8601.
function iso(v) {
  if (Array.isArray(v)) return v.map(iso);
  if (!v || typeof v !== "object") return v;
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, typeof x === "number" && ["time", "created", "updated", "committed"].includes(k) ? new Date(x).toISOString() : iso(x)]));
}

const OWNER = { type: "string", description: "who owns it: omit for you, admins, or a group you're an admin of as group:<id>, from groups_list" };
const REPO = { type: "string", description: "a repo's name, from repos_list, like notes" };
const REF = { type: "string", description: "a branch, like main, or a commit's sha, from repos_branches or repos_log; default main, else the first branch" };
const PATH = { type: "string", description: "a path in the repo, like src/app.js or docs; omit for the whole repo" };
const SHA = { type: "string", description: "a commit's sha, full or its first 7 characters, from repos_log" };

/// The repo named, or an error naming those there are.
async function named(ctx, name) {
  const r = await live(global(ctx), name);
  if (r) return r;
  const all = (await global(ctx).query("select name from repos where deleted is null order by name")).map((r) => r.name);
  throw new Error(`no repo ${name}: ${all.length ? `there's ${all.join(", ")}` : "there are none yet; repos_create makes one"}`);
}

const randomId = () => [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");

/// Writes a new repo, or a fork of `from` with its branches and history as they are now.
function newRepo(ctx, g, { name, owner, from = null }) {
  g.exec(`insert into repos (name, owner, "from", created, user) values (?, ?, ?, ?, ?)`, [name, owner, from, ctx.now(), ctx.me.user]);
  if (from) {
    g.exec(`insert into branches select ?, branch, head, ?, ?, ? from branches where repo = ?`, [name, ctx.me.user, ctx.me.name, ctx.now(), from]);
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
  if (!r) await named(ctx, name);
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

/// Where git reaches a repo on this computer, http://localhost:<port>/repos/<repo>.git; null where there's no computer
/// (the serverless node).
const address = (ctx, repo) => (ctx.port ? `http://localhost:${ctx.port}/repos/${repo}.git` : null);

export default {
  name: "repos",
  title: "Repos",
  icon: "git-branch",
  color: "#5856d6",
  advanced: true,
  description: "Git hosting for you and your agents: clone, push and suggest changes",
  instructions: `Repos is git hosting for the person and their agents. On any of their computers, \`git clone http://localhost:<port>/repos/<repo>.git\` (the address its page shows), \`git push\` and \`git fetch\` work as with any git host; pushing to a name that isn't a repo yet makes it, yours. Only a repo's owner pushes to it: to suggest changes, fork it (repos fork), push to your fork, and tell its owner, who fetches from it.`,

  schema: {
    global: [`
      create table repos (name text primary key, owner text not null, "from" text, created integer not null, user text not null,
                          deleted integer, deleted_by text);
      create table branches (repo text not null, branch text not null, head text not null, user text not null, name text not null,
                             time integer not null, primary key (repo, branch));
      create table packs (repo text not null, hash text not null, time integer not null, primary key (repo, hash));
      create table pushes (id text primary key, repo text not null, branch text not null, base text, head text,
                           user text not null, name text not null, time integer not null);
      create index pushes_repo on pushes (repo, time);`,
      // Each pack's name (its checksum) and index file, which git fetches beside it.
      `create table indexes (pack text primary key, name text not null, idx text not null, size integer not null);`],
  },

  ops: {
    list: {
      reads: true,
      description: "Every repo, the latest moved first: its name, its owner's name, where it was forked from, its default branch (main, or else its first), how many branches it has, and when it last moved. Deleted ones say when, and wait in Recently Deleted for repos_restore.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const repos = await global(ctx).query(
          `select r.name, r.owner, r."from", r.created, r.deleted, r.deleted_by,
                  coalesce((select branch from branches b where b.repo = r.name and b.branch = 'main'),
                           (select min(branch) from branches b where b.repo = r.name)) as branch,
                  (select count(*) from branches b where b.repo = r.name) as branches,
                  (select max(time) from branches b where b.repo = r.name) as updated
             from repos r order by coalesce(updated, r.created) desc`,
        );
        return iso(repos.map((r) => ({ ...r, deleted: deletedOf(r), deleted_by: undefined, owner_name: ownerName(ctx, r.owner), yours: actsFor(asWriter(ctx), r.owner) })));
      },
    },
    get: {
      reads: true,
      description: "A repo: its address for git clone and push on this computer, its owner, its branches with their heads, its forks, its last 30 pushes, and the packs that hold its objects, oldest first.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const branches = await g.query("select branch, head, user, name, time from branches where repo = ? order by branch", [repo]);
        const packs = (await g.query("select hash from packs where repo = ? order by time, hash", [repo])).map((p) => p.hash);
        const forks = (await g.query(`select name from repos where "from" = ? order by name`, [repo])).map((f) => f.name);
        const pushes = await g.query("select * from pushes where repo = ? order by time desc limit 30", [repo]);
        return iso({ ...r, address: address(ctx, repo), owner_name: ownerName(ctx, r.owner), branches, packs, forks, pushes });
      },
    },
    tree: {
      reads: true,
      description: "A folder of a repo at a branch or commit: its files and folders, its README, the latest commit, and the repo's forks. repos_file reads a file.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "a folder's path, like src; omit for the top" } } },
      async run(ctx, input) {
        const r = await repoOf(global(ctx), input.repo);
        if (!r) await named(ctx, input.repo);
        const yours = !!r && actsFor(asWriter(ctx), r.owner);
        // A deleted one shows only that it was, for its owners to restore or erase.
        if (r?.deleted) return iso({ repo: r.name, owner: r.owner, owner_name: ownerName(ctx, r.owner), yours, deleted: deletedOf(r), branches: [], entries: [] });
        const t = await browse.tree(ctx, global(ctx), input);
        const forks = (await global(ctx).query(`select name from repos where "from" = ? and deleted is null order by name`, [input.repo])).map((f) => f.name);
        return iso({ ...t, owner_name: ownerName(ctx, t.owner), yours, forks });
      },
    },
    file: {
      reads: true,
      description: "A file of a repo at a branch or commit: its text, or whether it's binary or too big to show (then git clone it).",
      input: { type: "object", required: ["repo", "path"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "the file's path, from repos_tree, like src/app.js" } } },
      run: async (ctx, input) => (await named(ctx, input.repo), iso(await browse.file(ctx, global(ctx), input))),
    },
    log: {
      reads: true,
      description: "A repo's history, as git log gives it: newest first, 25 commits a page, of the whole repo or of one path, each with its sha, subject, author and time; more says whether there's another page.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: PATH, page: { type: "integer", description: "which 25: 0 (the default) for the newest, 1 for the 25 before them, and on while more is true" } } },
      run: async (ctx, input) => (await named(ctx, input.repo), iso(await browse.log(ctx, global(ctx), input))),
    },
    commit: {
      reads: true,
      description: "A commit: its message, author, time, parents, and each file it changed with +/- counts and diff hunks (a big file's diff is left for repos_diff).",
      input: { type: "object", required: ["repo", "sha"], properties: { repo: REPO, sha: SHA } },
      run: async (ctx, input) => (await named(ctx, input.repo), iso(await browse.commit(ctx, global(ctx), input))),
    },
    diff: {
      reads: true,
      description: "One file's whole diff in a commit, against its first parent, as hunks with + and - lines: for a file repos_commit left out as too big.",
      input: { type: "object", required: ["repo", "sha", "path"], properties: { repo: REPO, sha: SHA, path: { type: "string", description: "the file's path, from repos_commit's files, like src/app.js" } } },
      run: async (ctx, input) => (await named(ctx, input.repo), browse.diff(ctx, global(ctx), input)),
    },
    latest: {
      reads: true,
      description: "The last commit that changed each entry of a folder, by the entry's name: its sha, subject, author and time.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "a folder's path, like src; omit for the top" } } },
      run: async (ctx, input) => (await named(ctx, input.repo), iso(await browse.latest(ctx, global(ctx), input))),
    },
    branches: {
      reads: true,
      description: "A repo's branches, each with its head commit's sha, subject, author and time, and who pushed it when.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      run: async (ctx, input) => (await named(ctx, input.repo), iso(await browse.branches(ctx, global(ctx), input))),
    },
    create: {
      reads: false,
      description: "Create an empty repo, owned by you (the default), a group you're an admin of, or admins, and give the git push that fills it. Pushing to a name that isn't a repo yet makes one too.",
      input: { type: "object", required: ["name"], properties: { name: { type: "string", description: "lowercase letters, digits, ., _ and -, like notes" }, owner: OWNER } },
      async run(ctx, { name, owner }) {
        const g = global(ctx);
        owner ||= `user:${ctx.me.user}`;
        if (!NAME.test(name || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
        if (await repoOf(g, name)) throw new Error(`${name} already exists`);
        if (!actsFor(asWriter(ctx), owner)) throw new Error(`you don't act for ${owner}: give user:<your id>, admins, or a group you're an admin of`);
        newRepo(ctx, g, { name, owner });
        return address(ctx, name) ? `Created ${name}: git push ${address(ctx, name)} main` : `Created ${name}.`;
      },
    },
    fork: {
      reads: false,
      description: "Fork a repo as a new one of yours, to push changes you'd suggest to its owner: its branches as they are now, and its history.",
      input: { type: "object", required: ["repo", "name"], properties: { repo: REPO, name: { type: "string", description: "the fork's name: lowercase letters, digits, ., _ and -, like notes-andre" }, owner: OWNER } },
      async run(ctx, { repo, name, owner }) {
        const g = global(ctx);
        owner ||= `user:${ctx.me.user}`;
        await named(ctx, repo);
        if (!NAME.test(name || "")) throw new Error("repo names are lowercase letters, digits, ., _ and -");
        if (await repoOf(g, name)) throw new Error(`${name} already exists`);
        if (!actsFor(asWriter(ctx), owner)) throw new Error(`you don't act for ${owner}`);
        newRepo(ctx, g, { name, owner, from: repo });
        return `Forked ${repo} as ${name}.`;
      },
    },
    delete: {
      reads: false,
      confirm: true,
      description: "Delete a repo you own: no one can read, clone or push to it, and it waits in Recently Deleted, where its owners restore it (repos_restore) or erase it for good.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (r.deleted) throw new Error(`${repo} is already in Recently Deleted`);
        global(ctx).exec("update repos set deleted = ?, deleted_by = ? where name = ? and deleted is null", [ctx.now(), ctx.me.name, repo]);
        return `Deleted ${repo}: it's in Recently Deleted.`;
      },
    },
    restore: {
      reads: false,
      description: "Restore a repo you own from Recently Deleted, as it was.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string", description: "a deleted repo's name, from repos_list" } } },
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (!r.deleted) throw new Error(`${repo} isn't deleted`);
        global(ctx).exec("update repos set deleted = null, deleted_by = null where name = ?", [repo]);
        return `Restored ${repo}.`;
      },
    },
    erase: {
      reads: false,
      confirm: true,
      description: "Erase a deleted repo you own for good: its branches, history and the packs no other repo shares are gone, and its name is free again.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string", description: "a deleted repo's name, from repos_list" } } },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await owned(ctx, repo);
        if (!r.deleted) throw new Error(`delete ${repo} before erasing it`);
        const packs = await g.query("select hash from packs where repo = ?", [repo]);
        // Erased, it's gone, and its name is free again; its forks keep their code, no longer forked from it.
        for (const p of packs) await http.forgetUnshared(g, repo, p.hash);
        for (const table of ["branches", "packs", "pushes"]) g.exec(`delete from ${table} where repo = ?`, [repo]);
        g.exec("delete from repos where name = ?", [repo]);
        g.exec(`update repos set "from" = null where "from" = ?`, [repo]);
        return `Erased ${repo}.`;
      },
    },
    _http: {
      reads: false,
      description: "Git over HTTP, as the node passes it on: clone, fetch and push with stock git.",
      input: { type: "object", required: ["method", "path"], properties: { method: { type: "string" }, path: { type: "string" }, query: { type: "string" }, body: { type: ["string", "null"] } } },
      run: (ctx, input) => http.serve(ctx, global(ctx), git, input),
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
        if (!commits.length) {
          const any = (await ctx.global.query("select 1 from repos where deleted is null limit 1")).length;
          return { t: "text", text: any ? "No commits yet" : "No repos yet", size: "sm", tone: "muted" };
        }
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

/// What http.js needs of repos: who owns one and may push, making one, and moving a branch.
const git = {
  BRANCH,
  repoOf,
  ownerName,
  mayPush: (ctx, r) => actsFor(asWriter(ctx), r.owner),
  defaultBranch: (names) => (names.includes("main") ? "main" : names[0]),
  /// Makes `name`, the caller's, or says why not.
  newRepo(ctx, g, name) {
    if (!NAME.test(name || "")) return "repo names are lowercase letters, digits, ., _ and -";
    newRepo(ctx, g, { name, owner: `user:${ctx.me.user}` });
    return null;
  },
  /// Moves `branch` from `base` (null if it's new) to `head` (null deletes it), only if no one moved it meanwhile.
  moveBranch(ctx, g, repo, branch, base, head) {
    g.check("select (select head from branches where repo = ? and branch = ?) is ?", [repo, branch, base], `${branch} has moved: fetch, then push again`);
    if (head) g.exec("insert or replace into branches values (?, ?, ?, ?, ?, ?)", [repo, branch, head, ctx.me.user, ctx.me.name, ctx.now()]);
    else g.exec("delete from branches where repo = ? and branch = ?", [repo, branch]);
    g.exec("insert into pushes values (?, ?, ?, ?, ?, ?, ?, ?)", [randomId(), repo, branch, base, head, ctx.me.user, ctx.me.name, ctx.now()]);
  },
};

/// The newest `many` commits across repos' main branches, each with its repo and branch: the branches pushed to last,
/// and their history, newest first. A repo whose packs can't be read is left out.
async function recent(ctx, many, repo = "") {
  const heads = await ctx.global.query(
    "select repo, branch from branches where (? = '' or repo = ?) and repo not in (select name from repos where deleted is not null) order by time desc limit ?",
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
