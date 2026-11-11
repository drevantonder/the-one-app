// Repos: git hosting as an ordinary app. Installed in global, so every repo is visible to everyone. Stock git clones and
// pushes over HTTP through the computer's local API (http.js), and a repo's packs are blobs. No git runs here: pages read
// the packs themselves (browse.js, git.js).
//
// As on GitHub, a repo is <owner>/<name>, its owner a user, a group or Everyone, by their handle (ctx.handles), so a
// handle's change moves its repos' addresses. Everyone reads and forks every repo. Its owner pushes, and its
// collaborators: a user's repo is theirs, and they add and remove collaborators; a group's is its admins', its members
// are its collaborators, and its admins add others. Everyone's is the network's admins', and only they push, since
// nobody was given a place in Everyone on purpose; they add others as collaborators. Who's which is the network's permissions (`types`, and the rows of
// `relations`, which ctx.global.may reads). Owners delete a repo, as apps are: it waits in Recently Deleted, read by no
// one, until they restore it or erase it, and its packs, for good. Rows name a repo by its id (an older repo's is the
// name it had before owners had handles) and its owner by user:<id>, group:<id> or the network, never by handle.
//
// GitHub's repos show beside them, as github.com/<owner>/<name>, each tool asking GitHub at once (github.js).

import * as browse from "./browse.js";
import * as http from "./http.js";
import * as issues from "./issues.js";
import * as github from "./github.js";
import mainScreen from "./dist/main.html" with { type: "text" };
import repoScreen from "./dist/repo.html" with { type: "text" };
import fileScreen from "./dist/file.html" with { type: "text" };
import historyScreen from "./dist/history.html" with { type: "text" };
import commitScreen from "./dist/commit.html" with { type: "text" };
import branchesScreen from "./dist/branches.html" with { type: "text" };
import issuesScreen from "./dist/issues.html" with { type: "text" };
import issueScreen from "./dist/issue.html" with { type: "text" };
import repoSettingsScreen from "./dist/repo-settings.html" with { type: "text" };
import settingsScreen from "./dist/settings.html" with { type: "text" };

import { BRANCH_HEAD, COMMIT, FILE_CHANGE, PERSON, REPO_LINE, TEXT, refusal } from "./shapes.js";

const NAME = /^[a-z0-9][a-z0-9._-]{0,63}$/;
const BRANCH = /^(?!\/)(?!.*(\.\.|\/\/|\/$|\.lock$|@\{))[A-Za-z0-9._\/@+-]{1,200}$/;

function iso(v) {
  if (Array.isArray(v)) return v.map(iso);
  if (!v || typeof v !== "object") return v;
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, typeof x === "number" && ["time", "created", "updated", "committed", "closed"].includes(k) ? new Date(x).toISOString() : iso(x)]));
}

const OWNER = { type: "string", description: "who owns it, by handle: omit for you; a group you're in, or, if you're an admin, Everyone, from groups_list" };
const REPO = { type: "string", description: "a repo as <owner>/<name>, from repos_list, like andre/notes; or a GitHub one as github.com/<owner>/<name>" };
const REF = { type: "string", description: "a branch, like main, or a commit's sha, from repos_branches or repos_log; default main, else the first branch" };
const PATH = { type: "string", description: "a path in the repo, like src/app.js or docs; omit for the whole repo" };
const SHA = { type: "string", description: "a commit's sha, full or its first 7 characters, from repos_log" };
const USER = { type: "string", description: "a user's handle, from groups_list, like sam" };
const CONFIRM = { type: "boolean", description: "true once the user agreed: it changes who may push" };
const MOVED_FOR = 30 * 24 * 60 * 60 * 1000;

const handles = (ctx) => ctx.handles;
const byId = (ctx, id) => handles(ctx).find((h) => h.id === id) || null;
const byHandle = (ctx, handle) => handles(ctx).find((h) => h.handle === handle) || null;
/// Whether `user` makes and pushes to the repos of `h`, a group or Everyone, by being in it: a member was put there on
/// purpose, unless it's automatic, where only its admins were.
const pushesAsOwner = (h, user) => (h?.automatic ? h.members?.[user] === "admin" : !!h?.members?.[user]);

/// A repo as its address and every tool name it: <owner's handle>/<name>.
const pathOf = (ctx, r) => `${byId(ctx, r.owner)?.handle ?? "unknown"}/${r.name}`;
const withPath = (ctx, r) => r && { ...r, path: pathOf(ctx, r) };

function parse(ctx, repo) {
  const m = /^([^/]+)\/([^/]+)$/.exec(String(repo || "").replace(/\.git$/, ""));
  if (!m) throw refusal(`give a repo as <owner>/<name>, like ${ctx.me.handle || "andre"}/${repo || "notes"}: repos_list has them`);
  const owner = byHandle(ctx, m[1]);
  if (!owner) throw refusal(`no one and no group has the handle ${m[1]}: groups_list has them`);
  return { owner: owner.id, name: m[2] };
}

async function repoAt(ctx, g, repo) {
  const { owner, name } = parse(ctx, repo);
  return withPath(ctx, (await g.query("select * from repos where owner = ? and name = ?", [owner, name]))[0] || null);
}

/// The repo transferred away from `owner`/`name`, while that address still leads to it, with `until`.
async function movedFrom(ctx, g, owner, name) {
  return withPath(ctx, (await g.query("select r.*, m.until from moved m join repos r on r.id = m.repo where m.owner = ? and m.name = ? and m.until > ?", [owner, name, ctx.now()]))[0] || null);
}

/// A repo by the address given: where it is, or where it went.
async function repoFrom(ctx, g, repo) {
  const { owner, name } = parse(ctx, repo);
  return (await repoAt(ctx, g, repo)) || movedFrom(ctx, g, owner, name);
}

