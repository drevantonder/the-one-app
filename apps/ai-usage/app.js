// AI Usage: every AI plan's limits, and whether the pace lasts to reset. Each machine's AI Usage service reads its
// providers' own numbers for every sign-in it keeps (Claude Code's and claude-swap's accounts, Codex's, OpenCode Go's);
// this asks every machine's and merges by account, keeps snapshots, and says in the providers' own words how much is
// left and what the pace does: deficit or reserve, what was expected, and whether it lasts.
// Agents read it through Agent Hooks; people get a card for each provider and the meters. Its screen is React, in
// screens/, built into dist/ when it's deployed (`one deploy apps/ai-usage`), which is committed because a release ships
// the app as it is.

import main from "./dist/main.html" with { type: "text" };

const FRESH = 5 * 60 * 1000;
const KEEP = 30 * 86400 * 1000;
const PRUNE = () => Date.now() - KEEP;

// Each brand's own mark, from its site (claude.ai, developers.openai.com, opencode.ai): a logo is inset on white,
// an icon is a finished tile drawn edge to edge.
const PROVIDERS = {
  claude: { title: "Claude", sessionMins: 300, weeklyMins: 10080, mark: "logo", svg: `<svg width="248" height="248" viewBox="0 0 248 248" fill="none" xmlns="http://www.w3.org/2000/svg"><path d="M52.4285 162.873L98.7844 136.879L99.5485 134.602L98.7844 133.334H96.4921L88.7237 132.862L62.2346 132.153L39.3113 131.207L17.0249 130.026L11.4214 128.844L6.2 121.873L6.7094 118.447L11.4214 115.257L18.171 115.847L33.0711 116.911L55.485 118.447L71.6586 119.392L95.728 121.873H99.5485L100.058 120.337L98.7844 119.392L97.7656 118.447L74.5877 102.732L49.4995 86.1905L36.3823 76.62L29.3779 71.7757L25.8121 67.2858L24.2839 57.3608L30.6515 50.2716L39.3113 50.8623L41.4763 51.4531L50.2636 58.1879L68.9842 72.7209L93.4357 90.6804L97.0015 93.6343L98.4374 92.6652L98.6571 91.9801L97.0015 89.2625L83.757 65.2772L69.621 40.8192L63.2534 30.6579L61.5978 24.632C60.9565 22.1032 60.579 20.0111 60.579 17.4246L67.8381 7.49965L71.9133 6.19995L81.7193 7.49965L85.7946 11.0443L91.9074 24.9865L101.714 46.8451L116.996 76.62L121.453 85.4816L123.873 93.6343L124.764 96.1155H126.292V94.6976L127.566 77.9197L129.858 57.3608L132.15 30.8942L132.915 23.4505L136.608 14.4708L143.994 9.62643L149.725 12.344L154.437 19.0788L153.8 23.4505L150.998 41.6463L145.522 70.1215L141.957 89.2625H143.994L146.414 86.7813L156.093 74.0206L172.266 53.698L179.398 45.6635L187.803 36.802L193.152 32.5484H203.34L210.726 43.6549L207.415 55.1159L196.972 68.3492L188.312 79.5739L175.896 96.2095L168.191 109.585L168.882 110.689L170.738 110.53L198.755 104.504L213.91 101.787L231.994 98.7149L240.144 102.496L241.036 106.395L237.852 114.311L218.495 119.037L195.826 123.645L162.07 131.592L161.696 131.893L162.137 132.547L177.36 133.925L183.855 134.279H199.774L229.447 136.524L237.215 141.605L241.8 147.867L241.036 152.711L229.065 158.737L213.019 154.956L175.45 145.977L162.587 142.787H160.805V143.85L171.502 154.366L191.242 172.089L215.82 195.011L217.094 200.682L213.91 205.172L210.599 204.699L188.949 188.394L180.544 181.069L161.696 165.118H160.422V166.772L164.752 173.152L187.803 207.771L188.949 218.405L187.294 221.832L181.308 223.959L174.813 222.777L161.187 203.754L147.305 182.486L136.098 163.345L134.745 164.2L128.075 235.42L125.019 239.082L117.887 241.8L111.902 237.31L108.718 229.984L111.902 215.452L115.722 196.547L118.779 181.541L121.58 162.873L123.291 156.636L123.14 156.219L121.773 156.449L107.699 175.752L86.304 204.699L69.3663 222.777L65.291 224.431L58.2867 220.768L58.9235 214.27L62.8713 208.48L86.304 178.705L100.44 160.155L109.551 149.507L109.462 147.967L108.959 147.924L46.6977 188.512L35.6182 189.93L30.7788 185.44L31.4156 178.115L33.7079 175.752L52.4285 162.873Z" fill="#D97757"/></svg>` },
  codex: { title: "Codex", sessionMins: 300, weeklyMins: 10080, mark: "logo", svg: `<svg width="24" height="24" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg"><circle cx="12" cy="12" r="12" fill="#0080F7"/><path d="M9.94494 9.59163V8.13227C9.94494 8.00935 9.99105 7.91713 10.0985 7.85575L13.0327 6.16599C13.4321 5.93558 13.9083 5.8281 14.3998 5.8281C16.2432 5.8281 17.4108 7.25677 17.4108 8.77751C17.4108 8.885 17.4108 9.00792 17.3953 9.13083L14.3537 7.34884C14.1694 7.24135 13.985 7.24135 13.8007 7.34884L9.94494 9.59163ZM16.7963 15.2755V11.7883C16.7963 11.5732 16.704 11.4196 16.5197 11.3121L12.664 9.0693L13.9236 8.34725C14.0311 8.28587 14.1234 8.28587 14.2308 8.34725L17.165 10.037C18.0099 10.5287 18.5782 11.5732 18.5782 12.587C18.5782 13.7544 17.887 14.8298 16.7963 15.2753V15.2755ZM9.03861 12.2031L7.77896 11.4658C7.67146 11.4045 7.62535 11.3122 7.62535 11.1893V7.8098C7.62535 6.16613 8.88501 4.92176 10.5902 4.92176C11.2354 4.92176 11.8344 5.13689 12.3415 5.52089L9.31526 7.27218C9.13097 7.37968 9.03875 7.53328 9.03875 7.74841V12.2033L9.03861 12.2031ZM11.75 13.77L9.94494 12.7562V10.6056L11.75 9.59178L13.5549 10.6056V12.7562L11.75 13.77ZM12.9098 18.44C12.2645 18.44 11.6655 18.2249 11.1585 17.8409L14.1847 16.0896C14.369 15.9821 14.4612 15.8285 14.4612 15.6134V11.1585L15.7363 11.8958C15.8438 11.9572 15.8899 12.0494 15.8899 12.1723V15.5519C15.8899 17.1955 14.6148 18.44 12.9098 18.44ZM9.26901 15.0144L6.33486 13.3246C5.4899 12.833 4.92161 11.7885 4.92161 10.7746C4.92161 9.59177 5.62824 8.53183 6.71886 8.0863V11.5887C6.71886 11.8039 6.81109 11.9575 6.99538 12.065L10.8359 14.2923L9.57621 15.0144C9.46872 15.0758 9.37649 15.0758 9.26901 15.0144ZM9.10013 17.5337C7.36426 17.5337 6.08919 16.2279 6.08919 14.6149C6.08919 14.492 6.1046 14.3691 6.11988 14.2462L9.1461 15.9975C9.33039 16.105 9.51483 16.105 9.69912 15.9975L13.5549 13.7702V15.2295C13.5549 15.3524 13.5088 15.4446 13.4013 15.506L10.4671 17.1958C10.0677 17.4262 9.59148 17.5337 9.09999 17.5337H9.10013ZM12.9098 19.3616C14.7685 19.3616 16.32 18.0406 16.6735 16.2893C18.3939 15.8438 19.5 14.2308 19.5 12.5872C19.5 11.5118 19.0391 10.4673 18.2096 9.71454C18.2864 9.39192 18.3326 9.0693 18.3326 8.74682C18.3326 6.55014 16.5505 4.90634 14.4921 4.90634C14.0774 4.90634 13.6779 4.96772 13.2785 5.10605C12.5872 4.43011 11.6347 4 10.5902 4C8.7314 4 7.17996 5.32103 6.8265 7.07232C5.10605 7.51786 4 9.13083 4 10.7745C4 11.8498 4.4608 12.8944 5.29035 13.6471C5.21354 13.9697 5.16743 14.2923 5.16743 14.6148C5.16743 16.8114 6.94941 18.4552 9.00792 18.4552C9.42261 18.4552 9.82204 18.3939 10.2215 18.2556C10.9127 18.9315 11.8651 19.3616 12.9098 19.3616Z" fill="white"/></svg>` },
  opencodego: { title: "OpenCode Go", sessionMins: 300, weeklyMins: 10080, mark: "icon", svg: `<svg width="512" height="512" viewBox="0 0 512 512" fill="none" xmlns="http://www.w3.org/2000/svg"><rect width="512" height="512" fill="#131010"></rect><path d="M320 224V352H192V224H320Z" fill="#5A5858"></path><path fill-rule="evenodd" clip-rule="evenodd" d="M384 416H128V96H384V416ZM320 160H192V352H320V160Z" fill="white"></path></svg>` },
};
const markOf = (id) => ({ image: "data:image/svg+xml," + encodeURIComponent(PROVIDERS[id].svg), kind: PROVIDERS[id].mark });

