// What every repo page shares: links, the path bar and the latest-commit card.
const P = one.params;
const esc = one.esc;
const short = (sha) => String(sha || "").slice(0, 7);
const wide = () => matchMedia("(min-width: 700px)").matches;
// A script that opens a screen, for an onclick="…".
const go = (screen, params) => esc(`one.open(${JSON.stringify(screen)}, ${JSON.stringify(params)})`);
// The ref to keep using: the branch if there is one, else the commit.
const refOf = (d) => d.branch || d.sha;
// Where git reaches a repo, on any computer in this install.
const address = (repo) => `${one.config?.api || "http://localhost"}/repos/${repo}.git`;
const plural = (n, one_, many) => `${n.toLocaleString()} ${n === 1 ? one_ : many}`;
const avatar = (c, cls = "") => `<span class="who-avatar ${cls}">${one.avatar(c.email || c.author, c.author)}</span>`;
one.haptics();

/// The branch chip and the path, each part tappable to go up; at an older commit, the chip is its hash.
function pathBar(d, { file = false } = {}) {
  const parts = (d.path || "").split("/").filter(Boolean);
  const ref = refOf(d);
  const chip = d.branch
    ? `<button class="chip" onclick="${go("branches", { repo: d.repo, path: d.path || "" })}">${one.icon("branch")}<span>${esc(d.branch)}</span>${one.icon("down", "chev")}</button>`
    : `<button class="chip" onclick="${go("branches", { repo: d.repo, path: d.path || "" })}">${one.icon("commit")}<span>${short(d.sha)}</span>${one.icon("down", "chev")}</button>` +
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
      <span class="latest-meta"><code>${short(c.sha)}</code> · ${one.ago(c.time, { long: true })}</span>
    </a>
    <a class="latest-history" onclick="${history}">${one.icon("history")}<span>History</span><span class="chev-row">${one.icon("chevron")}</span></a>
  </div>`;
}