async function named(ctx, repo) {
  const g = global(ctx);
  if (!String(repo || "").includes("/")) {
    const same = (await g.query("select * from repos where name = ? and deleted is null", [repo || ""])).map((r) => pathOf(ctx, r));
    if (same.length) throw refusal(`give a repo as <owner>/<name>: ${same.join(" or ")}`);
  }
  const r = await live(ctx, repo);
  if (r) return r;
  const all = (await g.query("select owner, name from repos where deleted is null order by name")).map((r) => pathOf(ctx, r));
  throw refusal(`no repo ${repo}: ${all.length ? `there's ${all.join(", ")}` : "there are none yet; repos_create makes one"}`);
}

async function live(ctx, repo) {
  const r = await repoFrom(ctx, global(ctx), repo);
  if (r?.deleted) throw refusal(`${r.path} was deleted: its owners restore it from Recently Deleted`);
  return r;
}

const sql = (v) => `'${String(v).replace(/'/g, "''")}'`;

const randomId = () => [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");

const object = (r) => `repo:${r.id}`;

/// Who holds a role on a repo by owning it.
function holders(ctx, owner) {
  const h = byId(ctx, owner);
  return !h?.members ? [[owner, "owner"]] : [[`${owner}#admin`, "owner"], ...(h.automatic ? [] : [[`${owner}#member`, "collaborator"]])];
}

const give = (ctx, g, owner, id) => {
  for (const [subject, relation] of holders(ctx, owner)) g.exec("insert or ignore into relations (subject, relation, object, by, time) values (?, ?, ?, ?, ?)", [subject, relation, object({ id }), ctx.me.user, ctx.now()]);
};

/// Writes a new repo, or a fork of `from` (a row) with its branches and history as they are now: its owner's, a user
/// or the admins of a group or Everyone, and a group's members its collaborators.
function newRepo(ctx, g, { name, owner, from = null }) {
  const id = randomId();
  g.exec(`insert into repos (id, owner, name, "from", created, user) values (?, ?, ?, ?, ?, ?)`, [id, owner, name, from?.id ?? null, ctx.now(), ctx.me.user]);
  g.exec("delete from moved where owner = ? and name = ?", [owner, name]);
  give(ctx, g, owner, id);
  if (from) {
    g.exec(`insert into branches select ?, branch, head, ?, ?, ? from branches where repo = ?`, [id, ctx.me.user, ctx.me.user_name || ctx.me.name, ctx.now(), from.id]);
    g.exec("insert or ignore into packs select ?, hash, time from packs where repo = ?", [id, from.id]);
    g.exec("insert into labels select ?, name, color, description from labels where repo = ?", [id, from.id]);
  } else {
    for (const l of issues.LABELS) g.exec("insert into labels values (?, ?, ?, ?)", [id, ...l]);
  }
  return id;
}

const ownerName = (ctx, owner) => byId(ctx, owner)?.name || "someone who left";

function ownerFor(ctx, handle) {
  if (!handle || handle === ctx.me.handle) return `user:${ctx.me.user}`;
  const h = byHandle(ctx, handle);
  if (!h) throw refusal(`no one and no group has the handle ${handle}`);
  if (h.id === `user:${ctx.me.user}` || pushesAsOwner(h, ctx.me.user)) return h.id;
  throw refusal(h.automatic ? `only ${h.name}'s admins make its repos` : h.members ? `you're not in ${h.name}: only its members make its repos` : `only ${h.name} makes repos as ${handle}`);
}

async function owned(ctx, repo) {
  const r = await repoFrom(ctx, global(ctx), repo);
  if (!r) await named(ctx, repo);
  if (!(await global(ctx).may("manage", object(r)))) throw refusal(`only ${byId(ctx, r.owner)?.members ? `${ownerName(ctx, r.owner)}'s admins` : ownerName(ctx, r.owner)} ${r.deleted ? "restore or erase" : "manage"} ${r.path}`);
  return r;
}

const deletedOf = (r) => (r.deleted ? { time: r.deleted, by: r.deleted_by } : null);

function global(ctx) {
  if (!ctx.global) throw refusal("repos lives in global: install it there");
  return ctx.global;
}

async function rights(g, r) {
  const [yours, push] = await g.may([["manage", object(r)], ["push", object(r)]]);
  return { yours, push };
}

/// Where git reaches a repo, the same on every computer: <the network's address>/repos/<owner>/<name>.git; null in a
/// network with no address yet.
const address = (ctx, path) => (ctx.address ? `${ctx.address}/repos/${path}.git` : null);

async function paths(ctx, g, ids) {
  const want = [...new Set(ids.filter(Boolean))];
  if (!want.length) return {};
  const rows = await g.query(`select id, owner, name from repos where id in (${want.map(() => "?").join(", ")})`, want);
  return Object.fromEntries(rows.map((r) => [r.id, pathOf(ctx, r)]));
}

const forksOf = async (ctx, g, r) => (await g.query(`select owner, name from repos where "from" = ? and deleted is null order by name`, [r.id])).map((f) => pathOf(ctx, f));

const who = (ctx, id) => ({ handle: byId(ctx, id)?.handle ?? null, name: ownerName(ctx, id) });

async function ownList(ctx) {
  const g = global(ctx);
  const repos = await g.query(
    `select r.id, r.owner, r.name, r."from", r.created, r.deleted, r.deleted_by,
            coalesce((select branch from branches b where b.repo = r.id and b.branch = 'main'),
                     (select min(branch) from branches b where b.repo = r.id)) as branch,
            (select count(*) from branches b where b.repo = r.id) as branches,
            (select max(time) from branches b where b.repo = r.id) as updated,
            (select count(*) from issues i where i.repo = r.id and i.state = 'open') as open_issues
       from repos r order by coalesce(updated, r.created) desc`,
  );
  const from = await paths(ctx, g, repos.map((r) => r.from));
  const may = await g.may(repos.flatMap((r) => [["manage", object(r)], ["push", object(r)]]));
  return iso(repos.map((r, i) => ({
    repo: pathOf(ctx, r), name: r.name, owner: who(ctx, r.owner), from: from[r.from] ?? null, branch: r.branch, branches: r.branches,
    created: r.created, updated: r.updated, open_issues: r.open_issues, deleted: deletedOf(r), yours: may[2 * i], push: may[2 * i + 1],
  })));
}

const app = {
  name: "repos",
  title: "Repos",
  tagline: "Shared code",
  icon: "git-branch",
  color: "#5856d6",
  advanced: true,
  description: "Git hosting for you and your agents: clone, push and suggest changes",
  instructions: `Repos is git hosting for the user and their agents, as on GitHub. A repo is <owner>/<name>, its owner a user, a group or Everyone by their handle (groups_list has them), and its address is \`<their One App's address>/repos/<owner>/<name>.git\` (repos_get gives it, and its page shows it), the same on every computer: \`git clone\`, \`git push\` and \`git fetch\` work with it as with any git host on any computer where it is installed and in the network; pushing to a name that isn't a repo yet makes it, under that owner (you, or a group you're in). Everyone reads and forks every repo. A repo's owner pushes, and its collaborators: a user adds and removes the collaborators of their own repos (repos_add_collaborator), and a group's members are collaborators of its repos, whose admins own them; Everyone's are the network's admins' alone, who add others as collaborators. An owner gives a repo to a group they're in or Everyone with repos_transfer; its old address keeps working for 30 days. Anyone else forks it (repos_fork, by default to <you>/<same name>), pushes to the fork, and tells its owner, who fetches from it. Each repo has issues, as on GitHub, numbered from #1: anyone opens one (repos_open_issue) and comments (repos_comment); its author edits and closes it, and those who push to the repo close, label and assign any (repos_issues finds them). GitHub's repos show beside them, as github.com/<owner>/<name>: those the user's own GitHub accounts see (repos_accounts, repos_sign_in) and those of GitHub organizations connected for their groups. Reading and the issue tools work on them the same, asking GitHub at once, and what they write goes to GitHub; clone one from GitHub's own address (repos_get gives it).`,
  vault: github.VAULT,

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
      `create table indexes (pack text primary key, name text not null, idx text not null, size integer not null);`,
      // Repos as <owner>/<name>: each its own id (an older one's, its name), so two owners may each have one of a name.
      // One the network's admins owned is now its maker's.
      `create table repos_owned (id text primary key, owner text not null, name text not null, "from" text, created integer not null,
                                 user text not null, deleted integer, deleted_by text, unique (owner, name));
      insert into repos_owned select name, case when owner = 'admins' then 'user:' || user else owner end, name, "from", created, user, deleted, deleted_by from repos;
      drop table repos;
      alter table repos_owned rename to repos;
      create table collaborators (repo text not null, user text not null, by text not null, time integer not null, primary key (repo, user));`,
      // Issues, as GitHub's: numbered from 1 in each repo (repos.issues is the last number given, which a deleted
      // issue's keeps), each with its labels, assignees and comments; and each repo's labels, GitHub's to start with.
      `alter table repos add column issues integer not null default 0;
      create table issues (repo text not null, number integer not null, title text not null, body text not null, state text not null,
                           state_reason text, user text not null, created integer not null, updated integer not null, closed integer,
                           closed_by text, primary key (repo, number));
      create table labels (repo text not null, name text not null collate nocase, color text not null, description text not null,
                           primary key (repo, name));
      create table issue_labels (repo text not null, number integer not null, label text not null collate nocase, primary key (repo, number, label));
      create table issue_assignees (repo text not null, number integer not null, user text not null, primary key (repo, number, user));
      create table comments (id text primary key, repo text not null, number integer not null, user text not null, body text not null,
                             created integer not null, updated integer not null);
      create index comments_issue on comments (repo, number, created);
      insert into labels select r.id, d.column1, d.column2, d.column3 from repos r, (values ${issues.LABELS.map((l) => `(${l.map(sql).join(", ")})`).join(", ")}) d;`,
      github.SCHEMA,
      // Who holds a role on a repo, as the network's permissions' rows (subject, relation, object, until), and who gave
      // it when: its owner (a user, or a group's admins), a group's members as its collaborators, and the
      // collaborators its owners added.
      `create table relations (subject text not null, relation text not null, object text not null, until integer, by text, time integer,
                               primary key (subject, relation, object));
      create index relations_by_object on relations (object, relation);
      insert into relations select 'user:' || substr(owner, 6), 'owner', 'repo:' || id, null, user, created from repos where owner like 'user:%';
      insert into relations select owner || '#admin', 'owner', 'repo:' || id, null, user, created from repos where owner like 'group:%';
      insert into relations select owner || '#member', 'collaborator', 'repo:' || id, null, user, created from repos where owner like 'group:%';
      insert or ignore into relations select 'user:' || user, 'collaborator', 'repo:' || repo, null, by, time from collaborators;
      drop table collaborators;`,
      `update relations set subject = 'user:' || substr(subject, 8) where subject like 'person:%';`,
      // Sign-offs on a repo's commits, as git refs (refs/signoff/…), which aren't branches: one-dev's.
      `create table signoffs (repo text not null, ref text not null, head text not null, primary key (repo, ref));`,
      // Addresses repos were transferred away from, which lead to them until `until` or until a repo is made there.
      `create table moved (owner text not null, name text not null, repo text not null, until integer not null, primary key (owner, name));`,
    ],
  },

  // A repo's roles: its owner pushes and manages it (its collaborators; deleting, restoring and erasing it; deleting
  // its issues and anyone's comments), and its collaborators push (and close, edit, label and assign any of its
  // issues, and manage its labels). Everyone reads and forks every repo, so nothing says that.
  types: {
    repo: { permissions: ["push", "manage"], roles: { owner: ["push", "manage"], collaborator: ["push"] } },
  },

  tools: {
    list: {
      title: "Every repo",
      reads: true,
      description: "Every repo, the latest moved first: its <owner>/<name>, its owner's handle and name, where it was forked from, its default branch (main, or else its first), how many branches it has, when it last moved, how many open issues it has, and whether you push to it. Deleted ones say when, and wait in Recently Deleted for repos_restore. Then GitHub's that you see, as github.com/<owner>/<name>, with github: { url, private, fork }; source github lists only those, and says what GitHub refused.",
      input: { type: "object", properties: { source: { type: "string", enum: ["one", "github"], description: "one: only this network's own repos; github: only GitHub's; omit for both" } } },
      output: { type: "array", items: REPO_LINE, description: "the latest moved first" },
      async run(ctx, { source } = {}) {
        if (source === "github") return github.list(ctx);
        const own = await ownList(ctx);
        if (source === "one") return own;
        return [...own, ...(await github.list(ctx).catch((err) => { throw new Error(`${err.message} (source one lists only this network's repos)`); }))];
      },
    },
    get: {
      title: "A repo",
      reads: true,
      description: "A repo: its address for git clone and push, the same on every computer, its owner, its branches with their heads, its forks, its last 30 pushes, and the packs that hold its objects, oldest first.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      output: {
        type: "object",
        required: ["repo", "name", "branches"],
        properties: {
          repo: { type: "string", description: "<owner>/<name>, or github.com/<owner>/<name>" },
          name: { type: "string" },
          address: { type: ["string", "null"], description: "for git clone and push; null in a network with no address yet" },
          owner: PERSON,
          from: { type: ["string", "null"] },
          created: { type: "string", description: "ISO 8601" },
          yours: { type: "boolean" },
          push: { type: "boolean" },
          branches: { type: "array", items: { type: "object", required: ["branch", "head"], additionalProperties: true } },
          packs: { type: "array", items: { type: "string" } },
          forks: { type: "array", items: { type: "string" } },
          pushes: { type: "array", items: { type: "object", additionalProperties: true }, description: "the last 30" },
          github: { type: "object", additionalProperties: true },
        },
        additionalProperties: true,
      },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const branches = await g.query("select branch, head, user, name, time from branches where repo = ? order by branch", [r.id]);
        const packs = (await g.query("select hash from packs where repo = ? order by time, hash", [r.id])).map((p) => p.hash);
        const pushes = (await g.query("select branch, base, head, user, name, time from pushes where repo = ? order by time desc limit 30", [r.id]));
        const from = (await paths(ctx, g, [r.from]))[r.from] ?? null;
        return iso({ repo: r.path, name: r.name, address: address(ctx, r.path), owner: who(ctx, r.owner), from, created: r.created, ...(await rights(g, r)), branches, packs, forks: await forksOf(ctx, g, r), pushes });
      },
    },
    tree: {
      title: "A folder of a repo",
      reads: true,
      description: "A folder of a repo at a branch or commit: its files and folders, its README, the latest commit, the repo's forks and how many open issues it has. repos_file reads a file.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "a folder's path, like src; omit for the top" } } },
      output: {
        type: "object",
        required: ["repo", "entries"],
        properties: {
          repo: { type: "string" },
          name: { type: "string" },
          branches: { type: "array", items: { type: "string" } },
          ref: { type: ["string", "null"] },
          branch: { type: ["string", "null"] },
          path: { type: "string" },
          sha: { type: "string" },
          commit: COMMIT,
          latest: COMMIT,
          entries: { type: "array", items: { type: "object", required: ["name", "type"], properties: { name: { type: "string" }, type: { type: "string", enum: ["blob", "tree", "commit"] }, mode: { type: ["string", "null"] }, sha: { type: "string" } } } },
          readme: { type: ["object", "null"], description: "{ name, text }", additionalProperties: true },
          forks: { type: "array", items: { type: "string" } },
          open_issues: { type: "integer" },
          empty: { type: "boolean", description: "true when the repo has no code yet" },
          file: { type: "boolean", description: "true when path is a file, not a folder" },
          deleted: { type: ["object", "null"], additionalProperties: true },
        },
        additionalProperties: true,
      },
      async run(ctx, input) {
        const g = global(ctx);
        const r = (await repoAt(ctx, g, input.repo)) || (await named(ctx, input.repo));
        const open = (await g.query("select count(*) as n from issues where repo = ? and state = 'open'", [r.id]))[0].n;
        const about = { owner: who(ctx, r.owner), ...(await rights(g, r)), from: (await paths(ctx, g, [r.from]))[r.from] ?? null, open_issues: open };
        if (r.deleted) return iso({ repo: r.path, name: r.name, ...about, deleted: deletedOf(r), branches: [], entries: [] });
        const t = await browse.tree(ctx, g, { ...input, repo: r });
        return iso({ ...t, name: r.name, ...about, forks: await forksOf(ctx, g, r) });
      },
    },
    file: {
      title: "A file of a repo",
      reads: true,
      description: "A file of a repo at a branch or commit: its text, or whether it's binary or too big to show (then git clone it).",
      input: { type: "object", required: ["repo", "path"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "the file's path, from repos_tree, like src/app.js" } } },
      output: {
        type: "object",
        required: ["repo", "path"],
        properties: {
          repo: { type: "string" },
          ref: { type: ["string", "null"] },
          branch: { type: ["string", "null"] },
          sha: { type: "string" },
          path: { type: "string" },
          branches: { type: "array", items: { type: "string" } },
          mode: { type: ["string", "null"] },
          latest: COMMIT,
          size: { type: "integer", description: "bytes" },
          text: { type: "string", description: "its text, unless it is binary or too big" },
          binary: { type: "boolean" },
          too_big: { type: "boolean" },
          submodule: { type: "string", description: "the commit it points to, for a submodule" },
        },
      },
      run: async (ctx, input) => iso(await browse.file(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) })),
    },
    log: {
      title: "A repo's history",
      reads: true,
      description: "A repo's history, as git log gives it: newest first, 25 commits a page, of the whole repo or of one path, each with its sha, subject, author and time; more says whether there's another page.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: PATH, page: { type: "integer", description: "which 25: 0 (the default) for the newest, 1 for the 25 before them, and on while more is true" } } },
      output: {
        type: "object",
        required: ["repo", "commits", "more"],
        properties: {
          repo: { type: "string" },
          ref: { type: ["string", "null"] },
          branch: { type: ["string", "null"] },
          sha: { type: "string" },
          path: { type: "string" },
          page: { type: "integer" },
          commits: { type: "array", items: COMMIT },
          more: { type: "boolean", description: "whether there is another page" },
        },
      },
      run: async (ctx, input) => iso(await browse.log(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) })),
    },
    commit: {
      title: "A commit",
      reads: true,
      description: "A commit: its message, author, time, parents, and each file it changed with +/- counts and diff hunks (a big file's diff is left for repos_diff).",
      input: { type: "object", required: ["repo", "sha"], properties: { repo: REPO, sha: SHA } },
      output: {
        type: "object",
        required: ["repo", "sha", "subject", "files"],
        properties: { ...COMMIT.properties, repo: { type: "string" }, files: { type: "array", items: FILE_CHANGE } },
      },
      run: async (ctx, input) => iso(await browse.commit(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) })),
    },
    diff: {
      title: "One file's whole diff in a commit",
      reads: true,
      description: "One file's whole diff in a commit, against its first parent, as hunks with + and - lines: for a file repos_commit left out as too big.",
      input: { type: "object", required: ["repo", "sha", "path"], properties: { repo: REPO, sha: SHA, path: { type: "string", description: "the file's path, from repos_commit's files, like src/app.js" } } },
      output: FILE_CHANGE,
      run: async (ctx, input) => browse.diff(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) }),
    },
    latest: {
      title: "The last commit that changed each entry",
      reads: true,
      description: "The last commit that changed each entry of a folder, by the entry's name: its sha, subject, author and time.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, ref: REF, path: { type: "string", description: "a folder's path, like src; omit for the top" } } },
      output: {
        type: "object",
        description: "each entry's name → its last commit",
        additionalProperties: { type: "object", required: ["sha", "subject"], properties: { sha: { type: "string" }, subject: { type: "string" }, time: { type: "string" }, author: { type: "string" } } },
      },
      run: async (ctx, input) => iso(await browse.latest(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) })),
    },
    branches: {
      title: "A repo's branches",
      reads: true,
      description: "A repo's branches, each with its head commit's sha, subject, author and time, and who pushed it when.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      output: { type: "array", items: BRANCH_HEAD },
      run: async (ctx, input) => iso(await browse.branches(ctx, global(ctx), { ...input, repo: await named(ctx, input.repo) })),
    },
    collaborators: {
      title: "Who pushes to a repo",
      reads: true,
      description: "Who pushes to a repo: its owner; for a group's, its members (its admins own it); for Everyone's, the network's admins; and the collaborators its owners added, each with who added them when. yours says whether you own it, to add and remove them; users, whom you could add.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      output: {
        type: "object",
        required: ["repo", "owner", "yours"],
        properties: {
          repo: { type: "string" },
          owner: PERSON,
          group: { type: "boolean", description: "whether a group or Everyone owns it" },
          members: { type: "array", items: { type: "object", additionalProperties: true }, description: "those who push by being in the group or Everyone, each { handle, name, role }" },
          collaborators: { type: "array", items: { type: "object", additionalProperties: true }, description: "the collaborators its owners added, each { handle, name, by, time }" },
          yours: { type: "boolean", description: "whether you may add and remove collaborators" },
          users: { type: "array", items: PERSON, description: "whom you could add" },
        },
      },
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await named(ctx, repo);
        const group = byId(ctx, r.owner)?.members ? byId(ctx, r.owner) : null;
        const pushing = Object.entries(group?.members || {}).filter(([user]) => pushesAsOwner(group, user));
        const members = pushing.map(([user, role]) => ({ ...who(ctx, `user:${user}`), role }));
        const added = (await g.query("select subject, by, time from relations where object = ? and relation = 'collaborator' and subject like 'user:%' order by time", [object(r)]))
          .map((c) => ({ ...who(ctx, c.subject), by: ownerName(ctx, `user:${c.by}`), time: c.time }));
        const yours = await g.may("manage", object(r));
        const taken = new Set([r.owner.slice(5), ...pushing.map(([user]) => user), ...added.map((c) => byHandle(ctx, c.handle)?.id.slice(5))]);
        const users = yours ? handles(ctx).filter((h) => h.id.startsWith("user:") && !h.removed && !taken.has(h.id.slice(5))).map((h) => ({ handle: h.handle, name: h.name })) : [];
        return iso({ repo: r.path, owner: who(ctx, r.owner), group: !!group, members, collaborators: added, yours, users });
      },
    },
    add_collaborator: {
      title: "Let someone push to a repo",
      reads: false,
      confirm: true,
      description: "Let someone push to a repo you own (a group's you're an admin of, or yours), by their handle.",
      input: { type: "object", required: ["repo", "user"], properties: { repo: REPO, user: USER, confirm: CONFIRM } },
      output: TEXT("that they push now, in words"),
      async run(ctx, { repo, user }) {
        const r = await owned(ctx, repo);
        const h = byHandle(ctx, user);
        if (!h || !h.id.startsWith("user:") || h.removed) throw refusal(`no one has the handle ${user}: groups_list has them`);
        const id = h.id.slice(5);
        if (r.owner === h.id) throw refusal(`${h.name} owns ${r.path}`);
        if (pushesAsOwner(byId(ctx, r.owner), id)) throw refusal(`${h.name} is in ${ownerName(ctx, r.owner)}, so pushes to ${r.path} already`);
        global(ctx).exec("insert or ignore into relations (subject, relation, object, by, time) values (?, 'collaborator', ?, ?, ?)", [`user:${id}`, object(r), ctx.me.user, ctx.now()]);
        return `${h.name} pushes to ${r.path} now.`;
      },
    },
    remove_collaborator: {
      title: "Stop someone pushing to a repo",
      reads: false,
      confirm: true,
      description: "Stop a collaborator you added pushing to a repo you own, by their handle. A group's members stay its collaborators while they're in it.",
      input: { type: "object", required: ["repo", "user"], properties: { repo: REPO, user: USER, confirm: CONFIRM } },
      output: TEXT("that they no longer push, in words"),
      async run(ctx, { repo, user }) {
        const g = global(ctx);
        const r = await owned(ctx, repo);
        const h = byHandle(ctx, user);
        if (!h || !h.id.startsWith("user:")) throw refusal(`no one has the handle ${user}`);
        const id = h.id.slice(5);
        const row = [`user:${id}`, object(r)];
        if (!(await g.query("select 1 from relations where subject = ? and relation = 'collaborator' and object = ?", row)).length) {
          throw refusal(pushesAsOwner(byId(ctx, r.owner), id) ? `${h.name} pushes to ${r.path} as a member of ${ownerName(ctx, r.owner)}: take them out of the group to stop that` : `${h.name} isn't a collaborator of ${r.path}`);
        }
        g.exec("delete from relations where subject = ? and relation = 'collaborator' and object = ?", row);
        return `${h.name} no longer pushes to ${r.path}.`;
      },
    },
    create: {
      title: "Create a repo",
      reads: false,
      description: "Create an empty repo, owned by you (the default) or a group you're in, and give the git push that fills it. Pushing to an address that isn't a repo yet makes one too.",
      input: { type: "object", required: ["name"], properties: { name: { type: "string", description: "lowercase letters, digits, ., _ and -, like notes" }, owner: OWNER } },
      output: TEXT("the repo's path, and the git push that fills it"),
      async run(ctx, { name, owner }) {
        const g = global(ctx);
        const id = ownerFor(ctx, owner);
        if (!NAME.test(name || "")) throw refusal("repo names are lowercase letters, digits, ., _ and -");
        const path = `${byId(ctx, id).handle}/${name}`;
        if (await repoAt(ctx, g, path)) throw refusal(`${path} already exists`);
        newRepo(ctx, g, { name, owner: id });
        return address(ctx, path) ? `Created ${path}: git push ${address(ctx, path)} main` : `Created ${path}.`;
      },
    },
    fork: {
      title: "Fork a repo",
      reads: false,
      description: "Fork a repo as a new one of yours (or a group's you're in), to push changes you'd suggest to its owner: its branches as they are now, and its history. By default it's <you>/<the same name>.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO, name: { type: "string", description: "the fork's name, by default the repo's: lowercase letters, digits, ., _ and -" }, owner: OWNER } },
      output: TEXT("the fork's path, and the git clone of it"),
      async run(ctx, { repo, name, owner }) {
        const g = global(ctx);
        const from = await named(ctx, repo);
        const id = ownerFor(ctx, owner);
        name ||= from.name;
        if (!NAME.test(name)) throw refusal("repo names are lowercase letters, digits, ., _ and -");
        const path = `${byId(ctx, id).handle}/${name}`;
        if (await repoAt(ctx, g, path)) throw refusal(`${path} already exists: give the fork another name`);
        newRepo(ctx, g, { name, owner: id, from });
        return address(ctx, path) ? `Forked ${from.path} as ${path}: git clone ${address(ctx, path)}` : `Forked ${from.path} as ${path}.`;
      },
    },
    transfer: {
      title: "Transfer a repo",
      reads: false,
      confirm: true,
      description: "Give a repo you own to yourself, a group you're in, or, if you're an admin, Everyone. Its history, issues and added collaborators stay with it, and who owns it changes who may push. Its old address keeps working for 30 days, with git saying where it went, unless a repo is made there first.",
      input: { type: "object", required: ["repo", "owner"], properties: { repo: REPO, owner: OWNER, confirm: CONFIRM } },
      output: TEXT("where it is now, and when its old address stops working"),
      async run(ctx, { repo, owner }) {
        const g = global(ctx);
        const r = await owned(ctx, repo);
        if (r.deleted) throw refusal(`${r.path} was deleted: restore it first`);
        const id = ownerFor(ctx, owner);
        if (id === r.owner) throw refusal(`${r.path} is already theirs`);
        const to = `${byId(ctx, id).handle}/${r.name}`;
        if (await repoAt(ctx, g, to)) throw refusal(`${to} already exists`);
        const until = ctx.now() + MOVED_FOR;
        for (const [subject, relation] of holders(ctx, r.owner)) g.exec("delete from relations where subject = ? and relation = ? and object = ?", [subject, relation, object(r)]);
        give(ctx, g, id, r.id);
        g.exec("update repos set owner = ? where id = ?", [id, r.id]);
        g.exec("delete from moved where owner = ? and name = ?", [id, r.name]);
        g.exec("insert or replace into moved values (?, ?, ?, ?)", [r.owner, r.name, r.id, until]);
        return `${r.path} is now ${to}. Its old address works until ${new Date(until).toISOString().slice(0, 10)}: git remote set-url origin ${address(ctx, to) ?? to}`;
      },
    },
    delete: {
      title: "Delete a repo",
      reads: false,
      confirm: true,
      description: "Delete a repo you own: no one can read, clone or push to it, and it waits in Recently Deleted, where its owners restore it (repos_restore) or erase it for good.",
      input: { type: "object", required: ["repo"], properties: { repo: REPO } },
      output: TEXT("that it's in Recently Deleted, in words"),
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (r.deleted) throw refusal(`${r.path} is already in Recently Deleted`);
        global(ctx).exec("update repos set deleted = ?, deleted_by = ? where id = ? and deleted is null", [ctx.now(), ctx.me.user_name || ctx.me.name, r.id]);
        return `Deleted ${r.path}: it's in Recently Deleted.`;
      },
    },
    restore: {
      title: "Restore a deleted repo",
      reads: false,
      description: "Restore a repo you own from Recently Deleted, as it was.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string", description: "a deleted repo as <owner>/<name>, from repos_list" } } },
      output: TEXT("that it's restored, in words"),
      async run(ctx, { repo }) {
        const r = await owned(ctx, repo);
        if (!r.deleted) throw refusal(`${r.path} isn't deleted`);
        global(ctx).exec("update repos set deleted = null, deleted_by = null where id = ?", [r.id]);
        return `Restored ${r.path}.`;
      },
    },
    erase: {
      title: "Erase a deleted repo for good",
      reads: false,
      confirm: true,
      description: "Erase a deleted repo you own for good: its branches, history and the packs no other repo shares are gone, and its name is free again.",
      input: { type: "object", required: ["repo"], properties: { repo: { type: "string", description: "a deleted repo as <owner>/<name>, from repos_list" } } },
      output: TEXT("that it's erased, in words"),
      async run(ctx, { repo }) {
        const g = global(ctx);
        const r = await owned(ctx, repo);
        if (!r.deleted) throw refusal(`delete ${r.path} before erasing it`);
        const packs = await g.query("select hash from packs where repo = ?", [r.id]);
        // Erased, it's gone, and its name is free again; its forks keep their code, no longer forked from it.
        for (const p of packs) await http.forgetUnshared(g, r.id, p.hash);
        for (const table of ["branches", "signoffs", "packs", "pushes", "issues", "labels", "issue_labels", "issue_assignees", "comments"]) g.exec(`delete from ${table} where repo = ?`, [r.id]);
        g.exec("delete from relations where object = ?", [object(r)]);
        g.exec("delete from repos where id = ?", [r.id]);
        g.exec(`update repos set "from" = null where "from" = ?`, [r.id]);
        return `Erased ${r.path}.`;
      },
    },
    ...issues.tools({ global, named, object, rights, who, byHandle, handles, randomId, iso, REPO }),
    ...github.tools({ global }),
    _address: {
      title: "Where git reaches repos (for the screens)",
      reads: true,
      description: "Where git reaches repos, for the screens: base, <address>/repos/ (null in a network with no address yet), and the caller's handle.",
      input: { type: "object", properties: {} },
      output: {
        type: "object",
        properties: { base: { type: ["string", "null"], description: "<address>/repos/, null in a network with no address yet" }, handle: { type: ["string", "null"], description: "the caller's handle" } },
      },
      run: (ctx) => ({ base: ctx.address ? `${ctx.address}/repos/` : null, handle: ctx.me.handle || null }),
    },
    _http: {
      title: "Git over HTTP (for onet)",
      reads: false,
      description: "Git over HTTP, as a computer's onet passes it on: clone, fetch and push with stock git.",
      input: { type: "object", required: ["method", "path"], properties: { method: { type: "string" }, path: { type: "string" }, query: { type: "string" }, body: { type: ["string", "null"] } } },
      output: { type: "object", description: "the response git is given: status, headers and body", additionalProperties: true },
      run: (ctx, input) => http.serve(ctx, global(ctx), git, input),
    },
  },

  screens: {
    main: { title: "Repos", html: mainScreen },
    repo: { title: "Repo", html: repoScreen },
    file: { title: "File", html: fileScreen },
    history: { title: "History", html: historyScreen },
    commit: { title: "Commit", html: commitScreen },
    branches: { title: "Branches", html: branchesScreen },
    issues: { title: "Issues", html: issuesScreen },
    issue: { title: "Issue", html: issueScreen },
    repo_settings: { title: "Settings", html: repoSettingsScreen },
    settings: { title: "Settings", html: settingsScreen },
    // Where GitHub sends an admin back once the network's GitHub App is made: Settings, finishing that.
    github_app: { title: "Settings", html: settingsScreen },
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
          // By id, which a handle's change leaves as it is.
          choices: async (ctx) => [{ value: "", title: "All Repos" }, ...(ctx.global ? await ctx.global.query("select id, owner, name from repos where deleted is null order by name") : []).map((r) => ({ value: r.id, title: pathOf(ctx, r) }))],
        },
      },
      async render(ctx) {
        if (!ctx.global) return { t: "text", text: "No repos here.", tone: "muted" };
        const many = { small: 1, medium: 2, large: 4, xl: 6 }[ctx.size] || 2;
        const { commits, week, left } = await recent(ctx, many, ctx.options.repo);
        const problem = left.length ? { t: "text", text: `Couldn't read ${left.join(", ")}`, size: "xs", tone: "bad", lines: 1 } : null;
        if (!commits.length) {
          const any = (await ctx.global.query("select 1 from repos where deleted is null limit 1")).length;
          return problem || { t: "text", text: any ? "No commits yet." : "No repos yet.", tone: "muted" };
        }
        const total = week.reduce((a, n) => a + n, 0);
        const now = ctx.now(), DAY = 86400000;
        const days = week.map((_, i) => new Date(now - (6 - i) * DAY).toLocaleDateString("en-US", { weekday: "narrow" }));
        const link = (c) => ({ screen: "commit", params: { repo: c.repo, sha: c.sha } });
        const muted = (text, extra = {}) => ({ t: "text", text, size: "sm", tone: "muted", lines: 1, ...extra });
        const subject = (c, lines) => ({ t: "text", text: c.subject, weight: "medium", lines });
        const about = (c, who) => ({ t: "row", gap: 6, c: [muted(who), { t: "col", grow: true, c: [] }, { t: "time", at: c.time, size: "sm", tone: "muted" }] });
        const commit = (c, lines) => ({ t: "col", gap: 2, link: link(c), c: [subject(c, lines), about(c, `${c.repo} · ${c.author}`)] });
        const activity = { t: "row", gap: 8, c: [muted(`${total} this week`), { t: "col", grow: true, c: [] }, { t: "spark", values: week, tone: "muted", w: 64, h: 14, alt: "Commits a day this week" }] };
        const spacer = { t: "col", grow: true, gap: 0, c: [] };
        const spread = (nodes) => nodes.flatMap((n, i) => (i ? [spacer, n] : [n]));
        const shown = (tree) => (problem ? { ...tree, c: [...tree.c, problem] } : tree);
        // Small is one fact: the latest commit.
        if (ctx.size === "small") return shown({ t: "col", gap: 4, link: link(commits[0]), c: [subject(commits[0], 4), spacer, about(commits[0], commits[0].repo)] });
        if (ctx.size === "medium") return shown({ t: "col", gap: 4, c: spread([...commits.map((c) => commit(c, 1)), activity]) });
        const chart = { t: "chart", values: week, labels: days, tone: "muted", h: 36, alt: "Commits a day this week" };
        const summary = { t: "col", gap: 4, c: [muted(`${total} commits this week`), chart] };
        if (ctx.size === "large") return shown({ t: "col", gap: 4, c: spread([...commits.map((c) => commit(c, 2)), summary]) });
        // Extra large: the commits beside the week. The row grows inside a column: a row at the top of a tree doesn't
        // take the card's height on every client.
        return shown({ t: "col", gap: 4, c: [{ t: "row", grow: true, gap: 24, c: [{ t: "col", grow: true, gap: 8, c: commits.map((c) => commit(c, 1)) }, { t: "col", w: 260, gap: 4, c: [muted(`${total} commits this week`), { ...chart, h: 160 }] }] }] });
      },
    },
  },
};