const int = (v) => (v == null || Number.isNaN(Number(v)) ? null : Math.round(Number(v)));
const ago = (at, now) => {
  const m = Math.max(0, Math.round((now - at) / 60000));
  if (m < 1) return "just now";
  if (m < 60) return `${m} min ago`;
  const h = Math.round(m / 60);
  return h < 24 ? `${h} h ago` : `${Math.round(h / 24)} d ago`;
};
const clock = (t) => new Date(t).toLocaleTimeString("en-US", { hour: "numeric", minute: "2-digit" });
const day = (t) => new Date(t).toLocaleDateString("en-US", { weekday: "short" });
const monthDay = (t) => new Date(t).toLocaleDateString("en-US", { month: "short", day: "numeric" });

function resets(t, now) {
  if (t == null) return "reset unknown";
  const d = t - now;
  if (d <= 0) return "resetting now";
  if (d < 2 * 3600000) {
    const m = Math.round(d / 60000);
    return m <= 1 ? "in 1 min" : `in ${m} min`;
  }
  if (d < 86400000) return `today at ${clock(t)}`;
  if (d < 7 * 86400000) return `${day(t)} at ${clock(t)}`;
  return `${monthDay(t)} at ${clock(t)}`;
}

function countdown(t, now) {
  if (t == null) return "";
  const m = Math.max(0, Math.round((t - now) / 60000));
  if (m < 1) return "now";
  if (m < 120) return `in ${m} min`;
  const h = Math.floor(m / 60);
  if (h < 48) return `in ${h}h ${m % 60}m`;
  const d = Math.floor(h / 24);
  return d < 14 ? `in ${d}d ${h % 24}h` : `on ${monthDay(t)}`;
}

