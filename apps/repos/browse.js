// Reading a repo's code for its pages: folders, files, history and diffs, from the packs (git.js).
//
// jsdiff for line diffs: hunks with context, and it gives up on huge rewrites instead of running for minutes.
import { structuredPatch } from "diff";
import { objects } from "./git.js";

const PAGE = 25;
const PREVIEW = 1 << 20; // bigger files aren't shown
const BIG_DIFF = 4000; // changed lines in a file before its diff waits to be asked for
const DIFF_BUDGET = 20000; // changed lines in a commit before later files wait
const README = /^readme(\.(md|mdx|markdown|txt|rst))?$/i;
const SHA = /^[0-9a-f]{40}$/;
const natural = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
const utf8 = new TextDecoder();

export async function open(ctx, g, r, ref) {
  const branches = await g.query("select branch, head, name, time from branches where repo = ? order by branch", [r.id]);
  const o = await objects(ctx, await g.query("select p.hash, i.idx from packs p join indexes i on i.pack = p.hash where p.repo = ? order by p.time, p.hash", [r.id]));
  if (!branches.length) return { r, o, branches, ref: null, sha: null };
  ref ||= branches.find((b) => b.branch === "main")?.branch || branches[0].branch;
  const b = branches.find((x) => x.branch === ref);
  const sha = b ? b.head : SHA.test(ref) && o.has(ref) ? ref : null;
  if (!sha) throw new Error(`There's no branch or commit ${ref} in ${r.path}`);
  return { r, o, branches, ref, sha, branch: b ? ref : null };
}

export const summary = (c) => {
  const [subject, ...rest] = c.message.replace(/\s+$/, "").split("\n");
  return {
    sha: c.sha, subject: subject || "(no message)", body: rest.join("\n").replace(/^\n+/, ""),
    author: c.author.name, email: c.author.email, time: c.author.time, committed: c.committer.time, parents: c.parents,
  };
};

function at(o, commit, path) {
  let entry = { type: "tree", sha: o.commit(commit).tree, mode: "040000" };
  for (const part of path.split("/").filter(Boolean)) {
    if (entry.type !== "tree") return null;
    entry = o.tree(entry.sha).find((e) => e.name === part);
    if (!entry) return null;
  }
  return entry;
}

const sorted = (entries) =>
  [...entries].sort((a, b) => (a.type === "tree") !== (b.type === "tree") ? (a.type === "tree" ? -1 : 1) : natural.compare(a.name, b.name));

const binary = (bytes) => bytes.subarray(0, 8000).includes(0);

export async function tree(ctx, g, { repo, ref, path = "" }) {
  const { r, o, branches, ref: name, sha, branch } = await open(ctx, g, repo, ref);
  const info = { repo: r.path, branches: branches.map((b) => b.branch), ref: name, branch, path };
  if (!sha) return { ...info, empty: true, entries: [] };
  const here = at(o, sha, path);
  if (!here) throw new Error(`There's no ${path} in ${repo.path} at ${branch || sha.slice(0, 7)}`);
  if (here.type !== "tree") return { ...info, commit: summary(o.commit(sha)), file: true };
  const entries = sorted(o.tree(here.sha)).map((e) => ({ name: e.name, type: e.type, mode: e.mode, sha: e.sha }));
  const rm = entries.filter((e) => e.type === "blob" && README.test(e.name)).sort((a, b) => /\.md$/i.test(b.name) - /\.md$/i.test(a.name))[0];
  let readme = null;
  if (rm) {
    const bytes = o.blob(rm.sha);
    if (bytes.length < PREVIEW && !binary(bytes)) readme = { name: rm.name, text: utf8.decode(bytes) };
  }
  return { ...info, sha, commit: summary(o.commit(sha)), latest: summary(o.commit(last(o, sha, path) || sha)), entries, readme };
}

