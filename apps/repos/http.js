// Git over HTTP, as the node's local API passes it on (`/repos/<owner>/<repo>.git/...`), so stock git clones, fetches and
// pushes.
//
// Fetching is git's "dumb" protocol: the branches, the list of packs, and each pack and its index are files git
// downloads, so nothing is worked out per fetch. Pushing is the smart protocol's receive-pack, asking for whole packs
// (no-thin): each push's pack is checked, indexed and kept as a blob, and a branch moves only from the head it was at.
//
// Each push adds a pack, so the newest ones are merged while they're smaller than the one before them (sizes roughly
// double going back), until a pack passes FROZEN: a fetch downloads whole packs, so a merge rewrites at most that much
// of what someone behind already has.
import { check, idx, merge, objects } from "./git.js";

const FROZEN = 16 << 20;
const ZERO = "0".repeat(40);
const CAPS = "report-status delete-refs ofs-delta no-thin";
const enc = new TextEncoder();
const utf8 = new TextDecoder();

/// One pkt-line.
const pkt = (line) => (enc.encode(line).length + 4).toString(16).padStart(4, "0") + line;

const text = (body, status = 200, type = "text/plain") => ({ status, type, text: body });
const missing = (why = "not found") => text(why, 404);

/// Answers a request for `path` (`<owner>/<repo>.git/<file>`). `api` is what app.js knows about repos and who may push.
export async function serve(ctx, g, api, { method, path, query, body }) {
  const m = /^([^/]+)\/([^/]+?)(?:\.git)?\/(.+)$/.exec(path || "");
  if (!m || m[1].endsWith(".git")) return missing(`A repo's address is <your One App's address>/repos/<owner>/<repo>.git, its owner a person's or group's handle: repos_list has them`);
  const [, owner, name, file] = m;
  const service = new URLSearchParams(query || "").get("service");
  const found = await api.find(ctx, g, owner, name);
  if (method === "POST" && file === "git-receive-pack") return receive(ctx, g, api, owner, name, found, body);
  if (method !== "GET") return missing();
  if (file === "info/refs" && service === "git-receive-pack") return advertise(g, found.r);
  const r = found.r;
  if (!r) return missing(found.why || `There's no repo ${owner}/${name}`);
  if (r.deleted) return missing(`${r.path} was deleted: its owners restore it from Recently Deleted`);
  const branches = await g.query("select branch, head from branches where repo = ? order by branch", [r.id]);
  if (file === "info/refs") return text(branches.map((b) => `${b.head}\trefs/heads/${b.branch}\n`).join(""));
  if (file === "HEAD") return text(`ref: refs/heads/${api.defaultBranch(branches.map((b) => b.branch)) || "main"}\n`);
  if (file === "objects/info/packs") return text((await indexed(ctx, g, r.id)).map((p) => `P pack-${p.name}.pack\n`).join("") + "\n");
  const want = /^objects\/pack\/pack-([0-9a-f]{40})\.(pack|idx)$/.exec(file);
  if (want) {
    const p = (await indexed(ctx, g, r.id)).find((p) => p.name === want[1]);
    if (!p) return missing();
    return { status: 200, type: "application/octet-stream", blob: want[2] === "pack" ? p.pack : p.idx };
  }
  // Loose objects and alternates: there are none, every object is in a pack.
  return missing();
}

/// The packs of the repo with id `repo`, oldest first, each with its name and index; a pack kept before indexes were is
/// indexed now.
async function indexed(ctx, g, repo) {
  const rows = await g.query(
    "select p.hash as pack, i.name, i.idx, i.size from packs p left join indexes i on i.pack = p.hash where p.repo = ? order by p.time, p.hash",
    [repo],
  );
  for (const row of rows.filter((r) => !r.idx)) {
    const bytes = await ctx.blob.get(row.pack);
    const { name, entries } = await check(bytes);
    Object.assign(row, { name, idx: await ctx.blob.put(await idx(entries, name)), size: bytes.length });
    g.exec("insert or ignore into indexes values (?, ?, ?, ?)", [row.pack, row.name, row.idx, row.size], [row.idx]);
  }
  return rows;
}

/// What git asks before it pushes: the branches, and what this end can do.
async function advertise(g, r) {
  const branches = r && !r.deleted ? await g.query("select branch, head from branches where repo = ? order by branch", [r.id]) : [];
  const refs = branches.length ? branches.map((b, i) => pkt(`${b.head} refs/heads/${b.branch}${i ? "" : "\0" + CAPS}\n`)) : [pkt(`${ZERO} capabilities^{}\0${CAPS}\n`)];
  return text(pkt("# service=git-receive-pack\n") + "0000" + refs.join("") + "0000", 200, "application/x-git-receive-pack-advertisement");
}