function paceFrom(used, resetsAt, windowMins, prev, now) {
  if (resetsAt == null || used == null) return null;
  const start = resetsAt - windowMins * 60000;
  const expected = Math.max(0, Math.min(100, ((now - start) / (windowMins * 60000)) * 100));
  const delta = Math.round((used - expected) * 10) / 10;
  const exp = Math.round(expected);
  if (used >= 100) return { summary: `Out now | Expected ${exp}% used | Back at reset`, willLast: false, delta };
  if (Math.abs(delta) < 3) return { summary: `On pace | Expected ${exp}% used | Lasts until reset`, willLast: true, delta };
  let tail = "Lasts until reset";
  let willLast = delta <= 0 ? true : null;
  const separated = prev && prev.at > start && prev.used != null && now - prev.at > 10 * 60000;
  if (delta > 0) tail = separated ? tail : "Ahead of even pace";
  if (separated) {
    const burn = (used - prev.used) / (now - prev.at);
    if (burn > 0) {
      const eta = (100 - used) / burn;
      willLast = now + eta >= resetsAt;
      tail = willLast ? "Lasts until reset" : emptyIn(eta);
    }
  }
  const head = delta > 0 ? `${trimNum(delta)}% in deficit` : `${trimNum(-delta)}% in reserve`;
  return { summary: `${head} | Expected ${exp}% used | ${tail}`, willLast, delta };
}
const trimNum = (v) => String(Math.round(v * 10) / 10);
const emptyIn = (eta) => {
  const mins = Math.max(0, Math.round(eta / 60000));
  return mins < 120 ? `Projected empty in ${mins} min` : mins < 2880 ? `Projected empty in ${Math.floor(mins / 60)}h ${mins % 60}m` : `Runs out in ${Math.round(mins / 1440)}d`;
};

function accountName(a) {
  return a.account || a.email || null;
}

