// GitHub's repos in Repos, connected rather than copied: every repo a person's GitHub can see is
// github.com/<owner>/<repo>, and each tool on one asks GitHub's API at once, in the shapes Repos' own give.
//
// The network's GitHub App, which an admin makes once from Repos' Settings with GitHub's App Manifest flow, is a
// "GitHub App" item in Everyone's Vault: its id, client id, client secret and private key, the secrets sent only to
// GitHub. Each person signs in to their own GitHub accounts with it (user tokens, which it refreshes), each a "GitHub
// account" item in their own Vault. A group's admins connect an organization by installing the App on it: its repos
// show for the group's members, read and written as the App's installation (whose short-lived tokens it asks for with
// a JWT the Vault signs with the App's key), unless their own sign-in reaches the repo, which is used instead.
// Nothing GitHub gives is kept but those sign-ins and which organizations are connected for which group.

export const PREFIX = "github.com/";
const PAGE = 25;
const PREVIEW = 1 << 20;
const natural = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

export const VAULT = {
  app: {
    title: "GitHub App", type: "GitHub App", domain: "github.com", hosts: ["github.com", "api.github.com"],
    why: "Repos makes it: Repos › Settings › Create GitHub App. It's set once, for everyone.",
    shared: true,
    fields: { app_id: { label: "App ID" }, slug: { label: "Slug" }, client_id: { label: "Client ID" }, client_secret: { label: "Client secret", secret: true }, private_key: { label: "Private key", secret: true } },
  },
  account: {
    title: "GitHub account", type: "GitHub account", domain: "github.com", hosts: ["github.com", "api.github.com"], accounts: true,
    fields: { login: { label: "Username" }, access_token: { label: "Access token", secret: true }, refresh_token: { label: "Refresh token", secret: true } },
  },
};

export const SCHEMA = `create table github_orgs (installation integer primary key, org text not null, owner text not null, user text not null, time integer not null);
  create table github_pending (what text primary key, value text not null, time integer not null);`;

export const isGithub = (repo) => String(repo || "").toLowerCase().startsWith(PREFIX);

function fullOf(repo) {
  const m = /^github\.com\/([A-Za-z0-9-]+)\/([A-Za-z0-9._-]+?)(\.git)?$/i.exec(String(repo || "").trim());
  if (!m) throw new Error(`give a GitHub repo as github.com/<owner>/<name>, like github.com/octocat/hello-world: repos_list has them`);
  return `${m[1]}/${m[2]}`;
}

/// The network's GitHub App: an admin made it, in Everyone's Vault (or the caller's own, for one of theirs).
async function appItem(ctx) {
  const all = (await ctx.vault.items()).filter((i) => i.type === "GitHub App" && i.fields.client_id?.value);
  return all.find((i) => i.scope.startsWith("user:")) || all[0] || null;
}
const noApp = (ctx) => new Error(ctx.me.role === "admin"
  ? "GitHub isn't connected yet: Repos › Settings › Create GitHub App makes the network's GitHub App, once, for everyone (repos_create_github_app)"
  : "GitHub isn't connected yet: an admin makes the network's GitHub App once, in Repos › Settings; ask them");
/// GitHub's site and API for an App: github.com's unless it was made on another (GitHub Enterprise, or a stand-in).
const webOf = (app) => app?.fields.web?.value || "https://github.com";
const apiOf = (app) => app?.fields.api?.value || (webOf(app) === "https://github.com" ? "https://api.github.com" : `${webOf(app)}/api/v3`);
const graphqlOf = (app) => (webOf(app) === "https://github.com" ? "https://api.github.com/graphql" : `${webOf(app)}/api/graphql`);

function redirect(ctx) {
  if (!ctx.address) throw new Error("This device is out of date: update it, then try again");
  return `${ctx.address}/back`;
}

async function accountItems(ctx) {
  if (!ctx.user) return [];
  return (await ctx.vault.items()).filter((i) => i.type === "GitHub account" && i.fields.login?.value && i.fields.access_token).sort((a, b) => a.time - b.time);
}

// By account item id and installation id.
const userTokens = new Map(), installTokens = new Map();

async function userAuth(ctx, app, item) {
  const held = userTokens.get(item.id);
  if (held && (!held.expires || held.expires > Date.now() + 60_000)) return `Bearer ${held.value}`;
  const expires = Number(item.fields.expires?.value || 0);
  if (!held && (!expires || expires > Date.now() + 60_000)) return `Bearer {{${item.id}:access_token}}`;
  if (!item.fields.refresh_token) throw new Error(`GitHub signed ${item.fields.login.value} out: Repos › Settings signs in again (repos_sign_in)`);
  // GitHub takes each refresh token once: calls at the same time share one refresh.
  if (!refreshing.has(item.id)) refreshing.set(item.id, refreshed(ctx, app, item).finally(() => refreshing.delete(item.id)));
  return refreshing.get(item.id);
}
const refreshing = new Map();

async function refreshed(ctx, app, item) {
  const r = await fetch(`${webOf(app)}/login/oauth/access_token`, {
    method: "POST",
    headers: { accept: "application/json", "content-type": "application/x-www-form-urlencoded" },
    body: `grant_type=refresh_token&client_id=${encodeURIComponent(app.fields.client_id.value)}&client_secret={{${app.id}:client_secret}}&refresh_token={{${item.id}:refresh_token}}`,
  });
  const t = await r.json().catch(() => ({}));
  if (!r.ok || !t.access_token) {
    // Another call (here, or on another device) may have refreshed it first: its new token reaches the Vault once
    // that call is done, so it waits a few seconds for it.
    for (let tries = 0; tries < 12; tries++) {
      const now = (await accountItems(ctx)).find((i) => i.id === item.id);
      if (!now) break;
      const expires = Number(now.fields.expires?.value || 0);
      if (now.fields.expires?.value !== item.fields.expires?.value && (!expires || expires > Date.now() + 60_000)) return `Bearer {{${item.id}:access_token}}`;
      await new Promise((ok) => setTimeout(ok, 500));
    }
    throw new Error(`GitHub signed ${item.fields.login.value} out (${t.error_description || t.error || r.status}): Repos › Settings signs in again (repos_sign_in)`);
  }
  keepTokens(ctx, item.id, t);
  return `Bearer ${t.access_token}`;
}