export async function file(ctx, g, { repo, ref, path }) {
  const { o, ref: name, sha, branch, branches } = await open(ctx, g, repo, ref);
  if (!sha) throw new Error(`${repo.path} has no code yet`);
  const e = at(o, sha, path || "");
  if (!e || e.type === "tree") throw new Error(`There's no file ${path} in ${repo.path}`);
  const info = { repo: repo.path, ref: name, branch, sha, path, branches: branches.map((b) => b.branch), mode: e.mode, latest: summary(o.commit(last(o, sha, path) || sha)) };
  if (e.type === "commit") return { ...info, submodule: e.sha, size: 0 };
  const bytes = o.blob(e.sha);
  if (binary(bytes)) return { ...info, size: bytes.length, binary: true };
  if (bytes.length > PREVIEW) return { ...info, size: bytes.length, too_big: true };
  return { ...info, size: bytes.length, text: utf8.decode(bytes) };
}

function* walk(o, heads, keep = () => true) {
  const seen = new Set(heads);
  const queue = heads.map((s) => o.commit(s));
  while (queue.length) {
    let k = 0;
    for (let j = 1; j < queue.length; j++) if (queue[j].committer.time > queue[k].committer.time) k = j;
    const [c] = queue.splice(k, 1);
    if (keep(c)) yield c;
    for (const p of c.parents) {
      if (seen.has(p) || !o.has(p)) continue;
      seen.add(p);
      queue.push(o.commit(p));
    }
  }
}

function touches(o, c, path) {
  if (!path) return true;
  const mine = at(o, c.sha, path)?.sha;
  const theirs = c.parents[0] && o.has(c.parents[0]) ? at(o, c.parents[0], path)?.sha : undefined;
  return mine !== theirs;
}

function last(o, sha, path) {
  if (!path) return sha;
  let n = 0;
  for (const c of walk(o, [sha])) {
    if (touches(o, c, path)) return c.sha;
    if (++n > 500) return null;
  }
  return null;
}

export async function log(ctx, g, { repo, ref, path = "", page = 0 }) {
  const { o, ref: name, branch, sha } = await open(ctx, g, repo, ref);
  if (!sha) return { repo: repo.path, ref: name, branch, path, page, commits: [], more: false };
  const commits = [];
  let scanned = 0;
  for (const c of walk(o, [sha], (c) => (scanned++, touches(o, c, path)))) {
    commits.push(c);
    if (commits.length > (page + 1) * PAGE || scanned > 5000) break;
  }
  return { repo: repo.path, ref: name, branch, sha, path, page, commits: commits.slice(page * PAGE, (page + 1) * PAGE).map(summary), more: commits.length > (page + 1) * PAGE };
}

export async function latest(ctx, g, { repo, ref, path = "" }) {
  const { o, sha } = await open(ctx, g, repo, ref);
  if (!sha) return {};
  const here = at(o, sha, path);
  if (!here || here.type !== "tree") return {};
  const out = {};
  const wanted = new Set(o.tree(here.sha).map((e) => e.name));
  let n = 0;
  for (const c of walk(o, [sha])) {
    const mine = at(o, c.sha, path), parent = c.parents[0] && o.has(c.parents[0]) ? at(o, c.parents[0], path) : null;
    if (mine?.type === "tree" && mine.sha !== parent?.sha) {
      const before = new Map(parent?.type === "tree" ? o.tree(parent.sha).map((e) => [e.name, e.sha]) : []);
      for (const e of o.tree(mine.sha)) {
        if (!wanted.has(e.name) || before.get(e.name) === e.sha) continue;
        const s = summary(c);
        out[e.name] = { sha: s.sha, subject: s.subject, time: s.time, author: s.author };
        wanted.delete(e.name);
      }
    }
    // A folder of thousands of entries is read afresh from each commit, so it's given fewer commits to look back through.
    if (!wanted.size || (n += 1 + (mine?.type === "tree" ? o.tree(mine.sha).length : 0) / 1000) > 300) break;
  }
  return out;
}

function files(o, sha, prefix = "", out = new Map()) {
  if (!sha) return out;
  for (const e of o.tree(sha)) {
    const p = prefix + e.name;
    if (e.type === "tree") files(o, e.sha, p + "/", out);
    else out.set(p, { sha: e.sha, mode: e.mode, type: e.type });
  }
  return out;
}