function steer(models) {
  const used = {};
  for (const m of models || []) {
    const n = String(m.name || "").toLowerCase();
    if (n.includes("opus")) used.opus = m.used;
    else if (n.includes("sonnet")) used.sonnet = m.used;
  }
  const bits = [];
  if (used.opus != null) bits.push(`Opus ${used.opus}% used`);
  if (used.sonnet != null) bits.push(`Sonnet ${used.sonnet}% used`);
  const head = bits.length ? bits.join(" · ") + " — " : "";
  if (used.opus == null && used.sonnet == null) return "";
  if ((used.opus ?? 0) >= 85 && (used.sonnet ?? 0) >= 85) return `${head}Haiku edits, Sonnet thinking, no Opus`;
  if ((used.opus ?? 0) >= 85) return `${head}Sonnet or Haiku; Opus review-only`;
  if ((used.sonnet ?? 0) >= 85) return `${head}Opus or Haiku; spare Sonnet`;
  const spare = (models || []).filter((m) => (m.used ?? 0) >= 85 && !/opus|sonnet/i.test(String(m.name || ""))).map((m) => m.name);
  if (spare.length) return `${head}spare ${spare.join(", ")}; spend the cooler pool`;
  return `${head}any model fine`;
}

async function latest(b, provider, before = null) {
  const rows = await b.query(
    "select * from snapshots where provider = ? and (at <= ? or ? is null) and at = (select max(at) from snapshots x where x.provider = snapshots.provider and ifnull(x.account, '') = ifnull(snapshots.account, '') and (x.at <= ? or ? is null)) order by active desc, weekly_used",
    [provider, before, before, before, before],
  );
  return rows.map((r) => ({ ...r, devices: JSON.parse(r.devices || "null") || [r.device], active: !!r.active, disabled: !!r.disabled, models: JSON.parse(r.models || "[]"), pace: JSON.parse(r.pace || "{}") }));
}

async function previous(b, provider, account, at, resetsAt) {
  const rows = await b.query("select at, session_used, weekly_used, session_resets from snapshots where provider = ? and ifnull(account, '') is ifnull(?, '') and at < ? order by at desc limit 5", [provider, account, at]);
  return rows.map((r) => ({ at: r.at, used: r.session_used, resets: r.session_resets })).find((r) => r.resets === resetsAt) || null;
}

function fromService(a) {
  const win = (w) => ({ used: int(w?.used), resetsAt: w?.resetsAt ?? null, resetText: null });
  return {
    source: "service", confidence: "quota",
    session: win(a.session), weekly: win(a.weekly), monthly: win(a.monthly),
    models: (a.models || []).map((m) => ({ name: String(m.name || "pool"), used: int(m.used), resetsAt: m.resetsAt ?? null })),
    plan: a.plan || null, account: accountName(a), email: a.email || null,
    active: !!a.active, disabled: !!a.disabled,
    notes: a.error ? [String(a.error)] : [],
    extra: a.extra?.limit != null ? a.extra : null,
  };
}