export default app;

for (const [name, tool] of Object.entries(app.tools)) {
  const own = tool.run;
  if (github.run[name]) tool.run = (ctx, input = {}) => (github.isGithub(input.repo) ? github.run[name](ctx, input) : own(ctx, input));
  else if (github.elsewhere[name]) tool.run = (ctx, input = {}) => (github.isGithub(input.repo) ? github.refuse(ctx, name, input.repo) : own(ctx, input));
}

const git = {
  BRANCH,
  find: async (ctx, g, owner, name) => {
    const h = byHandle(ctx, owner);
    if (!h) return { why: `no one and no group has the handle ${owner}` };
    const r = withPath(ctx, (await g.query("select * from repos where owner = ? and name = ?", [h.id, name]))[0] || null);
    if (r) return { r };
    const moved = await movedFrom(ctx, g, h.id, name);
    return { r: moved, moved: moved && `${owner}/${name} moved to ${moved.path}, and this address stops working on ${new Date(moved.until).toISOString().slice(0, 10)}: git remote set-url origin ${address(ctx, moved.path) ?? moved.path}` };
  },
  ownerName,
  object,
  defaultBranch: (names) => (names.includes("main") ? "main" : names[0]),
  newRepo(ctx, g, owner, name) {
    if (!NAME.test(name || "")) return { why: "repo names are lowercase letters, digits, ., _ and -" };
    let id;
    try {
      id = ownerFor(ctx, owner);
    } catch (err) {
      return { why: err.message };
    }
    return { r: { id: newRepo(ctx, g, { name, owner: id }), owner: id, name, path: `${owner}/${name}` } };
  },
  moveBranch(ctx, g, repo, branch, base, head) {
    const name = ctx.me.user_name || ctx.me.name;
    g.check("select (select head from branches where repo = ? and branch = ?) is ?", [repo, branch, base], `${branch} has moved: fetch, then push again`);
    if (head) g.exec("insert or replace into branches values (?, ?, ?, ?, ?, ?)", [repo, branch, head, ctx.me.user, name, ctx.now()]);
    else g.exec("delete from branches where repo = ? and branch = ?", [repo, branch]);
    g.exec("insert into pushes values (?, ?, ?, ?, ?, ?, ?, ?)", [randomId(), repo, branch, base, head, ctx.me.user, name, ctx.now()]);
  },
};

