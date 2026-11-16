// The plugin directory: every plugin anyone has installed, ranked by installs. Its pages, its API (README.md) and
// its counts, in D1. A plugin is listed the first time an installer says it installed it, once its SKILL.md is
// found on GitHub.

const OWNER = /^[A-Za-z0-9-]{1,39}$/;
const REPO = /^[A-Za-z0-9._-]{1,100}$/;
const NAME = /^[a-z0-9][a-z0-9-]{0,63}$/;
const COMMIT = /^[0-9a-f]{7,40}$/;
const DAY = 24 * 60 * 60 * 1000;

/** A SKILL.md's front matter: its `name:` and `description:` lines. */
export function frontMatter(text) {
  const block = text.match(/^---\r?\n([\s\S]*?)\r?\n---/)?.[1] ?? "";
  const field = (key) => block.match(new RegExp(`^${key}:\\s*(.+)$`, "m"))?.[1].trim().replace(/^(["'])(.*)\1$/, "$2");
  return { name: field("name"), description: field("description") };
}

const day = (ms) => new Date(ms).toISOString().slice(0, 10);

/** Plugins with their installs since a day, most first. */
async function ranked(env, period) {
  const since = period === "week" ? day(Date.now() - 6 * DAY) : "";
  const { results } = await env.DB.prepare(
    `SELECT p.owner, p.repo, p.name, p.description, COALESCE(SUM(i.count), 0) AS installs
     FROM plugins p LEFT JOIN installs i ON i.owner = p.owner AND i.repo = p.repo AND i.name = p.name AND i.day >= ?1
     GROUP BY p.owner, p.repo, p.name ORDER BY installs DESC, p.name`,
  ).bind(since).all();
  return results.map((p) => ({ ...p, source: `${p.owner}/${p.repo}` }));
}

const code = (p) => `https://github.com/${p.owner}/${p.repo}/tree/HEAD/plugins/${p.name}`;
const command = (p) => `one install ${p.owner}/${p.repo} --plugin ${p.name}`;

async function install(request, env) {
  // Rate limited per address at the edge; the address is a key to Cloudflare's counter only, and never kept.
  const { success } = await env.INSTALLS.limit({ key: request.headers.get("cf-connecting-ip") ?? "" });
  if (!success) return Response.json({ error: "Too many installs from here. Try again in a minute." }, { status: 429 });

  const body = await request.json().catch(() => null);
  const [owner, repo, extra] = String(body?.source ?? "").split("/");
  const name = body?.plugin;
  const commit = body?.commit;
  if (!OWNER.test(owner ?? "") || !REPO.test(repo ?? "") || extra !== undefined || !NAME.test(name ?? "") || (commit !== undefined && !COMMIT.test(commit))) {
    return Response.json({ error: 'Send {"source": "owner/repo", "plugin": "name", "commit": "sha" (optional)}.' }, { status: 400 });
  }
  const key = [owner.toLowerCase(), repo.toLowerCase(), name];

  const known = await env.DB.prepare("SELECT checked FROM plugins WHERE owner = ?1 AND repo = ?2 AND name = ?3").bind(...key).first();
  if (!known || Date.parse(known.checked) < Date.now() - DAY) {
    const response = await fetch(`https://raw.githubusercontent.com/${owner}/${repo}/${commit ?? "HEAD"}/plugins/${name}/SKILL.md`);
    const { description } = response.ok ? frontMatter(await response.text()) : {};
    if (description) {
      await env.DB.prepare(
        `INSERT INTO plugins (owner, repo, name, description, checked) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT DO UPDATE SET description = excluded.description, checked = excluded.checked`,
      ).bind(...key, description, new Date().toISOString()).run();
    } else if (!known) {
      return Response.json({ error: `No plugins/${name}/SKILL.md with a description in github.com/${owner}/${repo}.` }, { status: 404 });
    }
    // A plugin already listed is still counted when GitHub doesn't answer; its description is read again tomorrow.
  }

  await env.DB.prepare(
    `INSERT INTO installs (owner, repo, name, day, count) VALUES (?1, ?2, ?3, ?4, 1)
     ON CONFLICT DO UPDATE SET count = count + 1`,
  ).bind(...key, day(Date.now())).run();
  return new Response(null, { status: 204 });
}

const escape = (text) => String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const number = (n) => n.toLocaleString("en");
const installs = (n) => `${number(n)} ${n === 1 ? "install" : "installs"}`;

const TRUST = `<p class="trust">A plugin's server code runs on your own machine, with full access to it. Nothing here is reviewed by us: have your agent read a plugin before you install it. Plugins are listed and ranked by anonymous install counts; installing with <code>DO_NOT_TRACK=1</code> sends none.</p>`;

const page = (title, body) =>
  new Response(
    `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title>
<link rel="stylesheet" href="/plugins/style.css">
</head>
<body>
<header><a href="/">The One App</a></header>
<main>
${body}
${TRUST}
</main>
</body>
</html>`,
    { headers: { "content-type": "text/html; charset=utf-8" } },
  );

function list(plugins, period) {
  const tab = (value, label) => `<a href="/plugins${value === "all" ? "" : `?period=${value}`}"${period === value ? ' aria-current="page"' : ""}>${label}</a>`;
  const rows = plugins
    .map(
      (p, i) => `<li data-search="${escape(`${p.name} ${p.source} ${p.description}`.toLowerCase())}">
  <a href="/plugins/${p.source}/${p.name}">
    <span class="rank">${i + 1}</span>
    <span class="what"><span class="name">${escape(p.name)}</span> <span class="source">${escape(p.source)}</span><span class="description">${escape(p.description)}</span></span>
    <span class="count">${number(p.installs)}</span>
  </a>
</li>`,
    )
    .join("\n");
  return page(
    "Plugins · The One App",
    `<h1>Plugins</h1>
<p class="lead">Plugins for The One App, from any git repo, ranked by installs.</p>
<div class="controls">
  <input type="search" placeholder="Search" aria-label="Search plugins" autocomplete="off">
  <nav class="tabs">${tab("all", "All Time")}${tab("week", "This Week")}</nav>
</div>
<ol class="plugins">
${rows}
</ol>
<p class="empty" hidden>No plugins match.</p>
<script>
  const search = document.querySelector("input[type=search]");
  const rows = [...document.querySelectorAll(".plugins li")];
  search.addEventListener("input", () => {
    const words = search.value.toLowerCase().split(/\\s+/).filter(Boolean);
    for (const row of rows) row.hidden = !words.every((word) => row.dataset.search.includes(word));
    document.querySelector(".empty").hidden = rows.some((row) => !row.hidden);
  });
</script>`,
  );
}

function plugin(p) {
  return page(
    `${p.name} · Plugins · The One App`,
    `<p class="back"><a href="/plugins">Plugins</a></p>
<h1>${escape(p.name)}</h1>
<p class="source"><a href="https://github.com/${p.source}">${escape(p.source)}</a> · ${installs(p.installs)}</p>
<p>${escape(p.description)}</p>
<h2>Install</h2>
<div class="command"><code>${escape(command(p))}</code><button type="button">Copy</button></div>
<h2>Code</h2>
<p><a href="${code(p)}">${escape(`github.com/${p.source}/tree/HEAD/plugins/${p.name}`)}</a></p>
<script>
  const button = document.querySelector(".command button");
  button.addEventListener("click", async () => {
    await navigator.clipboard.writeText(document.querySelector(".command code").textContent);
    button.textContent = "Copied";
    setTimeout(() => (button.textContent = "Copy"), 1500);
  });
</script>`,
  );
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    const path = url.pathname.replace(/\/+$/, "");
    const period = url.searchParams.get("period") === "week" ? "week" : "all";

    if (path === "/plugins/api/installs") {
      if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
      return install(request, env);
    }
    if (request.method !== "GET" && request.method !== "HEAD") return new Response(null, { status: 405, headers: { allow: "GET" } });
    if (path === "/plugins/api/plugins") {
      const plugins = (await ranked(env, period)).map((p) => ({ source: p.source, name: p.name, description: p.description, installs: p.installs, code: code(p), install: command(p) }));
      return Response.json({ period, plugins }, { headers: { "access-control-allow-origin": "*" } });
    }
    if (path === "/plugins") return list(await ranked(env, period), period);

    const [owner, repo, name, extra] = path.split("/").slice(2);
    const found = extra === undefined && (await ranked(env, "all")).find((p) => p.owner === owner?.toLowerCase() && p.repo === repo?.toLowerCase() && p.name === name);
    if (found) return plugin(found);
    return new Response(page("Not found · Plugins · The One App", `<p class="back"><a href="/plugins">Plugins</a></p><p class="empty">No plugin is here.</p>`).body, {
      status: 404,
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  },
};