/// The newest read per account wins, on whatever machine saw it. A machine that couldn't answer is a problem only on
/// the reads it gave before, which stay shown with their age and why. `ask` is the one read of every machine this
/// status shares: `{ answers }`, or `{ problem }` when none could be asked.
async function refresh(b, id, ask, now) {
  const P = PROVIDERS[id];
  b.exec("delete from snapshots where session_used is null and weekly_used is null and monthly_used is null");
  const snap = await latest(b, id);
  const newest = snap.reduce((t, r) => Math.max(t, r.at), 0);
  if (snap.length && now - newest < FRESH) return snap.map((r) => ({ ...r, cached: true, now }));
  const { answers, problem } = await ask();
  if (problem) return snap.map((r) => ({ ...r, cached: true, stale: problem, now }));
  const failed = {}; // machine → why it couldn't answer
  const unread = {}; // account → why it couldn't be read
  const reads = [];
  const has = {}; // account → every machine that keeps its sign-in
  for (const [machine, answer] of Object.entries(answers || {})) {
    if (answer?.error) {
      failed[machine] = `${machine}: ${String(answer.error).slice(0, 160)}`;
      continue;
    }
    for (const a of answer?.accounts || []) {
      if (a.provider !== id) continue;
      const out = fromService(a);
      (has[out.account || out.email || ""] ??= []).push(machine);
      if (out.session.used == null && out.weekly.used == null && out.monthly.used == null) {
        if (a.error) unread[out.account || ""] ??= `${machine}: ${String(a.error).slice(0, 160)}`;
        continue;
      }
      reads.push({ machine, out });
    }
  }
  const results = [];
  const seen = new Set();
  for (const { machine, out } of reads) {
    const key = out.account || out.email || "";
    if (seen.has(key)) continue;
    seen.add(key);
    const prev = await previous(b, id, out.account || out.email, now, out.session.resetsAt);
    out.session.pace = paceFrom(out.session.used, out.session.resetsAt, P.sessionMins, prev && { at: prev.at, used: prev.used }, now);
    out.weekly.pace = paceFrom(out.weekly.used, out.weekly.resetsAt, P.weeklyMins, null, now);
    const active = out.active || reads.length === 1 ? 1 : 0;
    const devices = [...new Set(has[key])].sort();
    b.exec("insert into snapshots (provider, device, devices, at, source, confidence, session_used, session_resets, weekly_used, weekly_resets, monthly_used, monthly_resets, models, pace, note, account, active, disabled) values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", [id, machine, JSON.stringify(devices), now, out.source, out.confidence, out.session.used, out.session.resetsAt, out.weekly.used, out.weekly.resetsAt, out.monthly.used, out.monthly.resetsAt, JSON.stringify(out.models), JSON.stringify({ session: out.session.pace, weekly: out.weekly.pace }), (out.notes || [])[0] || null, out.account || null, active, out.disabled ? 1 : 0]);
    results.push({ provider: id, device: machine, devices, at: now, source: out.source, confidence: out.confidence, session_used: out.session.used, session_resets: out.session.resetsAt, session_resetText: out.session.resetText, weekly_used: out.weekly.used, weekly_resets: out.weekly.resetsAt, weekly_resetText: out.weekly.resetText, monthly_used: out.monthly.used, monthly_resets: out.monthly.resetsAt, models: out.models, pace: { session: out.session.pace, weekly: out.weekly.pace }, plan: out.plan, account: out.account || null, active: !!active, disabled: !!out.disabled, extra: out.extra, notes: out.notes || [], cached: false, now });
  }
  b.exec("delete from snapshots where at < ?", [PRUNE()]);
  const gone = (r) => r.device in (answers || {}) && !failed[r.device] && !unread[r.account || ""];
  for (const r of snap) if (!seen.has(r.account || "") && gone(r)) b.exec("delete from snapshots where provider = ? and ifnull(account, '') = ?", [id, r.account || ""]);
  // The writes land when this call returns, so the merge happens here.
  const kept = snap.filter((r) => !seen.has(r.account || "") && !gone(r)).map((r) => {
    const why = unread[r.account || ""] || failed[r.device];
    return { ...r, cached: true, now, ...(why ? { stale: why } : {}) };
  });
  return [...results, ...kept].sort((x, y) => (y.active - x.active) || ((x.weekly_used ?? 0) - (y.weekly_used ?? 0)));
}

function paceNews(pace) {
  if (!pace?.summary) return null;
  const [head, , tail] = pace.summary.split(" | ");
  if (pace.willLast !== false && !/deficit/.test(head)) return null;
  return [head.toLowerCase(), /Projected|Runs out/.test(tail || "") ? tail.replace(/^./, (c) => c.toLowerCase()) : null].filter(Boolean).join(", ");
}

/// One line per window: "Weekly 35% left, 37% in deficit, runs out in 1d 1h, resets Sun at 11:59 PM".
function windowText(label, w, now) {
  if (w.used == null) return null;
  const reset = w.resetsAt != null ? `resets ${resets(w.resetsAt, now)}` : w.resetText ? w.resetText.replace(/^./, (c) => c.toLowerCase()) : null;
  return [`${label} ${100 - w.used}% left`, paceNews(w.pace), reset].filter(Boolean).join(", ");
}

function alarm(p, now) {
  const wins = [["Session", p.session_used, p.session_resets, p.pace?.session], ["Weekly", p.weekly_used, p.weekly_resets, p.pace?.weekly], ["Monthly", p.monthly_used, p.monthly_resets, null]];
  for (const [label, used, at, pace] of wins) if (used >= 100) return { tone: "danger", text: `Out until ${resets(at, now)}` };
  for (const [label, used, at, pace] of wins) {
    if (pace?.willLast !== false) continue;
    const when = String(pace.summary || "").match(/(?:Projected empty|Runs out) in (.+)$/);
    return { tone: "warn", text: when ? `${label} runs out in ${when[1]}, before it resets` : `${label} runs out before it resets` };
  }
  return null;
}

