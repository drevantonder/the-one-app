// The plugin directory: every plugin anyone has installed, ranked by installs. Its pages, its API (README.md) and
// its counts, in D1. A plugin is listed the first time an installer says it installed it, once its SKILL.md is
// found on GitHub. Each is listed as a network's marketplace lists it (`Listing`): what its SKILL.md says, read from
// GitHub, and what its code says (its title, icon, apps and widgets), as the network that installed it gave it.

const OWNER = /^[A-Za-z0-9-]{1,39}$/;
const REPO = /^[A-Za-z0-9._-]{1,100}$/;
const NAME = /^[a-z0-9][a-z0-9-]{0,63}$/;
const COMMIT = /^[0-9a-f]{7,40}$/;
const DAY = 24 * 60 * 60 * 1000;

/** A SKILL.md's front matter: its `name:` and `description:` lines, and its `metadata`'s `subtitle:`, `about:` and
 * `category:`. */
export function frontMatter(text: string): { name?: string; description?: string; subtitle?: string; about?: string; category?: string } {
  const block = text.match(/^---\r?\n([\s\S]*?)\r?\n---/)?.[1] ?? "";
  const field = (key: string, indent = "") => block.match(new RegExp(`^${indent}${key}:\\s*(.+)$`, "m"))?.[1].trim().replace(/^(["'])(.*)\1$/, "$2");
  return { name: field("name"), description: field("description"), subtitle: field("subtitle", "[ \\t]+"), about: field("about", "[ \\t]+"), category: field("category", "[ \\t]+") };
}

/** What a network's server says of a plugin where it's listed (`OneCore.Plugins.Version.listing/1`), as its
 * marketplace and this directory show it, beside what its SKILL.md says. */
export interface Listing {
  title: string;
  icon: string | null;
  apps: { name: string; icon: string | null; category: string | null }[];
  widgets: { name: string; sizes: string[] }[];
  requires: string[];
}

const ICON = /^data:image\/(png|jpeg|webp|gif|svg\+xml);base64,[A-Za-z0-9+/]+=*$/;
const text = (value: unknown, most = 200) => (typeof value === "string" && value.length <= most ? value : null);
const icon = (value: unknown) => (typeof value === "string" && value.length <= 65536 && ICON.test(value) ? value : null);
const each = <T,>(value: unknown, one: (item: any) => T | null) =>
  (Array.isArray(value) ? value.slice(0, 50) : []).map(one).filter((item): item is T => item !== null);

/** What an installer said of a plugin, as a `Listing`, or `null` where it said nothing that is one. */
export function listing(value: any): Listing | null {
  const title = text(value?.title);
  if (!title) return null;
  return {
    title,
    icon: icon(value.icon),
    apps: each(value.apps, (app) => (text(app?.name) ? { name: app.name, icon: icon(app.icon), category: text(app.category) } : null)),
    widgets: each(value.widgets, (widget) => (text(widget?.name) ? { name: widget.name, sizes: each(widget.sizes, (size) => text(size, 20)) } : null)),
    requires: each(value.requires, (name) => (typeof name === "string" && NAME.test(name) ? name : null)),
  };
}

const day = (ms: number) => new Date(ms).toISOString().slice(0, 10);

type Period = "all" | "week";

/** A row of `plugins`, with its installs in the period asked for. */
interface Row {
  owner: string;
  repo: string;
  name: string;
  description: string;
  subtitle: string | null;
  about: string | null;
  category: string | null;
  listing: string | null;
  installs: number;
}

/** A plugin as it's listed, by where its code is, with its installs. */
type Plugin = Omit<Row, "listing"> & Listing & { source: string };

const UNLISTED: Listing = { title: "", icon: null, apps: [], widgets: [], requires: [] };

/** Plugins with their installs since a day, most first. */
async function ranked(env: Env, period: Period): Promise<Plugin[]> {
  const since = period === "week" ? day(Date.now() - 6 * DAY) : "";
  const { results } = await env.DB.prepare(
    `SELECT p.owner, p.repo, p.name, p.description, p.subtitle, p.about, p.category, p.listing, COALESCE(SUM(i.count), 0) AS installs
     FROM plugins p LEFT JOIN installs i ON i.owner = p.owner AND i.repo = p.repo AND i.name = p.name AND i.day >= ?1
     GROUP BY p.owner, p.repo, p.name ORDER BY installs DESC, p.name`,
  ).bind(since).all<Row>();
  return results.map(({ listing, ...p }) => ({ ...p, ...UNLISTED, title: p.name, ...(listing ? JSON.parse(listing) : {}), source: `${p.owner}/${p.repo}` }));
}

const code = (p: Plugin) => `https://github.com/${p.owner}/${p.repo}/tree/HEAD/plugins/${p.name}`;
const command = (p: Plugin) => `one install ${p.owner}/${p.repo} --plugin ${p.name}`;

async function install(request: Request, env: Env): Promise<Response> {
  // Rate limited per address at the edge; the address is a key to Cloudflare's counter only, and never kept.
  const { success } = await env.INSTALLS.limit({ key: request.headers.get("cf-connecting-ip") ?? "" });
  if (!success) return Response.json({ error: "Too many installs from here. Try again in a minute." }, { status: 429 });

  const body: Partial<Record<"source" | "plugin" | "commit" | "listing", unknown>> | null = await request.json<{}>().catch(() => null);
  const [owner = "", repo = "", extra] = String(body?.source ?? "").split("/");
  const name = String(body?.plugin ?? "");
  const commit = body?.commit === undefined ? undefined : String(body.commit);
  if (!OWNER.test(owner) || !REPO.test(repo) || extra !== undefined || !NAME.test(name) || (commit !== undefined && !COMMIT.test(commit))) {
    return Response.json({ error: 'Send {"source": "owner/repo", "plugin": "name", "commit": "sha" (optional)}.' }, { status: 400 });
  }
  const key = [owner.toLowerCase(), repo.toLowerCase(), name];

  const known = await env.DB.prepare("SELECT checked FROM plugins WHERE owner = ?1 AND repo = ?2 AND name = ?3").bind(...key).first<{ checked: string }>();
  if (!known || Date.parse(known.checked) < Date.now() - DAY) {
    const response = await fetch(`https://raw.githubusercontent.com/${owner}/${repo}/${commit ?? "HEAD"}/plugins/${name}/SKILL.md`);
    const { description, subtitle, about, category } = response.ok ? frontMatter(await response.text()) : {};
    if (description) {
      await env.DB.prepare(
        `INSERT INTO plugins (owner, repo, name, description, subtitle, about, category, checked) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT DO UPDATE SET description = excluded.description, subtitle = excluded.subtitle, about = excluded.about, category = excluded.category, checked = excluded.checked`,
      ).bind(...key, description, subtitle ?? null, about ?? null, category ?? null, new Date().toISOString()).run();
    } else if (!known) {
      return Response.json({ error: `No plugins/${name}/SKILL.md with a description in github.com/${owner}/${repo}.` }, { status: 404 });
    }
    // A plugin already listed is still counted when GitHub doesn't answer; its description is read again tomorrow.
  }
  const listed = listing(body?.listing);
  if (listed) await env.DB.prepare("UPDATE plugins SET listing = ?4 WHERE owner = ?1 AND repo = ?2 AND name = ?3").bind(...key, JSON.stringify(listed)).run();

  await env.DB.prepare(
    `INSERT INTO installs (owner, repo, name, day, count) VALUES (?1, ?2, ?3, ?4, 1)
     ON CONFLICT DO UPDATE SET count = count + 1`,
  ).bind(...key, day(Date.now())).run();
  return new Response(null, { status: 204 });
}

const escape = (text: unknown) => String(text).replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
const number = (n: number) => n.toLocaleString("en");
const installs = (n: number) => `${number(n)} ${n === 1 ? "install" : "installs"}`;

const TRUST = `<p class="trust">A plugin's server code runs on your own machine, with full access to it. Nothing here is reviewed by us: have your agent read a plugin before you install it. Plugins are listed and ranked by anonymous install counts; installing with <code>DO_NOT_TRACK=1</code> sends none.</p>`;

const ORIGIN = "https://one.drevan.me";
const CARD = { url: `${ORIGIN}/social-card.png`, alt: "The One App: make using AI to its full potential easy." };

/** The tags that make a link to a page a card in a chat or on social media, and its canonical address. */
const tags = (title: string, description: string, path: string) => {
  const url = `${ORIGIN}${path}`;
  const text = escape(description.length > 200 ? `${description.slice(0, 199).trimEnd()}…` : description);
  return `<meta name="description" content="${text}">
<link rel="canonical" href="${escape(url)}">
<link rel="icon" href="/ring.png">
<meta name="theme-color" content="#0a0a0a" media="(prefers-color-scheme: dark)">
<meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)">
<meta property="og:type" content="website">
<meta property="og:site_name" content="The One App">
<meta property="og:title" content="${escape(title)}">
<meta property="og:description" content="${text}">
<meta property="og:url" content="${escape(url)}">
<meta property="og:image" content="${CARD.url}">
<meta property="og:image:type" content="image/png">
<meta property="og:image:width" content="1200">
<meta property="og:image:height" content="630">
<meta property="og:image:alt" content="${CARD.alt}">
<meta name="twitter:card" content="summary_large_image">
<meta name="twitter:title" content="${escape(title)}">
<meta name="twitter:description" content="${text}">
<meta name="twitter:image" content="${CARD.url}">
<meta name="twitter:image:alt" content="${CARD.alt}">
`;
};

/** A page; `meta` is `[description, path]` for one that's linked to, and none for the not-found page. */
const page = (title: string, body: string, meta?: [description: string, path: string]) =>
  new Response(
    `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escape(title)}</title>
${meta ? tags(title, ...meta) : '<meta name="robots" content="noindex">\n<link rel="icon" href="/ring.png">\n'}<link rel="stylesheet" href="/plugins/style.css">
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

/** A plugin's icon, or a blank square where it has none. */
const image = (src: string | null, size = "") => (src ? `<img class="icon${size}" src="${escape(src)}" alt="">` : `<span class="icon${size}"></span>`);

function list(plugins: Plugin[], period: Period, category: string | null) {
  const link = (period: Period, category: string | null) => {
    const query = new URLSearchParams({ ...(period === "all" ? {} : { period }), ...(category ? { category } : {}) }).toString();
    return `/plugins${query ? `?${query}` : ""}`;
  };
  const tab = (value: Period, label: string) => `<a href="${escape(link(value, category))}"${period === value ? ' aria-current="page"' : ""}>${label}</a>`;
  // What plugins are for, most installed first.
  const categories = [...new Set(plugins.map((p) => p.category).filter((c): c is string => !!c))];
  const chip = (value: string | null, label: string) => `<a href="${escape(link(period, value))}"${category === value ? ' aria-current="page"' : ""}>${escape(label)}</a>`;
  const rows = plugins
    .filter((p) => !category || p.category === category)
    .map(
      (p, i) => `<li data-search="${escape(`${p.name} ${p.title} ${p.source} ${p.category ?? ""} ${p.subtitle ?? ""} ${p.about ?? ""}`.toLowerCase())}">
  <a href="/plugins/${p.source}/${p.name}">
    <span class="rank">${i + 1}</span>
    ${image(p.icon)}
    <span class="what"><span class="name">${escape(p.title)}</span> <span class="source">${escape(p.source)}</span><span class="subtitle">${escape(p.subtitle ?? "")}</span></span>
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
<nav class="categories">${chip(null, "All")}${categories.map((c) => chip(c, c)).join("")}</nav>
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
    ["Plugins for The One App, from any git repo, ranked by installs.", link(period, category)],
  );
}

function plugin(p: Plugin) {
  const apps = p.apps.map((app) => `<li>${image(app.icon, " small")}<span>${escape(app.name)}</span></li>`).join("");
  const widgets = p.widgets.map((widget) => `<li><span>${escape(widget.name)}</span> <span class="source">${escape(widget.sizes.join(", "))}</span></li>`).join("");
  return page(
    `${p.title} · Plugins · The One App`,
    `<p class="back"><a href="/plugins">Plugins</a></p>
<div class="heading">${image(p.icon, " large")}<div><h1>${escape(p.title)}</h1>
${p.subtitle ? `<p class="subtitle">${escape(p.subtitle)}</p>` : ""}</div></div>
<p class="source">${p.category ? `<a href="/plugins?category=${encodeURIComponent(p.category)}">${escape(p.category)}</a> · ` : ""}<a href="https://github.com/${p.source}">${escape(p.source)}</a> · ${installs(p.installs)}</p>
${p.about ? `<p>${escape(p.about)}</p>\n` : ""}${apps ? `<h2>Apps</h2>\n<ul class="parts">${apps}</ul>\n` : ""}${widgets ? `<h2>Widgets</h2>\n<ul class="parts">${widgets}</ul>\n` : ""}${p.requires.length ? `<h2>Requires</h2>\n<p>${escape(p.requires.join(", "))}</p>\n` : ""}<h2>Install</h2>
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
    [p.about ?? "", `/plugins/${p.source}/${p.name}`],
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
      const plugins = (await ranked(env, period)).map((p) => ({
        source: p.source, name: p.name, title: p.title, icon: p.icon, subtitle: p.subtitle, about: p.about, category: p.category,
        apps: p.apps, widgets: p.widgets, requires: p.requires, description: p.description, installs: p.installs, code: code(p), install: command(p),
      }));
      return Response.json({ period, plugins }, { headers: { "access-control-allow-origin": "*" } });
    }
    if (path === "/plugins") return list(await ranked(env, period), period, url.searchParams.get("category"));

    const [owner, repo, name, extra] = path.split("/").slice(2);
    const found = extra === undefined && (await ranked(env, "all")).find((p) => p.owner === owner?.toLowerCase() && p.repo === repo?.toLowerCase() && p.name === name);
    if (found) return plugin(found);
    return new Response(page("Not found · Plugins · The One App", `<p class="back"><a href="/plugins">Plugins</a></p><p class="empty">No plugin is here.</p>`).body, {
      status: 404,
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  },
} satisfies ExportedHandler<Env>;