function changes(o, c) {
  const before = files(o, c.parents[0] && o.has(c.parents[0]) ? o.commit(c.parents[0]).tree : null);
  const after = files(o, c.tree);
  const rows = [];
  for (const [path, a] of after) {
    const b = before.get(path);
    if (!b) rows.push({ path, status: "added", new: a });
    else if (b.sha !== a.sha || b.mode !== a.mode) rows.push({ path, status: "modified", old: b, new: a });
  }
  for (const [path, b] of before) if (!after.has(path)) rows.push({ path, status: "deleted", old: b });
  for (const del of rows.filter((r) => r.status === "deleted")) {
    const add = rows.find((r) => r.status === "added" && r.new.sha === del.old.sha);
    if (!add) continue;
    Object.assign(add, { status: "renamed", old_path: del.path, old: del.old });
    rows.splice(rows.indexOf(del), 1);
  }
  return rows.sort((x, y) => natural.compare(x.path, y.path));
}

const text = (o, side) => {
  if (!side || side.type === "commit") return { text: side ? `Subproject commit ${side.sha}\n` : "" };
  const bytes = o.blob(side.sha);
  if (binary(bytes)) return { binary: true };
  if (bytes.length > 4 * PREVIEW) return { too_big: true };
  return { text: utf8.decode(bytes) };
};

function diffFile(o, row, full) {
  const a = text(o, row.old), b = text(o, row.new);
  if (a.binary || b.binary) return { binary: true, additions: 0, deletions: 0 };
  if (a.too_big || b.too_big) return { too_big: true, additions: 0, deletions: 0 };
  if (a.text === b.text) return { additions: 0, deletions: 0, hunks: [] };
  let p = structuredPatch("", "", a.text, b.text, "", "", { context: 3, maxEditLength: full ? 20000 : 4000 });
  if (!p) {
    const lines = (t) => (t ? t.replace(/\n$/, "").split("\n") : []);
    const old = lines(a.text), neu = lines(b.text);
    p = { hunks: [{ oldStart: 1, newStart: 1, lines: [...old.map((l) => "-" + l), ...neu.map((l) => "+" + l)] }] };
  }
  let additions = 0, deletions = 0;
  const hunks = p.hunks.map((h) => ({ a: h.oldStart, b: h.newStart, lines: h.lines.filter((l) => l[0] !== "\\") }));
  for (const h of hunks) for (const l of h.lines) l[0] === "+" ? additions++ : l[0] === "-" && deletions++;
  return { additions, deletions, hunks };
}

const row = (r) => ({ path: r.path, old_path: r.old_path || null, status: r.status, old_mode: r.old?.mode || null, new_mode: r.new?.mode || null });

export async function commit(ctx, g, { repo, sha }) {
  const { o } = await open(ctx, g, repo, sha);
  const c = o.commit(sha);
  let budget = DIFF_BUDGET;
  const out = changes(o, c).map((r) => {
    const d = diffFile(o, r, false);
    const lines = d.additions + d.deletions;
    if (d.hunks && (lines > BIG_DIFF || lines > budget)) {
      delete d.hunks;
      d.large = lines > BIG_DIFF;
      d.later = true;
    } else budget -= lines;
    return { ...row(r), ...d };
  });
  return { repo: repo.path, ...summary(c), files: out };
}

export async function diff(ctx, g, { repo, sha, path }) {
  const { o } = await open(ctx, g, repo, sha);
  const r = changes(o, o.commit(sha)).find((x) => x.path === path);
  if (!r) throw new Error(`There's no change to ${path} in ${sha.slice(0, 7)}`);
  return { ...row(r), ...diffFile(o, r, true) };
}

export async function branches(ctx, g, { repo }) {
  const { o, branches } = await open(ctx, g, repo);
  return branches.map((b) => {
    const c = o.has(b.head) ? summary(o.commit(b.head)) : null;
    return { branch: b.branch, head: b.head, pushed_by: b.name, time: b.time, subject: c?.subject ?? null, author: c?.author ?? null, committed: c?.time ?? null };
  });
}