function section(reads, now) {
  const p = reads[0];
  const title = PROVIDERS[p.provider].title;
  const head = [title, p.account ? `${p.account}${reads.length > 1 ? ", in use" : ""}` : null, p.plan, p.devices?.length ? `on ${p.devices.join(", ")}` : null].filter(Boolean).join(" · ");
  const age = now - p.at > FRESH ? `read ${ago(p.at, now)}` : null;
  const problems = [...new Set(reads.map((r) => r.stale).filter(Boolean))];
  const steered = steer(p.models);
  const others = reads.slice(1).map((o) => `${o.account} ${o.weekly_used >= 100 ? `out, back ${resets(o.weekly_resets, now)}` : `${100 - (o.weekly_used ?? 0)}% weekly left`}${o.disabled ? ", not in rotation" : ""}${o.stale ? `, read ${ago(o.at, now)}` : ""}`);
  return [
    head,
    windowText("Session", { used: p.session_used, resetsAt: p.session_resets, resetText: p.session_resetText, pace: p.pace?.session }, now),
    windowText("Weekly", { used: p.weekly_used, resetsAt: p.weekly_resets, resetText: p.weekly_resetText, pace: p.pace?.weekly }, now),
    p.monthly_used != null ? windowText("Monthly", { used: p.monthly_used, resetsAt: p.monthly_resets }, now) : null,
    p.extra ? `Extra usage $${p.extra.used} of $${p.extra.limit} this month` : null,
    steered && !/any model fine$/.test(steered) ? steered : null,
    ...(p.notes || []).map((n) => `Note: ${n}`),
    others.length ? `Other accounts: ${others.join("; ")}` : null,
    age,
    problems.length ? `Couldn't read it now: ${problems.join("; ")}` : null,
  ].filter(Boolean).join("\n");
}

function tripped(p) {
  const left = (u) => (u == null ? null : 100 - u);
  const sLeft = left(p.session_used), wLeft = left(p.weekly_used);
  const out = sLeft != null && (sLeft < 20 || p.pace?.session?.willLast === false);
  const outW = wLeft != null && (wLeft < 15 || p.pace?.weekly?.willLast === false);
  return out || outW ? { out, outW } : null;
}

/// Which plans have run nearly out (under 20% session, 15% weekly) or begun to outrun their reset since `since`, on
/// every Claude account at once: their rows and which pool to spend, else nothing. Each is said once, when it starts.
async function warnings(b, now, since) {
  // With several accounts, claude-swap moves to one with room: only all of them nearly out is news.
  const low = (reads) => (reads.length && !reads.some((r) => !tripped(r)) ? JSON.stringify(tripped(reads[0])) : null);
  const out = [];
  for (const id of Object.keys(PROVIDERS)) {
    const reads = await latest(b, id);
    const was = low(await latest(b, id, since));
    if (!low(reads) || low(reads) === was) continue;
    const p = reads[0];
    const t = tripped(p);
    const rows = [];
    if (t.out) rows.push(windowText("Session", { used: p.session_used, resetsAt: p.session_resets, pace: p.pace?.session }, now));
    if (t.outW) rows.push(windowText("Weekly", { used: p.weekly_used, resetsAt: p.weekly_resets, pace: p.pace?.weekly }, now));
    rows.push(steer(p.models) || null);
    out.push([`== ${PROVIDERS[id].title}${reads.length > 1 ? `: all ${reads.length} accounts nearly out` : ""} ==`, ...rows.filter(Boolean)].join("\n"));
  }
  return out.join("\n\n");
}