/// A push: which branches move from what to what, then the pack that holds what's new. Each branch is told ok or why
/// not; a pack that can't be read refuses them all.
async function receive(ctx, g, api, owner, name, found, hash) {
  const bytes = hash ? await ctx.blob.get(hash) : new Uint8Array();
  const commands = [];
  let at = 0;
  while (at + 4 <= bytes.length) {
    const len = parseInt(utf8.decode(bytes.subarray(at, at + 4)), 16);
    if (!len) { at += 4; break; }
    const [line] = utf8.decode(bytes.subarray(at + 4, at + len)).split("\0");
    const [old, head, ref] = line.trim().split(" ");
    commands.push({ old, head, ref });
    at += len;
  }
  // git checks it may push by sending nothing first.
  if (!commands.length) return text("", 200, "application/x-git-receive-pack-result");
  const report = (unpack, lines) => text(pkt(`unpack ${unpack}\n`) + lines.map(pkt).join("") + "0000", 200, "application/x-git-receive-pack-result");
  // Refused, each branch with why; the pack too, only if it's the pack that's wrong.
  const refuse = (why, unpacked = "ok") => report(unpacked, commands.map((c) => `ng ${c.ref} ${why.replace(/\n/g, " ")}\n`));

  let r = found.r;
  if (found.why) return refuse(found.why);
  if (r?.deleted) return refuse(`${r.path} was deleted: its owners restore it from Recently Deleted`);
  if (r && !(await g.may("push", api.object(r)))) return refuse(`only ${api.ownerName(ctx, r.owner)} and its collaborators push to ${r.path}: fork it (repos fork ${r.path}), push to your fork, and tell them`);
  if (!r) {
    const made = api.newRepo(ctx, g, owner, name);
    if (made.why) return refuse(made.why);
    r = made.r;
  }

  const kept = (await g.query("select hash from packs where repo = ? order by time, hash", [r.id])).map((p) => p.hash);
  let added = null;
  if (at < bytes.length) {
    const pack = bytes.subarray(at);
    let checked;
    try {
      checked = await check(pack);
    } catch (err) {
      return refuse(err.message, err.message.replace(/\n/g, " "));
    }
    if (checked.entries.length) added = { bytes: pack, ...checked };
  }
  const have = await objects(ctx, kept);
  const fresh = new Set(added?.entries.map((e) => e.sha));
  const branches = new Map((await g.query("select branch, head from branches where repo = ?", [r.id])).map((b) => [b.branch, b.head]));
  const lines = [];
  for (const { old, head, ref } of commands) {
    const branch = ref.startsWith("refs/heads/") ? ref.slice(11) : null;
    const why = !branch || !api.BRANCH.test(branch) ? "only branches can be pushed"
      : (branches.get(branch) ?? ZERO) !== old ? `${branch} has moved: fetch, then push again`
      : head !== ZERO && !fresh.has(head) && !have.has(head) ? `the push doesn't hold ${head.slice(0, 12)}`
      : null;
    if (why) { lines.push(`ng ${ref} ${why}\n`); continue; }
    api.moveBranch(ctx, g, r.id, branch, old === ZERO ? null : old, head === ZERO ? null : head);
    branches.set(branch, head);
    lines.push(`ok ${ref}\n`);
  }
  if (added && lines.some((l) => l.startsWith("ok "))) await keep(ctx, g, r.id, added);
  return report("ok", lines);
}

/// Keeps a pushed pack for the repo with id `repo`, then merges its newest packs while each is smaller than twice the ones after it.
async function keep(ctx, g, repo, pack) {
  const rows = await indexed(ctx, g, repo);
  let newest = [{ bytes: pack.bytes, name: pack.name, entries: pack.entries, size: pack.bytes.length }];
  let sum = pack.bytes.length;
  for (let i = rows.length - 1; i >= 0; i--) {
    const r = rows[i];
    if (r.size >= FROZEN || r.size > 2 * sum || sum + r.size > FROZEN) break;
    newest.unshift({ ...r, size: r.size });
    sum += r.size;
  }
  const old = newest.slice(0, -1);
  let out = newest[0];
  if (old.length) {
    const parts = await Promise.all(old.map(async (r) => {
      const bytes = await ctx.blob.get(r.pack);
      return { bytes, entries: (await check(bytes)).entries };
    }));
    out = await merge([...parts, { bytes: pack.bytes, entries: pack.entries }]);
  }
  const blob = await ctx.blob.put(out.bytes);
  const index = await ctx.blob.put(await idx(out.entries, out.name));
  g.exec("insert or ignore into packs values (?, ?, ?)", [repo, blob, ctx.now()], [blob]);
  g.exec("insert or ignore into indexes values (?, ?, ?, ?)", [blob, out.name, index, out.bytes.length], [index]);
  for (const r of old) {
    g.exec("delete from packs where repo = ? and hash = ?", [repo, r.pack]);
    await forgetUnshared(g, repo, r.pack);
  }
}

/// Lets go of a pack `repo` no longer has, and its index, unless another repo (a fork) still does.
export async function forgetUnshared(g, repo, pack) {
  const shared = await g.query("select 1 from packs where hash = ? and repo != ?", [pack, repo]);
  if (shared.length) return;
  const index = (await g.query("select idx from indexes where pack = ?", [pack]))[0];
  g.exec("delete from indexes where pack = ?", [pack]);
  g.forget(pack);
  if (index) g.forget(index.idx);
}