function keepTokens(ctx, id, t) {
  const expires = t.expires_in ? Date.now() + t.expires_in * 1000 : 0;
  userTokens.set(id, { value: t.access_token, expires });
  ctx.vault.update(id, { fields: { access_token: { value: t.access_token, secret: true }, ...(t.refresh_token ? { refresh_token: { value: t.refresh_token, secret: true } } : {}), expires: { value: String(expires) } } });
}

const b64url = (s) => btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");

/// A token GitHub gives the App for an hour, asked for with a JWT the Vault signs with the App's private key.
async function installAuth(app, installation) {
  const held = installTokens.get(installation);
  if (held && held.expires > Date.now() + 60_000) return `Bearer ${held.value}`;
  const now = Math.floor(Date.now() / 1000);
  const claims = b64url(JSON.stringify({ iat: now - 60, exp: now + 540, iss: app.fields.app_id.value }));
  const r = await fetch(`${apiOf(app)}/app/installations/${installation}/access_tokens`, {
    method: "POST",
    headers: { accept: "application/vnd.github+json", authorization: `Bearer {{rs256 ${app.id}:private_key ${claims}}}`, "user-agent": "Repos" },
  });
  const t = await r.json().catch(() => ({}));
  if (!r.ok || !t.token) throw new Error(`GitHub didn't let the App act for that organization (${t.message || r.status}): its admins connect it again in Repos › Settings`);
  installTokens.set(installation, { value: t.token, expires: Date.parse(t.expires_at) || Date.now() + 3_000_000 });
  return `Bearer ${t.token}`;
}

/// In the order tried: the caller's own accounts, then the organizations connected for groups they're in.
async function credentials(ctx) {
  const app = await appItem(ctx);
  if (!app) return { app: null, all: [] };
  const all = (await accountItems(ctx)).map((item) => ({ kind: "user", login: item.fields.login.value, item, auth: () => userAuth(ctx, app, item) }));
  for (const o of await myOrgs(ctx)) all.push({ kind: "installation", org: o.org, installation: o.installation, auth: () => installAuth(app, o.installation) });
  return { app, all };
}

async function myOrgs(ctx) {
  if (!ctx.global) return [];
  const rows = await ctx.global.query("select installation, org, owner from github_orgs order by org");
  return rows.filter((o) => inGroup(ctx, o.owner));
}
const groupOf = (ctx, id) => (ctx.handles || []).find((h) => h.id === id) || null;
const inGroup = (ctx, id) => !!groupOf(ctx, id)?.members?.[ctx.me.user];
const adminOf = (ctx, id) => groupOf(ctx, id)?.members?.[ctx.me.user] === "admin";

const reaches = new Map();

async function credFor(ctx, full) {
  const { app, all } = await credentials(ctx);
  if (!app) throw noApp(ctx);
  if (!all.length) throw new Error("Sign in to GitHub first: Repos › Settings › Connect GitHub (repos_sign_in)");
  const key = `${ctx.me.user} ${full.toLowerCase()}`;
  const kept = reaches.get(key);
  const owner = full.split("/")[0].toLowerCase();
  const order = all.filter((c) => c.kind === "user" || c.org.toLowerCase() === owner);
  if (kept && kept.time > Date.now() - 600_000) {
    const c = order.find((x) => (x.login || x.org) === kept.who);
    if (c) return { app, c };
  }
  let why = null;
  for (const c of order) {
    const r = await send(app, c, "GET", `/repos/${full}`).catch((e) => ((why = e), null));
    if (r && r.status === 200) {
      reaches.set(key, { who: c.login || c.org, time: Date.now() });
      repoMeta.set(`${c.login || c.org} ${full.toLowerCase()}`, { r: r.json, time: Date.now() });
      return { app, c };
    }
  }
  if (why) throw why;
  throw new Error(`GitHub has no ${PREFIX}${full} that ${all.filter((c) => c.kind === "user").map((c) => c.login).join(" or ") || "your groups' organizations"} can see`);
}

