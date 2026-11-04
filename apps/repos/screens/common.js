// What every repo page shares: links, the path bar and the latest-commit card.
const P = one.params;
const esc = one.esc;
// How long ago an ISO 8601 time from an operation was.
const ago = (iso, opts) => one.ago(Date.parse(iso), opts);
const short = (sha) => String(sha || "").slice(0, 7);
const wide = () => matchMedia("(min-width: 700px)").matches;
// A script that opens a screen, for an onclick="…".
const go = (screen, params) => esc(`one.open(${JSON.stringify(screen)}, ${JSON.stringify(params)})`);
// The ref to keep using: the branch if there is one, else the commit.
const refOf = (d) => d.branch || d.sha;
// Where git reaches a repo, the same on every computer: <address>/repos/<owner>/<repo>.git, once `based` has it; and
// the person's handle, for a repo of theirs. Git goes through the CLI there, so on a phone or in a browser this says
// where to run it.
let base = "<address>/repos/", mine = "<you>";
const based = one.call("_address").then((b) => ((base = b?.base || base), (mine = b?.handle || mine)), () => {});
const address = (repo) => `${base}${repo}.git`;
const where = `On any computer where you’ve run ${one.config?.cli || "one"} sign-in`;
// A repo of GitHub's, connected: github.com/<owner>/<name>.
const isGithub = (repo) => String(repo || "").startsWith("github.com/");
const plural = (n, one_, many) => `${n.toLocaleString()} ${n === 1 ? one_ : many}`;
const avatar = (c, cls = "") => `<span class="who-avatar ${cls}">${one.avatar(c.email || c.author, c.author)}</span>`;
one.haptics();

/// The branch chip and the path, each part tappable to go up; at an older commit, the chip is its hash.
function pathBar(d, { file = false } = {}) {
  const parts = (d.path || "").split("/").filter(Boolean);
  const ref = refOf(d);
  const chip = d.branch
    ? `<button class="chip" onclick="${go("branches", { repo: d.repo, path: d.path || "" })}">${one.icon("git-branch")}<span>${esc(d.branch)}</span>${one.icon("caret-down", "chev")}</button>`
    : `<button class="chip" onclick="${go("branches", { repo: d.repo, path: d.path || "" })}">${one.icon("git-commit")}<span>${short(d.sha)}</span>${one.icon("caret-down", "chev")}</button>` +
      `<button class="button small-button" onclick="${go("repo", { repo: d.repo })}">Back to main</button>`;
  const crumbs = [`<a onclick="${go("repo", { repo: d.repo, ref })}">${esc(d.repo)}</a>`];
  parts.forEach((p, k) => {
    const last = k === parts.length - 1;
    crumbs.push(last ? `<strong>${esc(p)}</strong>` : `<a onclick="${go("repo", { repo: d.repo, ref, path: parts.slice(0, k + 1).join("/") })}">${esc(p)}</a>`);
  });
  if (!parts.length && !file) return `<div class="pathbar">${chip}</div>`;
  return `<div class="pathbar">${chip}<nav class="crumbs" aria-label="Path">${crumbs.join('<span class="sep">/</span>')}${file ? one.copyButton(d.path, { label: "", cls: "path-copy" }) : ""}</nav></div>`;
}

/// The newest commit here, with History: a bar on wide windows, a card with a History row on phones.
function commitCard(c, d) {
  const history = go("history", { repo: d.repo, ref: refOf(d), path: d.path || "" });
  return `<div class="latest">
    <a class="latest-main" onclick="${go("commit", { repo: d.repo, sha: c.sha })}">
      ${avatar(c, "small")}<span class="latest-author">${esc(c.author)}</span>
      <span class="latest-subject">${esc(c.subject)}</span>
      <span class="latest-meta"><code>${short(c.sha)}</code> · ${ago(c.time, { long: true })}</span>
    </a>
    <a class="latest-history" onclick="${history}">${one.icon("clock-counter-clockwise")}<span>History</span><span class="chev-row">${one.icon("caret-right")}</span></a>
  </div>`;
}

// Issues: a label as GitHub draws one, a pill in its colour with its name in black or white, whichever reads.
function labelChip(l) {
  const c = /^[0-9a-f]{6}$/i.test(l.color) ? l.color : "ededed";
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(c.slice(i, i + 2), 16));
  return `<span class="label" style="background:#${c};color:${0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#1f2328" : "#fff"}">${esc(l.name)}</span>`;
}
// An issue's state as its symbol: open, closed as completed, or closed as not planned.
const stateIcon = (i, cls = "state-icon") => one.icon(i.state === "open" ? "record" : i.state_reason === "not_planned" ? "prohibit" : "check-circle", `${cls} ${i.state}`);
const person = (p) => `<span class="who-avatar tiny">${one.avatar(p.handle || p.name, p.name)}</span>`;

// A Markdown field with Write and Preview, as GitHub's: `id` names its textarea.
function markdownField(id, { value = "", placeholder = "", rows = 6 } = {}) {
  return `<div class="md-field" data-field="${id}">
    <div class="segmented narrow"><button class="on" data-write>Write</button><button data-preview>Preview</button></div>
    <textarea id="${id}" class="md-input" rows="${rows}" placeholder="${esc(placeholder)}">${esc(value)}</textarea>
    <div class="md-preview" hidden></div>
  </div>`;
}
document.addEventListener("click", (e) => {
  const b = e.target.closest?.(".md-field [data-write], .md-field [data-preview]");
  if (!b) return;
  const f = b.closest(".md-field"), preview = "preview" in b.dataset, text = f.querySelector("textarea"), out = f.querySelector(".md-preview");
  f.querySelectorAll(".segmented button").forEach((x) => x.classList.toggle("on", x === b));
  if (preview) out.innerHTML = text.value.trim() ? one.markdown(text.value) : '<p class="muted">Nothing to preview.</p>';
  text.hidden = preview;
  out.hidden = !preview;
});

// A repo's name and GitHub's tab row over its Code, Issues and Settings (Settings for its owners), on wide windows:
// each tab is its own screen, opened with the repo in its params, so Back and reload return to it. On phones it's
// hidden, and the repo's page has rows that open them.
function repoTabs(d, on) {
  const parts = d.repo.split("/"), name = parts.pop(), owner = parts.join("/");
  // A GitHub repo's settings are GitHub's own.
  const settings = d.yours && !isGithub(d.repo);
  const tab = (key, screen, icon, title, extra = "") =>
    `<a class="tab${on === key ? " on" : ""}"${on === key ? ' aria-current="page"' : ` onclick="${go(screen, { repo: d.repo })}"`}>${one.icon(icon)}<span>${title}</span>${extra}</a>`;
  return `<div class="repo-top">
    <div class="repo-path">${one.icon(isGithub(d.repo) ? "github-logo" : "book")}<span class="owner">${esc(owner)}</span><span class="sep">/</span><a onclick="${on === "code" ? "" : go("repo", { repo: d.repo })}">${esc(name)}</a></div>
    <nav class="tabs" aria-label="${esc(d.repo)}">
      ${tab("code", "repo", "code", "Code")}
      ${tab("issues", "issues", "record", "Issues", d.open_issues ? `<span class="counter">${d.open_issues.toLocaleString()}</span>` : "")}
      ${settings ? tab("settings", "repo_settings", "gear", "Settings") : ""}
    </nav>
  </div>`;
}