export default {
  name: "ai-usage",
  title: "AI Usage",
  tagline: "AI plan limits",
  icon: "gauge",
  color: "#636366",
  advanced: true,
  description: "Every AI plan's limits, and whether the pace lasts to reset",
  instructions: `Every AI plan's limits in the providers' own words: what's left, the pace against what's expected, and whether it lasts to reset. ai-usage_status is the full read (fresh when older than 5 minutes), merged from the AI Usage service on every machine that has one. Agents get the last readings with their first prompt, then a warning when a window is nearly out or outruns its reset.`,

  uses: { services: ["ai-usage"] },
  service: { network: ["api.anthropic.com", "api.openai.com", "chatgpt.com", "opencode.ai"], person: true, platforms: ["linux", "mac"] },

  // Fresh readings for agents' prompts, from the device that runs AI Usage's schedules (one devices run_schedules ai-usage <it>).
  schedules: { refresh: { tool: "status", every: "5m", description: "Reads every plan, so agents' prompts get fresh numbers" } },

  schema: Object.fromEntries(["user", "group", "global"].map((kind) => [kind, [`
    create table snapshots (provider text not null, device text not null, at integer not null, source text not null, confidence text not null, session_used real, session_resets integer, weekly_used real, weekly_resets integer, monthly_used real, monthly_resets integer, models text, pace text);
    create index snapshots_provider on snapshots (provider, device, at);`,
    `alter table snapshots add column note text;`,
    `alter table snapshots add column account text;`,
    `alter table snapshots add column active integer;`,
    `alter table snapshots add column disabled integer;`,
    // An email column a test deploy once added, unused: kept so every database has the same changes in the same order.
    `alter table snapshots add column email text;`,
    `alter table snapshots add column devices text;`]])),

  tools: {
    status: {
      words: ["provider", "format"],
      reads: false,
      description: "Every AI plan's limits (or one provider's), each Claude account apart: what's left, the pace, and whether it lasts to reset, in the providers' own words, plus which pool to spend. Fresh when older than 5 minutes, else the last read with its age.",
      input: {
        type: "object",
        properties: {
          provider: { type: "string", enum: ["all", "claude", "codex", "opencodego"], description: "all (the default) or one" },
          format: { type: "string", enum: ["text", "json"], description: "text (the default) to read, json for screens" },
          device: { type: "string", description: "one machine to read, by name; omit for every machine's" },
        },
      },
      async run(ctx, { provider = "all", format = "text", device } = {}) {
        const b = ctx.user;
        if (!b) throw new Error("your own data is on one of your devices: run it there");
        const now = ctx.now();
        const ids = provider === "all" ? Object.keys(PROVIDERS) : [provider];
        let asked = null;
        const ask = () => (asked ??= ctx.services["ai-usage"].usage({}, { device: device || "all" }).then((answers) => ({ answers }), (err) => ({ problem: String(err?.message || err).slice(0, 160) })));
        const all = (await Promise.all(ids.map((id) => refresh(b, id, ask, now)))).flat();
        // Nothing to show because no machine could be asked is the one problem; nothing because none has a sign-in, none.
        const problem = !all.length && asked ? (await asked).problem || null : null;
        if (format === "json") {
          return {
            at: now, problem,
            marks: Object.fromEntries(ids.map((id) => [id, markOf(id)])),
            providers: all.map((p) => ({
              id: p.provider, title: PROVIDERS[p.provider].title, account: p.account || null, active: !!p.active, disabled: !!p.disabled, alarm: alarm(p, now), plan: p.plan || null, source: p.source || null,
              devices: p.devices || [], updatedAgo: p.at ? ago(p.at, now) : null,
              stale: p.stale || null,
              session: { used: p.session_used, left: p.session_used == null ? null : 100 - p.session_used, resets: p.session_resets ? resets(p.session_resets, now) : null, countdown: p.session_resets ? countdown(p.session_resets, now) : null, pace: p.pace?.session?.summary || null, willLast: p.pace?.session?.willLast ?? null },
              weekly: { used: p.weekly_used, left: p.weekly_used == null ? null : 100 - p.weekly_used, resets: p.weekly_resets ? resets(p.weekly_resets, now) : null, countdown: p.weekly_resets ? countdown(p.weekly_resets, now) : null, pace: p.pace?.weekly?.summary || null, willLast: p.pace?.weekly?.willLast ?? null },
              monthly: p.monthly_used == null ? null : { used: p.monthly_used, left: 100 - p.monthly_used, resets: p.monthly_resets ? resets(p.monthly_resets, now) : null, countdown: p.monthly_resets ? countdown(p.monthly_resets, now) : null },
              models: (p.models || []).map((m) => ({ name: m.name, used: m.used, left: m.used == null ? null : 100 - m.used, countdown: m.resetsAt ? countdown(m.resetsAt, now) : null })),
              extra: p.extra || null,
              steer: steer(p.models),
            })),
          };
        }
        if (problem) return `Couldn't read your machines' AI plans: ${problem}`;
        if (!all.length) return "No AI sign-ins on your machines.";
        return ids.map((id) => all.filter((p) => p.provider === id)).filter((r) => r.length).map((r) => section(r, now)).join("\n\n");
      },
    },
  },

  hooks: {
    // With an agent's prompt anywhere: every plan's last reading on its session's first, then a warning when one runs low.
    async prompt(ctx, { since }) {
      const b = ctx.user;
      if (!b) return "";
      const now = ctx.now();
      if (since !== null) return warnings(b, now, since);
      const reads = (await Promise.all(Object.keys(PROVIDERS).map((id) => latest(b, id)))).filter((r) => r.length);
      return reads.length ? ["AI usage, for choosing a model:", ...reads.map((r) => section(r, now))].join("\n\n") : "";
    },
  },

  screens: {
    main: { title: "AI Usage", html: main },
  },

  widgets: {
    usage: {
      title: "AI Usage",
      description: "Every AI plan's limits, metered",
      size: "medium",
      sizes: ["small", "medium", "large"],
      async render(ctx) {
        const b = ctx.user;
        if (!b) return { t: "text", text: "No plans yet.", size: "sm", tone: "muted" };
        const now = ctx.now();
        const plans = [];
        for (const id of Object.keys(PROVIDERS)) {
          const reads = await latest(b, id);
          for (const p of reads) {
            if (p.session_used == null && p.weekly_used == null) continue;
            const account = reads.length > 1 && p.account ? p.account.split(" · ")[0] : "";
            plans.push({ name: PROVIDERS[id].title, mark: markOf(id).image, account, session: p.session_used, week: p.weekly_used, resets: p.session_resets ?? p.weekly_resets });
          }
        }
        if (!plans.length) return { t: "text", text: "No readings yet.", size: "sm", tone: "muted" };
        const worst = (p) => Math.max(p.session ?? 0, p.week ?? 0);
        const fit = { small: 3, medium: 4, large: 8 }[ctx.size] || 4;
        const top = new Set([...plans].sort((a, b) => worst(b) - worst(a)).slice(0, fit));
        const show = plans.filter((p) => top.has(p));
        const level = (u) => (u >= 85 ? "bad" : u >= 70 ? "warn" : "good");
        const cell = (w, n) => ({ t: "col", w, align: "center", c: [n || { t: "col", c: [] }] });
        const label = (text) => ({ t: "text", text, size: "xs", weight: "medium", tone: "muted", lines: 1 });
        const ring = (u, w, alt) => (u == null ? null : { t: "gauge", value: u / 100, tone: level(u), w, label: "", alt: `${alt} ${u}% used` });
        const small = ctx.size === "small", w = small ? 22 : 44, r = small ? 14 : 20;
        const header = { t: "row", gap: small ? 4 : 6, c: [{ t: "col", grow: true, c: [label("Plans")] }, cell(w, label(small ? "5H" : "SESSION")), cell(w, label(small ? "7D" : "WEEK"))] };
        const row = (p) => {
          const name = [p.name, p.account].filter(Boolean).join(" · ");
          const sub = ctx.size === "large" && p.resets != null ? `Resets ${countdown(p.resets, now)}` : null;
          return {
            t: "row", gap: small ? 4 : 6,
            c: [
              { t: "image", url: p.mark, w: small ? 16 : 20, h: small ? 16 : 20 },
              { t: "col", grow: true, gap: 1, c: [{ t: "text", text: small ? p.account.split(" ")[0] || p.name : name, size: "sm", lines: 1 }, ...(sub ? [{ t: "text", text: sub, size: "xs", tone: "muted", lines: 1 }] : [])] },
              cell(w, ring(p.session, r, `${name} session`)),
              cell(w, ring(p.week, r, `${name} week`)),
            ],
          };
        };
        const body = [header, ...show.map(row)];
        const out = ctx.size === "large" && show.filter((p) => worst(p) >= 85).sort((a, b) => worst(b) - worst(a))[0];
        if (out) body.push({ t: "row", gap: 6, c: [{ t: "icon", name: "warning-fill", tone: "bad", size: "xs" }, { t: "text", text: `${[out.name, out.account].filter(Boolean).join(" · ")} is at ${worst(out)}%${out.resets != null ? `, resets ${countdown(out.resets, now)}` : ""}`, size: "xs", tone: "bad", lines: 1 }] });
        const c = body.length < 4 ? [{ t: "col", gap: 8, c: body }, { t: "col", grow: true, gap: 0, c: [] }] : body.flatMap((n, i) => (i ? [{ t: "col", grow: true, gap: 0, c: [] }, n] : [n]));
        return { t: "col", header: false, next: Date.now() + 300_000, c };
      },
    },
  },
};