async function send(app, c, method, path, body, { accept = "application/vnd.github+json", ok = [] } = {}) {
  const url = path.startsWith("http") ? path : `${apiOf(app)}${path}`;
  const r = await fetch(url, {
    method,
    headers: { accept, authorization: await c.auth(), "x-github-api-version": "2022-11-28", "user-agent": "Repos", ...(body ? { "content-type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await r.text();
  let json = null;
  try { json = text ? JSON.parse(text) : null; } catch { json = text; }
  if (r.ok || ok.includes(r.status)) return { status: r.status, json, headers: r.headers };
  if ((r.status === 403 || r.status === 429) && (r.headers.get("x-ratelimit-remaining") === "0" || /rate limit/i.test(json?.message || ""))) {
    const reset = Number(r.headers.get("x-ratelimit-reset")) * 1000 || Date.now() + 60_000;
    throw new Error(`GitHub's limit on requests is used up until ${new Date(reset).toISOString().slice(11, 16)} UTC: try again then`);
  }
  if (r.status === 401) {
    if (c.item) userTokens.delete(c.item.id);
    if (c.installation) installTokens.delete(c.installation);
    throw new Error(c.kind === "user" ? `GitHub signed ${c.login} out: Repos › Settings signs in again (repos_sign_in)` : `GitHub no longer lets the App act for ${c.org}: its admins connect it again in Repos › Settings`);
  }
  if (r.status === 404) throw Object.assign(new Error(`GitHub has nothing at ${path.replace(/^\/repos\//, PREFIX).replace(/\?.*$/, "")} that ${c.login || c.org} can see`), { missing: true });
  const detail = json?.errors?.map((e) => e.message || `${e.field} ${e.code}`).filter(Boolean).join("; ");
  throw new Error(`GitHub: ${json?.message || r.statusText || r.status}${detail ? ` (${detail})` : ""}`);
}

const graphql = async (app, c, query, variables) => {
  const r = await send(app, c, "POST", graphqlOf(app), { query, variables });
  if (r.json?.errors?.length) throw new Error(`GitHub: ${r.json.errors[0].message}`);
  return r.json.data;
};

const repoMeta = new Map();
async function repoOf(app, c, full) {
  const key = `${c.login || c.org} ${full.toLowerCase()}`;
  const kept = repoMeta.get(key);
  if (kept && kept.time > Date.now() - 60_000) return kept.r;
  const r = (await send(app, c, "GET", `/repos/${full}`)).json;
  repoMeta.set(key, { r, time: Date.now() });
  return r;
}

async function open(ctx, repo) {
  const full = fullOf(repo);
  const { app, c } = await credFor(ctx, full);
  const r = await repoOf(app, c, full);
  return { app, c, full: r.full_name, r, name: `${PREFIX}${r.full_name}` };
}

const person = (u) => (u ? { handle: u.login, name: u.login } : null);
const about = (g) => ({
  owner: person(g.owner), yours: !!g.permissions?.admin, push: !!g.permissions?.push,
  from: g.parent ? `${PREFIX}${g.parent.full_name}` : null, open_issues: g.open_issues_count ?? 0, github: { url: g.html_url, private: !!g.private },
});
const enc = (path) => String(path || "").split("/").filter(Boolean).map(encodeURIComponent).join("/");

function summary(g) {
  const [subject, ...rest] = String(g.commit.message || "").replace(/\s+$/, "").split("\n");
  return {
    sha: g.sha, subject: subject || "(no message)", body: rest.join("\n").replace(/^\n+/, ""),
    author: g.commit.author?.name || g.author?.login || "", email: g.commit.author?.email || "", time: g.commit.author?.date, committed: g.commit.committer?.date,
    parents: (g.parents || []).map((p) => p.sha),
  };
}

async function at(o, ref) {
  ref ||= o.r.default_branch;
  const r = await send(o.app, o.c, "GET", `/repos/${o.full}/commits/${encodeURIComponent(ref)}`, null, { ok: [409, 422] });
  if (r.status === 409) return { ref: null, branch: null, sha: null, commit: null };
  if (r.status === 422) throw new Error(`There's no branch or commit ${ref} in ${o.name}`);
  const isBranch = !/^[0-9a-f]{7,40}$/i.test(ref) || ref === o.r.default_branch;
  return { ref, branch: isBranch ? ref : null, sha: r.json.sha, commit: r.json };
}

async function branchNames(o) {
  const r = await send(o.app, o.c, "GET", `/repos/${o.full}/branches?per_page=100`, null, { ok: [409] });
  return r.status === 409 ? [] : r.json.map((b) => b.name);
}

const bytesOf = (b64) => Uint8Array.from(atob(String(b64 || "").replace(/\s+/g, "")), (ch) => ch.charCodeAt(0));
const binary = (bytes) => bytes.subarray(0, 8000).includes(0);

function hunks(patch) {
  const out = [];
  for (const line of String(patch || "").split("\n")) {
    const h = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (h) out.push({ a: Number(h[1]), b: Number(h[2]), lines: [] });
    else if (out.length && line[0] !== "\\") out.at(-1).lines.push(line);
  }
  return out;
}
const STATUS = { removed: "deleted", copied: "added", changed: "modified" };
function changed(f) {
  const row = { path: f.filename, old_path: f.previous_filename || null, status: STATUS[f.status] || f.status, old_mode: null, new_mode: null, additions: f.additions, deletions: f.deletions };
  if (f.patch) return { ...row, hunks: hunks(f.patch) };
  return f.additions || f.deletions ? { ...row, too_big: true } : f.status === "renamed" ? { ...row, hunks: [] } : { ...row, binary: true };
}

const issueShape = (i) => ({
  number: i.number, title: i.title, state: i.state, state_reason: i.state_reason || null, author: person(i.user),
  labels: (i.labels || []).map((l) => ({ name: l.name, color: l.color, description: l.description || "" })),
  assignees: (i.assignees || []).map(person), comments: i.comments ?? 0,
  created: i.created_at, updated: i.updated_at, closed: i.closed_at, closed_by: person(i.closed_by),
});

/// What a person filed through an organization's installation says, so the issue shows who in the network filed it.
const signed = (ctx, o, text, verb) => (o.c.kind === "installation" ? `${text || ""}\n\n<sub>${verb} by ${ctx.me.user_name || ctx.me.name} through ${ctx.address ? new URL(ctx.address).host : "Repos"}</sub>`.replace(/^\n\n/, "") : text);
const quote = (s) => (/\s/.test(s) ? `"${s.replace(/"/g, "")}"` : s);
const ON_GITHUB = (o, what) => new Error(`${o.name} is on GitHub: ${what} there, at ${o.r.html_url}`);

export const run = {
  async get(ctx, { repo }) {
    const o = await open(ctx, repo);
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/branches?per_page=100`, null, { ok: [409] });
    const branches = r.status === 409 ? [] : r.json.map((b) => ({ branch: b.name, head: b.commit.sha, user: null, name: null, time: null }));
    return { repo: o.name, name: o.r.name, address: o.r.clone_url, created: o.r.created_at, ...about(o.r), branches, packs: [], forks: [], pushes: [] };
  },
  async tree(ctx, { repo, ref, path = "" }) {
    const o = await open(ctx, repo);
    const [h, branches] = await Promise.all([at(o, ref), branchNames(o)]);
    const info = { repo: o.name, name: o.r.name, branches, ref: h.ref, branch: h.branch, path, ...about(o.r), forks: [], clone: o.r.clone_url };
    if (!h.sha) return { ...info, empty: true, entries: [] };
    const c = await send(o.app, o.c, "GET", `/repos/${o.full}/contents/${enc(path)}?ref=${h.sha}`);
    if (!Array.isArray(c.json)) return { ...info, sha: h.sha, commit: summary(h.commit), file: true };
    const type = { dir: "tree", submodule: "commit" };
    const entries = c.json.map((e) => ({ name: e.name, type: type[e.type] || "blob", mode: null, sha: e.sha }))
      .sort((a, b) => ((a.type === "tree") !== (b.type === "tree") ? (a.type === "tree" ? -1 : 1) : natural.compare(a.name, b.name)));
    const [readme, last] = await Promise.all([
      send(o.app, o.c, "GET", `/repos/${o.full}/readme/${enc(path)}?ref=${h.sha}`, null, { ok: [404] }).then((r) => (r.status === 200 && r.json.size < PREVIEW ? { name: r.json.name, text: new TextDecoder().decode(bytesOf(r.json.content)) } : null)),
      path ? send(o.app, o.c, "GET", `/repos/${o.full}/commits?sha=${h.sha}&path=${encodeURIComponent(path)}&per_page=1`).then((r) => r.json[0] || h.commit) : h.commit,
    ]);
    return { ...info, sha: h.sha, commit: summary(h.commit), latest: summary(last), entries, readme };
  },
  async file(ctx, { repo, ref, path }) {
    const o = await open(ctx, repo);
    const [h, branches] = await Promise.all([at(o, ref), branchNames(o)]);
    if (!h.sha) throw new Error(`${o.name} has no code yet`);
    const [c, last] = await Promise.all([
      send(o.app, o.c, "GET", `/repos/${o.full}/contents/${enc(path)}?ref=${h.sha}`),
      send(o.app, o.c, "GET", `/repos/${o.full}/commits?sha=${h.sha}&path=${encodeURIComponent(path || "")}&per_page=1`).then((r) => r.json[0] || h.commit),
    ]);
    if (Array.isArray(c.json)) throw new Error(`There's no file ${path} in ${o.name}`);
    const info = { repo: o.name, ref: h.ref, branch: h.branch, sha: h.sha, path, branches, mode: null, latest: summary(last) };
    if (c.json.type === "submodule") return { ...info, submodule: c.json.sha, size: 0 };
    if (c.json.size > PREVIEW || c.json.encoding === "none") return { ...info, size: c.json.size, too_big: true };
    const bytes = bytesOf(c.json.content);
    if (binary(bytes)) return { ...info, size: bytes.length, binary: true };
    return { ...info, size: bytes.length, text: new TextDecoder().decode(bytes) };
  },
  async log(ctx, { repo, ref, path = "", page = 0 }) {
    const o = await open(ctx, repo);
    const h = await at(o, ref);
    if (!h.sha) return { repo: o.name, ref: h.ref, branch: h.branch, path, page, commits: [], more: false };
    const p = Math.max(0, Math.floor(Number(page) || 0));
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/commits?sha=${h.sha}${path ? `&path=${encodeURIComponent(path)}` : ""}&per_page=${PAGE}&page=${p + 1}`);
    return { repo: o.name, ref: h.ref, branch: h.branch, sha: h.sha, path, page: p, commits: r.json.map(summary), more: /rel="next"/.test(r.headers.get("link") || "") };
  },
  async commit(ctx, { repo, sha }) {
    const o = await open(ctx, repo);
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/commits/${encodeURIComponent(sha)}`);
    let budget = 20000;
    const files = (r.json.files || []).map(changed).map((f) => {
      const lines = f.additions + f.deletions;
      if (f.hunks && (lines > 4000 || lines > budget)) return { ...(({ hunks, ...rest }) => rest)(f), large: lines > 4000, later: true };
      if (f.hunks) budget -= lines;
      return f;
    });
    return { repo: o.name, ...summary(r.json), files };
  },
  async diff(ctx, { repo, sha, path }) {
    const o = await open(ctx, repo);
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/commits/${encodeURIComponent(sha)}`);
    const f = (r.json.files || []).find((x) => x.filename === path);
    if (!f) throw new Error(`There's no change to ${path} in ${String(sha).slice(0, 7)}`);
    return changed(f);
  },
  /// The last commit to each entry, as GitHub's own page shows it: one GraphQL request, its history of each path.
  async latest(ctx, { repo, ref, path = "" }) {
    const o = await open(ctx, repo);
    const h = await at(o, ref);
    if (!h.sha) return {};
    const c = await send(o.app, o.c, "GET", `/repos/${o.full}/contents/${enc(path)}?ref=${h.sha}`);
    if (!Array.isArray(c.json) || !c.json.length) return {};
    const names = c.json.slice(0, 100).map((e) => e.name);
    const fields = names.map((n, i) => `e${i}: history(first: 1, path: ${JSON.stringify((path ? path + "/" : "") + n)}) { nodes { oid messageHeadline authoredDate author { name } } }`).join("\n");
    const data = await graphql(o.app, o.c, `query($owner: String!, $name: String!, $sha: GitObjectID!) { repository(owner: $owner, name: $name) { object(oid: $sha) { ... on Commit { ${fields} } } } }`,
      { owner: o.r.owner.login, name: o.r.name, sha: h.sha }).catch(() => null);
    const obj = data?.repository?.object || {};
    const out = {};
    names.forEach((n, i) => {
      const x = obj[`e${i}`]?.nodes?.[0];
      if (x) out[n] = { sha: x.oid, subject: x.messageHeadline, time: x.authoredDate, author: x.author?.name || "" };
    });
    return out;
  },
  async branches(ctx, { repo }) {
    const o = await open(ctx, repo);
    const data = await graphql(o.app, o.c, `query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { refs(refPrefix: "refs/heads/", first: 100) { nodes { name target { ... on Commit { oid messageHeadline authoredDate committedDate author { name } } } } } } }`,
      { owner: o.r.owner.login, name: o.r.name }).catch(() => null);
    if (data) return data.repository.refs.nodes.map((b) => ({ branch: b.name, head: b.target.oid, pushed_by: null, time: b.target.committedDate, subject: b.target.messageHeadline, author: b.target.author?.name || null, committed: b.target.committedDate }));
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/branches?per_page=100`, null, { ok: [409] });
    return r.status === 409 ? [] : r.json.map((b) => ({ branch: b.name, head: b.commit.sha, pushed_by: null, time: null, subject: null, author: null, committed: null }));
  },

  // Issues, as GitHub's own: its search for the list and counts (which leaves out pull requests), its API for the rest.
  async issues(ctx, { repo, state = "open", label, assignee, author, q, page = 0 }) {
    const o = await open(ctx, repo);
    if (!["open", "closed", "all"].includes(state)) throw new Error("state is open, closed or all");
    const terms = [`repo:${o.full}`, "is:issue"];
    if (label) terms.push(`label:${quote(label)}`);
    if (assignee === "none") terms.push("no:assignee");
    else if (assignee) terms.push(`assignee:${assignee.replace(/^@/, "")}`);
    if (author) terms.push(`author:${author.replace(/^@/, "")}`);
    const words = String(q || "").trim();
    if (/^#?\d+$/.test(words)) terms.push(`${words.replace("#", "")} in:number`);
    else if (words) terms.push(words);
    const p = Math.max(0, Math.floor(Number(page) || 0));
    const search = (extra, per, pg = 1) => send(o.app, o.c, "GET", `/search/issues?q=${encodeURIComponent([...terms, ...extra].join(" "))}&sort=created&order=desc&per_page=${per}&page=${pg}`).then((r) => r.json);
    const [open_, closed_] = await Promise.all([search(["is:open"], state === "open" ? PAGE : 1, state === "open" ? p + 1 : 1), search(["is:closed"], state === "closed" ? PAGE : 1, state === "closed" ? p + 1 : 1)]);
    const shown = state === "all" ? await search([], PAGE, p + 1) : state === "open" ? open_ : closed_;
    const total = state === "all" ? shown.total_count : state === "open" ? open_.total_count : closed_.total_count;
    return {
      repo: o.name, open_issues: open_.total_count, closed_issues: closed_.total_count, issues: shown.items.map(issueShape),
      more: (p + 1) * PAGE < total, push: !!o.r.permissions?.push, yours: !!o.r.permissions?.admin,
    };
  },
  async issue(ctx, { repo, number }) {
    const o = await open(ctx, repo);
    const n = Number(String(number).replace("#", ""));
    const [i, comments] = await Promise.all([
      send(o.app, o.c, "GET", `/repos/${o.full}/issues/${n}`).then((r) => r.json),
      send(o.app, o.c, "GET", `/repos/${o.full}/issues/${n}/comments?per_page=100`).then((r) => r.json),
    ]);
    if (i.pull_request) throw new Error(`#${n} of ${o.name} is a pull request: see it on GitHub, at ${i.html_url}`);
    const me = o.c.login?.toLowerCase(), push = !!o.r.permissions?.push, author = !!me && i.user?.login?.toLowerCase() === me;
    return {
      repo: o.name, ...issueShape(i), body: i.body || "", url: i.html_url,
      comments: comments.map((c) => ({ id: String(c.id), author: person(c.user), body: c.body || "", created: c.created_at, updated: c.updated_at, yours: !!me && c.user?.login?.toLowerCase() === me })),
      can: { edit: author || push, close: author || push, triage: push, delete: false },
    };
  },
  async open_issue(ctx, input) {
    const o = await open(ctx, input.repo);
    const r = await send(o.app, o.c, "POST", `/repos/${o.full}/issues`, {
      title: String(input.title || "").trim(), body: signed(ctx, o, input.body, "Filed"),
      ...(input.labels?.length ? { labels: input.labels } : {}), ...(input.assignees?.length ? { assignees: input.assignees.map((a) => a.replace(/^@/, "")) } : {}),
    });
    return { repo: o.name, ...issueShape(r.json), body: r.json.body || "", url: r.json.html_url };
  },
  async edit_issue(ctx, input) {
    const o = await open(ctx, input.repo);
    const change = {};
    if (input.title !== undefined) change.title = input.title;
    if (input.body !== undefined) change.body = input.body;
    if (input.labels !== undefined) change.labels = input.labels;
    if (input.assignees !== undefined) change.assignees = input.assignees.map((a) => a.replace(/^@/, ""));
    if (!Object.keys(change).length) throw new Error("give what changes: title, body, labels or assignees");
    const r = await send(o.app, o.c, "PATCH", `/repos/${o.full}/issues/${Number(input.number)}`, change);
    return { repo: o.name, ...issueShape(r.json), body: r.json.body || "" };
  },
  async close_issue(ctx, { repo, number, state_reason = "completed", comment }) {
    const o = await open(ctx, repo);
    if (comment?.trim()) await send(o.app, o.c, "POST", `/repos/${o.full}/issues/${Number(number)}/comments`, { body: signed(ctx, o, comment, "Commented") });
    await send(o.app, o.c, "PATCH", `/repos/${o.full}/issues/${Number(number)}`, { state: "closed", state_reason });
    return `Closed ${o.name}#${Number(number)} as ${state_reason.replace("_", " ")}.`;
  },
  async reopen_issue(ctx, { repo, number, comment }) {
    const o = await open(ctx, repo);
    if (comment?.trim()) await send(o.app, o.c, "POST", `/repos/${o.full}/issues/${Number(number)}/comments`, { body: signed(ctx, o, comment, "Commented") });
    await send(o.app, o.c, "PATCH", `/repos/${o.full}/issues/${Number(number)}`, { state: "open" });
    return `Reopened ${o.name}#${Number(number)}.`;
  },
  async delete_issue(ctx, { repo }) {
    throw ON_GITHUB(await open(ctx, repo), "delete an issue");
  },
  async comment(ctx, input) {
    const o = await open(ctx, input.repo);
    if (!String(input.body || "").trim()) throw new Error("a comment needs some text");
    const r = await send(o.app, o.c, "POST", `/repos/${o.full}/issues/${Number(input.number)}/comments`, { body: signed(ctx, o, input.body, "Commented") });
    return { repo: o.name, number: Number(input.number), id: String(r.json.id), author: person(r.json.user), body: r.json.body, created: r.json.created_at, updated: r.json.updated_at, yours: true };
  },
  async edit_comment(ctx, input) {
    const o = await open(ctx, input.repo);
    const r = await send(o.app, o.c, "PATCH", `/repos/${o.full}/issues/comments/${encodeURIComponent(input.id)}`, { body: input.body });
    return { repo: o.name, id: String(r.json.id), author: person(r.json.user), body: r.json.body, created: r.json.created_at, updated: r.json.updated_at, yours: true };
  },
  async delete_comment(ctx, input) {
    const o = await open(ctx, input.repo);
    await send(o.app, o.c, "DELETE", `/repos/${o.full}/issues/comments/${encodeURIComponent(input.id)}`);
    return `Deleted the comment on ${o.name}.`;
  },
  async labels(ctx, { repo }) {
    const o = await open(ctx, repo);
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/labels?per_page=100`);
    return r.json.map((l) => ({ name: l.name, color: l.color, description: l.description || "", open_issues: null }));
  },
  async set_label(ctx, input) {
    const o = await open(ctx, input.repo);
    const name = String(input.name || "").trim();
    const color = input.color === undefined ? undefined : String(input.color).replace("#", "").toLowerCase();
    const was = await send(o.app, o.c, "GET", `/repos/${o.full}/labels/${encodeURIComponent(name)}`, null, { ok: [404] });
    const r = was.status === 404
      ? await send(o.app, o.c, "POST", `/repos/${o.full}/labels`, { name: input.new_name || name, color: color || "ededed", description: input.description || "" })
      : await send(o.app, o.c, "PATCH", `/repos/${o.full}/labels/${encodeURIComponent(name)}`, { ...(input.new_name ? { new_name: input.new_name } : {}), ...(color ? { color } : {}), ...(input.description !== undefined ? { description: input.description } : {}) });
    return { name: r.json.name, color: r.json.color, description: r.json.description || "", open_issues: null };
  },
  async delete_label(ctx, { repo, name }) {
    const o = await open(ctx, repo);
    await send(o.app, o.c, "DELETE", `/repos/${o.full}/labels/${encodeURIComponent(name)}`);
    return `Deleted the label ${name} from ${o.name}.`;
  },
  async _people(ctx, { repo }) {
    const o = await open(ctx, repo);
    const r = await send(o.app, o.c, "GET", `/repos/${o.full}/assignees?per_page=100`, null, { ok: [403, 404] });
    return Array.isArray(r.json) ? r.json.map((u) => ({ handle: u.login, name: u.login })) : [];
  },
};

export const elsewhere = {
  collaborators: "see and change who pushes to it",
  add_collaborator: "add a collaborator",
  remove_collaborator: "remove a collaborator",
  fork: "fork it",
  delete: "delete it",
  restore: "restore it",
  erase: "erase it",
};
export async function refuse(ctx, op, repo) {
  const full = fullOf(repo);
  throw new Error(`${PREFIX}${full} is on GitHub: ${elsewhere[op]} there, at https://github.com/${full}`);
}

export async function list(ctx) {
  const { all, app } = await credentials(ctx);
  if (!app || !all.length) return [];
  // One sign-in GitHub refuses doesn't hide the others' repos; with none answering, it says why.
  const got = await Promise.allSettled(all.map((c) =>
    c.kind === "user"
      ? send(app, c, "GET", "/user/repos?per_page=100&sort=pushed").then((r) => r.json)
      : send(app, c, "GET", "/installation/repositories?per_page=100").then((r) => r.json.repositories)));
  if (got.every((g) => g.status === "rejected")) throw got[0].reason;
  const pages = got.map((g) => (g.status === "fulfilled" ? g.value : []));
  const seen = new Map();
  pages.forEach((repos, k) => repos.forEach((g) => {
    const key = g.full_name.toLowerCase();
    if (!seen.has(key)) seen.set(key, g);
    if (!reaches.has(`${ctx.me.user} ${key}`)) reaches.set(`${ctx.me.user} ${key}`, { who: all[k].login || all[k].org, time: Date.now() });
  }));
  return [...seen.values()].sort((a, b) => Date.parse(b.pushed_at || b.updated_at) - Date.parse(a.pushed_at || a.updated_at)).map((g) => ({
    repo: `${PREFIX}${g.full_name}`, name: g.name, owner: person(g.owner), from: null, branch: g.default_branch, branches: null,
    created: g.created_at, updated: g.pushed_at || g.updated_at, open_issues: g.open_issues_count ?? 0, deleted: null,
    yours: !!g.permissions?.admin, push: !!g.permissions?.push, github: { url: g.html_url, private: !!g.private, fork: !!g.fork },
  }));
}

const pending = async (ctx, what) => (await ctx.global.query("select value from github_pending where what = ?", [what]))[0]?.value || null;

export function tools({ global }) {
  const ACCOUNT = { type: "string", description: "a GitHub username, as repos_accounts lists them" };
  return {
    accounts: {
      reads: true,
      description: "The GitHub accounts you've signed in to Repos, the first used first: [{ account, ok }], with why when ok is false (repos_sign_in fixes it). Their repos show in repos_list as github.com/<owner>/<repo>.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const app = await appItem(ctx);
        return Promise.all((await accountItems(ctx)).map(async (item) => {
          const account = item.fields.login.value;
          if (!app) return { account, ok: false, why: noApp(ctx).message };
          try { await userAuth(ctx, app, item); return { account, ok: true }; } catch (e) { return { account, ok: false, why: e.message }; }
        }));
      },
    },
    sign_in: {
      reads: false,
      description: "Start signing a GitHub account in to Repos, so its repos show beside the network's own: gives { url, then }. The person opens url, lets the network's GitHub App act for them, and GitHub sends them back to finish.",
      input: { type: "object", properties: { account: { type: "string", description: "a GitHub username to suggest to GitHub, like one that needs signing in again" } } },
      async run(ctx, { account }) {
        if (!ctx.user) throw new Error("this device doesn't keep your data: use one of your own devices");
        const app = await appItem(ctx);
        if (!app) throw noApp(ctx);
        const q = new URLSearchParams({ client_id: app.fields.client_id.value, redirect_uri: redirect(ctx), state: "repos/settings", ...(account ? { login: account } : {}) });
        return { url: `${webOf(app)}/login/oauth/authorize?${q}`, then: "The person opens url and lets the App act for them; GitHub sends them back to Repos' Settings, which finishes. If it doesn't, give the address their browser came back to to repos_finish_sign_in." };
      },
    },
    finish_sign_in: {
      reads: false,
      description: "Finish signing in to GitHub with the address the browser came back to after repos_sign_in's url (Repos' Settings does it by itself). Gives { signed_in: true, account } or { signed_in: false, why }.",
      input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the whole address the browser came back to, like https://…/back?code=…&state=…, or just its code" } } },
      async run(ctx, { code: given }) {
        const app = await appItem(ctx);
        if (!app) throw noApp(ctx);
        const s = String(given || "").trim();
        const refused = s.match(/[?&]error_description=([^&#]+)/)?.[1] || s.match(/[?&]error=([^&#]+)/)?.[1];
        if (refused) return { signed_in: false, why: `GitHub didn't sign you in (${decodeURIComponent(refused.replace(/\+/g, " "))}). Try again.` };
        const code = s.includes("code=") ? new URL(s.startsWith("http") ? s : `http://x/?${s.split("?").pop()}`).searchParams.get("code") : s;
        if (!code) throw new Error("That address has no code. Copy the whole address the browser ended up on.");
        const r = await fetch(`${webOf(app)}/login/oauth/access_token`, {
          method: "POST",
          headers: { accept: "application/json", "content-type": "application/x-www-form-urlencoded" },
          body: `client_id=${encodeURIComponent(app.fields.client_id.value)}&client_secret={{${app.id}:client_secret}}&code=${encodeURIComponent(code)}&redirect_uri=${encodeURIComponent(redirect(ctx))}`,
        });
        const t = await r.json().catch(() => ({}));
        if (!r.ok || !t.access_token) return { signed_in: false, why: `GitHub didn't sign you in (${t.error_description || t.error || r.status}). Try again.` };
        const me = await fetch(`${apiOf(app)}/user`, { headers: { accept: "application/vnd.github+json", authorization: `Bearer ${t.access_token}`, "user-agent": "Repos" } }).then((x) => x.json()).catch(() => ({}));
        if (!me.login) return { signed_in: false, why: "GitHub didn't say which account signed in. Try again." };
        const id = await ctx.vault.create({
          title: `GitHub ${me.login}`, type: "GitHub account", domain: "github.com", hosts: app.hosts, same: "login",
          fields: { login: { value: me.login }, access_token: { value: t.access_token, secret: true }, ...(t.refresh_token ? { refresh_token: { value: t.refresh_token, secret: true } } : {}), expires: { value: String(t.expires_in ? Date.now() + t.expires_in * 1000 : 0) } },
        });
        userTokens.set(id, { value: t.access_token, expires: t.expires_in ? Date.now() + t.expires_in * 1000 : 0 });
        return { signed_in: true, account: me.login };
      },
    },
    sign_out: {
      reads: false,
      description: "Sign a GitHub account out of Repos: its repos no longer show for you. repos_sign_in signs it in again.",
      input: { type: "object", required: ["account"], properties: { account: ACCOUNT } },
      async run(ctx, { account }) {
        const item = (await accountItems(ctx)).find((i) => i.fields.login.value.toLowerCase() === String(account || "").replace(/^@/, "").toLowerCase());
        if (!item) throw new Error(`${account} isn't signed in to Repos: repos_accounts lists those that are`);
        ctx.vault.release(item.id);
        userTokens.delete(item.id);
        return `Signed ${item.fields.login.value} out of Repos.`;
      },
    },
    create_github_app: {
      reads: false,
      description: "For an admin, once for everyone: start making the network's GitHub App, with which people sign in to GitHub and groups connect organizations. Gives { url }: the admin opens it and confirms on GitHub, which sends them back to Repos' Settings to finish.",
      input: { type: "object", properties: {
        org: { type: "string", description: "a GitHub organization to own the App, by its login; omit for the admin's own account" },
        github: { type: "string", description: "GitHub's address if it isn't https://github.com (GitHub Enterprise)" },
      } },
      async run(ctx, { org, github = "https://github.com" }) {
        if (ctx.me.role !== "admin") throw new Error("only an admin makes the network's GitHub App: ask them");
        const web = String(github).replace(/\/+$/, "");
        const back = redirect(ctx);
        const host = new URL(ctx.address).host;
        const manifest = {
          name: host.slice(0, 34), url: ctx.address, redirect_url: back, callback_urls: [back], setup_url: back,
          hook_attributes: { url: `${ctx.address}/back`, active: false }, public: true, request_oauth_on_install: false,
          default_permissions: { contents: "read", issues: "write", metadata: "read" },
        };
        global(ctx).exec("insert or replace into github_pending values ('app', ?, ?)", [JSON.stringify({ web }), ctx.now()]);
        const to = `${web}/${org ? `organizations/${encodeURIComponent(org)}/` : ""}settings/apps/new?state=repos/github_app`;
        return { url: `${ctx.address}/post.html#${new URLSearchParams({ to, manifest: JSON.stringify(manifest) })}`, then: "The admin opens url, continues to GitHub and creates the App; GitHub sends them back to Repos' Settings, which finishes." };
      },
    },
    finish_github_app: {
      reads: false,
      description: "Finish making the network's GitHub App with the address GitHub sent the admin back to (Repos' Settings does it by itself).",
      input: { type: "object", required: ["code"], properties: { code: { type: "string", description: "the whole address GitHub sent the browser back to, or its code" } } },
      async run(ctx, { code: given }) {
        if (ctx.me.role !== "admin") throw new Error("only an admin makes the network's GitHub App");
        const s = String(given || "");
        const code = s.includes("code=") ? new URL(s).searchParams.get("code") : s;
        const web = JSON.parse((await pending(ctx, "app")) || "{}").web || "https://github.com";
        const api = web === "https://github.com" ? "https://api.github.com" : `${web}/api/v3`;
        const r = await fetch(`${api}/app-manifests/${encodeURIComponent(code)}/conversions`, { method: "POST", headers: { accept: "application/vnd.github+json", "user-agent": "Repos" } });
        const a = await r.json().catch(() => ({}));
        if (!r.ok || !a.id) throw new Error(`GitHub didn't make the App (${a.message || r.status}): start again from Repos › Settings`);
        const hosts = [...new Set([new URL(web).hostname, new URL(api).hostname])];
        await ctx.vault.create({
          scope: "global", title: `GitHub App ${a.slug}`, type: "GitHub App", domain: "github.com", hosts, same: "client_id",
          fields: {
            app_id: { value: String(a.id) }, slug: { value: a.slug }, client_id: { value: a.client_id }, owner: { value: a.owner?.login || "" },
            client_secret: { value: a.client_secret, secret: true }, private_key: { value: a.pem, secret: true },
            ...(web !== "https://github.com" ? { web: { value: web }, api: { value: api } } : {}),
          },
        });
        global(ctx).exec("delete from github_pending where what = 'app'");
        return { made: true, app: a.slug };
      },
    },
    connect_org: {
      reads: false,
      description: "For a group's admins: connect a GitHub organization, so its repos show for everyone in the group, read and written as the network's GitHub App. Gives { url }, where they install the App on the organization; GitHub sends them back to Repos' Settings to finish.",
      input: { type: "object", required: ["group"], properties: { group: { type: "string", description: "the group, by handle, from groups_list" } } },
      async run(ctx, { group }) {
        const g = (ctx.handles || []).find((h) => h.id.startsWith("group:") && h.handle === String(group || "").replace(/^@/, ""));
        if (!g) throw new Error(`no group has the handle ${group}: groups_list has them`);
        if (!adminOf(ctx, g.id)) throw new Error(`only ${g.name}'s admins connect an organization for it`);
        const app = await appItem(ctx);
        if (!app) throw noApp(ctx);
        global(ctx).exec("insert or replace into github_pending values (?, ?, ?)", [`org ${ctx.me.user}`, g.id, ctx.now()]);
        return { url: `${webOf(app)}/apps/${app.fields.slug.value}/installations/new?state=repos/settings`, then: "They pick the organization and install the App; GitHub sends them back to Repos' Settings, which finishes." };
      },
    },
    finish_connect_org: {
      reads: false,
      description: "Finish connecting an organization with the address GitHub sent the browser back to after installing the App (Repos' Settings does it by itself).",
      input: { type: "object", required: ["back"], properties: { back: { type: "string", description: "the whole address GitHub sent the browser back to, with installation_id" } } },
      async run(ctx, { back }) {
        const installation = Number(new URL(back).searchParams.get("installation_id"));
        if (!installation) throw new Error("That address has no installation_id: install the App from Repos › Settings again");
        const group = await pending(ctx, `org ${ctx.me.user}`);
        if (!group || !adminOf(ctx, group)) throw new Error("No organization is waiting to be connected: start from Repos › Settings › Connect an Organization");
        const app = await appItem(ctx);
        if (!app) throw noApp(ctx);
        const auth = await installAuth(app, installation);
        const r = await fetch(`${apiOf(app)}/installation/repositories?per_page=1`, { headers: { accept: "application/vnd.github+json", authorization: auth, "user-agent": "Repos" } });
        const d = await r.json().catch(() => ({}));
        if (!r.ok) throw new Error(`GitHub: ${d.message || r.status}`);
        const org = d.repositories?.[0]?.owner?.login || (await fetch(`${apiOf(app)}/app/installations/${installation}`, { headers: { accept: "application/vnd.github+json", authorization: `Bearer {{rs256 ${app.id}:private_key ${b64url(JSON.stringify({ iat: Math.floor(Date.now() / 1000) - 60, exp: Math.floor(Date.now() / 1000) + 540, iss: app.fields.app_id.value }))}}}`, "user-agent": "Repos" } }).then((x) => x.json()).catch(() => ({}))).account?.login;
        if (!org) throw new Error("GitHub didn't say which organization the App was installed on: try again");
        const g = global(ctx);
        g.exec("insert or replace into github_orgs values (?, ?, ?, ?, ?)", [installation, org, group, ctx.me.user, ctx.now()]);
        g.exec("delete from github_pending where what = ?", [`org ${ctx.me.user}`]);
        return { connected: true, org, group: groupOf(ctx, group)?.name };
      },
    },
    disconnect_org: {
      reads: false,
      confirm: true,
      description: "For a group's admins: stop showing a connected GitHub organization's repos to the group. The App stays installed on GitHub until its owners uninstall it there.",
      input: { type: "object", required: ["org"], properties: { org: { type: "string", description: "the organization's login, as repos_github lists it" }, confirm: { type: "boolean", description: "true once the person agreed" } } },
      async run(ctx, { org }) {
        const g = global(ctx);
        const rows = (await g.query("select * from github_orgs where lower(org) = lower(?)", [String(org || "")])).filter((o) => adminOf(ctx, o.owner));
        if (!rows.length) throw new Error(`no organization ${org} is connected for a group you're an admin of`);
        for (const o of rows) g.exec("delete from github_orgs where installation = ?", [o.installation]);
        return `${rows[0].org}'s repos no longer show for ${groupOf(ctx, rows[0].owner)?.name || "the group"}.`;
      },
    },
    github: {
      reads: true,
      description: "How GitHub is connected: the network's GitHub App (null until an admin makes it), your GitHub accounts, the organizations connected for groups you're in (and for each whether you're its group's admin), and the groups you may connect one for.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const app = await appItem(ctx);
        const rows = ctx.global ? await ctx.global.query("select installation, org, owner, user, time from github_orgs order by org") : [];
        return {
          app: app ? { slug: app.fields.slug?.value || null, url: `${webOf(app)}/apps/${app.fields.slug?.value || ""}` } : null,
          admin: ctx.me.role === "admin",
          accounts: (await accountItems(ctx)).map((i) => i.fields.login.value),
          orgs: rows.filter((o) => inGroup(ctx, o.owner)).map((o) => ({ org: o.org, group: groupOf(ctx, o.owner)?.name || null, yours: adminOf(ctx, o.owner), time: new Date(o.time).toISOString() })),
          groups: (ctx.handles || []).filter((h) => h.id.startsWith("group:") && adminOf(ctx, h.id)).map((h) => ({ handle: h.handle, name: h.name })),
        };
      },
    },
  };
}