/// The latest commits of the repos that moved last, and the repos whose packs couldn't be read, which are left out.
async function recent(ctx, many, repo = "") {
  const heads = await ctx.global.query(
    `select r.id, r.owner, r.name, b.branch from branches b join repos r on r.id = b.repo
      where (? = '' or r.id = ?) and r.deleted is null order by b.time desc limit ?`,
    [repo || "", repo || "", Math.max(many, 8)],
  );
  const all = [], left = new Set();
  for (const h of heads) {
    const r = withPath(ctx, h);
    try {
      const { commits } = await browse.log(ctx, ctx.global, { repo: r, ref: h.branch });
      all.push(...commits.map((c) => ({ ...c, repo: r.path, branch: h.branch })));
    } catch (err) {
      console.warn(`reading ${r.path}'s commits: ${err.message}`);
      left.add(r.path);
    }
  }
  const seen = new Set();
  const key = (c) => `${c.time} ${c.subject}`;
  const unique = all.sort((a, b) => b.time - a.time).filter((c) => !seen.has(key(c)) && seen.add(key(c)));
  const week = Array(7).fill(0), DAY = 86400000;
  for (const c of unique) {
    const d = Math.floor((ctx.now() - c.time) / DAY);
    if (d >= 0 && d < 7) week[6 - d]++;
  }
  return { commits: unique.slice(0, many), week, left: [...left] };
}
