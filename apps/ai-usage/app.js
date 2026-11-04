// AI Usage: every AI plan's limits, and whether the pace lasts to reset. Reads the providers' own numbers
// (every Claude account in claude-swap where it's installed, the codexbar CLI, Claude's own endpoint else), keeps snapshots, and says in the providers'
// own words how much is left and what the pace does: deficit or reserve, what was expected, and whether it lasts.
// Agents read it through Agent Hooks; people get the tiles and the meter.

const FRESH = 5 * 60 * 1000; // a snapshot counts as now for this long
const KEEP = 30 * 86400 * 1000; // snapshots kept this long
const PRUNE = () => Date.now() - KEEP;

const PROVIDERS = {
  claude: { title: "Claude", codexbar: "claude", sessionMins: 300, weeklyMins: 10080 },
  codex: { title: "Codex", codexbar: "codex", sessionMins: 300, weeklyMins: 10080 },
  opencodego: { title: "OpenCode Go", codexbar: "opencodego", sessionMins: 300, weeklyMins: 10080 },
};

const int = (v) => (v == null || Number.isNaN(Number(v)) ? null : Math.round(Number(v)));
const ms = (v) => {
  if (v == null) return null;
  if (typeof v === "number") return v < 1e12 ? v * 1000 : v;
  const t = Date.parse(v);
  return Number.isNaN(t) ? null : t;
};
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

/// "today at 2:15 PM", "Fri at 9:00 AM": the CLI's absolute voice.
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

/// "in 50 min", "in 3h 53m", "in 3d 20h": the menu's countdown voice.
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

/// Which device answers: the one given, else the one running this.
async function here(ctx, device) {
  if (device) return device;
  try {
    return (await ctx.devices()).find((s) => s.me)?.name || null;
  } catch {
    return null;
  }
}

function side(ctx, input, device) {
  return device ? ctx.sidecars.shell.run(input, { device }) : ctx.sidecars.shell.run(input);
}

/// A pace in the providers' own words, from two snapshots sharing a window reset.
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
    const burn = (used - prev.used) / (now - prev.at); // % per ms
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

/// A pace from claude-swap's own expected and projected numbers, in the same words.
function paceOfCswap(w, now) {
  if (w?.pct == null || w.expectedPct == null) return null;
  const used = w.pct, exp = Math.round(w.expectedPct);
  const delta = Math.round((used - w.expectedPct) * 10) / 10;
  if (used >= 100) return { summary: `Out now | Expected ${exp}% used | Back at reset`, willLast: false, delta };
  if (Math.abs(delta) < 3) return { summary: `On pace | Expected ${exp}% used | Lasts until reset`, willLast: true, delta };
  const willLast = w.willLastToReset !== false;
  const empty = ms(w.projectedExhaustionAt);
  const tail = willLast ? "Lasts until reset" : empty != null ? emptyIn(empty - now) : "Ahead of even pace";
  const head = delta > 0 ? `${trimNum(delta)}% in deficit` : `${trimNum(-delta)}% in reserve`;
  return { summary: `${head} | Expected ${exp}% used | ${tail}`, willLast, delta };
}

/// Who an account is, as a person tells them apart: its alias, else its team and email.
function accountName(a) {
  if (a.alias) return String(a.alias);
  const org = String(a.organizationName || "");
  return !org || /'s Organization$/.test(org) ? String(a.email || "") : `${org} · ${a.email || ""}`;
}

/// Which pool to spend: the cooler weekly one. Names come from the scoped rows.
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

/// The newest read of a provider on this device: one snapshot per account.
async function latest(b, provider, device) {
  const top = await b.query("select at from snapshots where provider = ? and (device = ? or ? is null) order by at desc limit 1", [provider, device, device]);
  if (!top[0]) return [];
  const rows = await b.query("select * from snapshots where provider = ? and (device = ? or ? is null) and at = ? order by active desc, weekly_used", [provider, device, device, top[0].at]);
  return rows.map((r) => ({ ...r, active: !!r.active, models: JSON.parse(r.models || "[]"), pace: JSON.parse(r.pace || "{}") }));
}

/// The one before for the same account, sharing the session window, for the burn rate.
async function previous(b, provider, device, account, at, resetsAt) {
  const rows = await b.query("select at, session_used, weekly_used, session_resets from snapshots where provider = ? and (device = ? or ? is null) and account is ? and at < ? order by at desc limit 5", [provider, device, device, account, at]);
  return rows.map((r) => ({ at: r.at, used: r.session_used, resets: r.session_resets })).find((r) => r.resets === resetsAt) || null;
}

/// codexbar's JSON into our windows. Pace summaries are repeated word for word.
function fromCodexbar(j, now) {
  const u = j.usage || {};
  const win = (w) => (w ? { used: int(w.usedPercent), resetsAt: ms(w.resetsAt), resetText: w.resetDescription || null } : { used: null, resetsAt: null, resetText: null });
  const pw = (w, mins) => {
    const p = (j.pace || {})[w];
    if (!p || !p.summary) return null;
    return { summary: String(p.summary), willLast: p.willLastToReset !== false, delta: p.deltaPercent ?? null };
  };
  const models = [];
  const hunt = (o, depth) => {
    if (!o || depth > 2) return;
    for (const [k, v] of Object.entries(o)) {
      if (v && typeof v === "object" && !Array.isArray(v)) {
        if (/sonnet|opus/i.test(k) && v.usedPercent != null) models.push({ name: /opus/i.test(k) ? "Opus" : "Sonnet", used: int(v.usedPercent), resetsAt: ms(v.resetsAt) });
        else hunt(v, depth + 1);
      }
    }
  };
  hunt(u, 0);
  for (const d of u.details || []) for (const r of d.rows || []) {
    const m = String(r.value || "").match(/(\d+)% left/);
    if (m && /sonnet|opus/i.test(`${r.label} ${d.title}`)) models.push({ name: /opus/i.test(`${r.label} ${d.title}`) ? "Opus" : "Sonnet", used: 100 - Number(m[1]), resetsAt: null });
  }
  for (const e of u.extraRateWindows || []) {
    const w = e.window || {};
    if (w.usedPercent != null) {
      const title = String(e.title || e.id || "pool");
      models.push({ name: /opus/i.test(title) ? "Opus" : /sonnet/i.test(title) ? "Sonnet" : title.replace(/ only$/i, ""), used: int(w.usedPercent), resetsAt: ms(w.resetsAt) });
    }
  }
  const account = j.accountEmail || u.identity?.accountEmail || null;
  const plan = u.identity?.loginMethod ? String(u.identity.loginMethod).replace(/^./, (c) => c.toUpperCase()) : null;
  const notes = [];
  for (const d of u.details || []) for (const r of d.rows || []) if (r.value && /status|note/i.test(String(r.label || ""))) notes.push(String(r.value));
  return {
    source: `codexbar (${j.source || "auto"})`,
    confidence: u.dataConfidence === "estimated" ? "estimated" : "quota",
    session: { ...win(u.primary), pace: u.dataConfidence === "estimated" ? null : pw("primary") },
    weekly: { ...win(u.secondary), pace: u.dataConfidence === "estimated" ? null : pw("secondary") },
    monthly: { ...win(u.tertiary), pace: null },
    models, plan, account, notes,
    extra: null,
  };
}

/// claude-swap's accounts into our windows, one read per account. It keeps no secrets in this output.
function fromCswap(j) {
  const win = (w) => ({ used: int(w?.pct), resetsAt: ms(w?.resetsAt), resetText: null, pace: null, raw: w });
  return (j.accounts || []).map((a) => {
    const u = a.usage || {};
    const out = {
      source: "cswap", confidence: "quota",
      session: win(u.fiveHour), weekly: win(u.sevenDay), monthly: { used: null, resetsAt: null },
      models: (u.scoped || []).map((m) => ({ name: String(m.name || "pool"), used: int(m.pct), resetsAt: ms(m.resetsAt) })),
      plan: null, account: accountName(a), active: !!a.active,
      notes: a.usageStatus && a.usageStatus !== "ok" ? [`usage ${a.usageStatus}`] : [], extra: null,
    };
    return out;
  });
}

/// Claude straight from its own endpoint. The token is never logged or kept.
async function fromClaude(ctx, b, device, now) {
  let creds;
  try {
    const r = await side(ctx, { command: "cat ~/.claude/.credentials.json", timeout: 10 }, device);
    if (r?.exit !== 0) throw new Error("no credentials file");
    creds = JSON.parse(String(r.stdout || ""));
  } catch {
    throw new Error("Claude Code isn't signed in on " + (device || "this device"));
  }
  const token = creds?.claudeAiOauth?.accessToken;
  if (!token) throw new Error("no OAuth token in Claude Code's credentials");
  const r = await fetch("https://api.anthropic.com/api/oauth/usage", { headers: { Authorization: "Bearer " + token, "anthropic-beta": "oauth-2025-04-20" }, signal: AbortSignal.timeout(20000) });
  if (r.status === 429) throw new Error("rate limited");
  if (!r.ok) throw new Error(`usage said ${r.status}`);
  const j = await r.json();
  const win = (w) => ({ used: int(w?.utilization), resetsAt: ms(w?.resets_at) });
  const prev = null;
  const out = {
    source: "oauth",
    confidence: "quota",
    session: win(j.five_hour),
    weekly: win(j.seven_day),
    monthly: { used: null, resetsAt: null },
    models: [
      ...(j.seven_day_sonnet ? [{ name: "Sonnet", used: int(j.seven_day_sonnet.utilization), resetsAt: ms(j.seven_day_sonnet.resets_at) }] : []),
      ...(j.seven_day_opus ? [{ name: "Opus", used: int(j.seven_day_opus.utilization), resetsAt: ms(j.seven_day_opus.resets_at) }] : []),
    ],
    plan: null, account: null,
    extra: j.extra_usage ? { used: j.extra_usage.used_credits, limit: j.extra_usage.monthly_limit } : null,
  };
  return out;
}

/// Fresh numbers, one per account, or the last ones with their age when fetching fails. Estimated rows never forecast.
async function refresh(ctx, b, id, device, now) {
  const P = PROVIDERS[id];
  b.exec("delete from snapshots where session_used is null and weekly_used is null and monthly_used is null");
  const snap = await latest(b, id, device);
  if (snap.length && now - snap[0].at < FRESH) return snap.map((r) => ({ ...r, cached: true, now }));
  const fail = (problem) => (snap.length ? snap.map((r) => ({ ...r, cached: true, stale: problem, now })) : [{ provider: id, error: problem, now }]);
  const probe = (n, args, err) => `for b in ${n} "$HOME/.local/bin/${n}" /opt/homebrew/bin/${n}; do out=$("$b" ${args} ${err}); if [ -n "$out" ]; then echo "$out"; break; fi; done`;
  try {
    let outs = null;
    if (id === "claude") {
      try {
        const r = await side(ctx, { command: probe("cswap", "list --json", "2>/dev/null"), timeout: 60 }, device);
        outs = fromCswap(JSON.parse(String(r?.stdout || ""))).filter((o) => o.session.used != null || o.weekly.used != null);
        if (!outs.length) outs = null;
      } catch {
        outs = null;
      }
    }
    if (!outs) {
      try {
        const r = await side(ctx, { command: probe("codexbar", `usage --provider ${P.codexbar} --format json --no-color`, "2>&1"), timeout: 25 }, device);
        const parsed = JSON.parse(String(r?.stdout || ""));
        const first = Array.isArray(parsed) ? parsed[0] : parsed;
        if (!first || first.error) throw new Error(first?.error?.message || "no codexbar");
        const out = fromCodexbar(first, now);
        if (out.session.used == null && out.weekly.used == null && out.monthly.used == null) throw new Error("no usable windows");
        outs = [out];
      } catch (e) {
        if (id !== "claude") throw e;
        outs = [await fromClaude(ctx, b, device, now)];
      }
    }
    const results = [];
    for (const out of outs) {
      if (out.confidence !== "estimated") {
        if (!out.session.pace) out.session.pace = paceOfCswap(out.session.raw, now);
        if (!out.weekly.pace) out.weekly.pace = paceOfCswap(out.weekly.raw, now);
        const prev = await previous(b, id, device, out.account || null, now, out.session.resetsAt);
        if (!out.session.pace) out.session.pace = paceFrom(out.session.used, out.session.resetsAt, P.sessionMins, prev && { at: prev.at, used: prev.used }, now);
        if (!out.weekly.pace) out.weekly.pace = paceFrom(out.weekly.used, out.weekly.resetsAt, P.weeklyMins, null, now);
      }
      const active = out.active || outs.length === 1 ? 1 : 0;
      b.exec("insert into snapshots values (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", [id, device, now, out.source, out.confidence, out.session.used, out.session.resetsAt, out.weekly.used, out.weekly.resetsAt, out.monthly.used, out.monthly.resetsAt, JSON.stringify(out.models), JSON.stringify({ session: out.session.pace, weekly: out.weekly.pace }), (out.notes || [])[0] || null, out.account || null, active]);
      results.push({ provider: id, device, at: now, source: out.source, confidence: out.confidence, session_used: out.session.used, session_resets: out.session.resetsAt, session_resetText: out.session.resetText, weekly_used: out.weekly.used, weekly_resets: out.weekly.resetsAt, weekly_resetText: out.weekly.resetText, monthly_used: out.monthly.used, monthly_resets: out.monthly.resetsAt, models: out.models, pace: { session: out.session.pace, weekly: out.weekly.pace }, plan: out.plan, account: out.account || null, active: !!active, extra: out.extra, notes: out.notes || [], cached: false, now });
    }
    b.exec("delete from snapshots where at < ?", [PRUNE()]);
    return results.sort((x, y) => y.active - x.active || (x.weekly_used ?? 0) - (y.weekly_used ?? 0));
  } catch (err) {
    return fail(String(err?.message || err).slice(0, 120));
  }
}

/// Pace only when it's news: a deficit, or a projection short of reset. Reserve is silent.
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

/// One provider's reads as text: the account in use in full, the others on one line.
function section(reads, now) {
  const p = reads[0];
  const title = PROVIDERS[p.provider].title;
  if (p.error) return `${title}: unavailable, ${p.error}`;
  const head = [title, p.account && p.source === "cswap" ? `${p.account}${reads.length > 1 ? ", in use" : ""}` : null, p.plan, p.confidence === "estimated" ? "~estimated from this device only" : null].filter(Boolean).join(" · ");
  const age = p.stale ? `last read ${ago(p.at, now)} (${p.stale})` : now - p.at > FRESH ? `read ${ago(p.at, now)}` : null;
  const steered = steer(p.models);
  const others = reads.slice(1).map((o) => `${o.account} ${o.weekly_used >= 100 ? `out, back ${resets(o.weekly_resets, now)}` : `${100 - (o.weekly_used ?? 0)}% weekly left`}`);
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
  ].filter(Boolean).join("\n");
}

function tripped(p) {
  if (p.error || p.confidence === "estimated") return null;
  const left = (u) => (u == null ? null : 100 - u);
  const sLeft = left(p.session_used), wLeft = left(p.weekly_used);
  const out = sLeft != null && (sLeft < 20 || p.pace?.session?.willLast === false);
  const outW = wLeft != null && (wLeft < 15 || p.pace?.weekly?.willLast === false);
  return out || outW ? { out, outW } : null;
}

export default {
  name: "ai-usage",
  title: "AI Usage",
  icon: "gauge",
  color: "#636366",
  description: "Every AI plan's limits, and whether the pace lasts to reset",
  instructions: `Every AI plan's limits in the providers' own words: what's left, the pace against what's expected, and whether it lasts to reset. ai-usage_status is the full read (fresh when older than 5 minutes); ai-usage_warn is silent unless a window is nearly out or outruns its reset, for every-prompt hooks. Estimated rows (marked ~) come from this device only and never forecast.`,

  uses: { sidecars: ["shell"] },

  schema: Object.fromEntries(["user", "group", "global"].map((kind) => [kind, [`
    create table snapshots (provider text not null, device text not null, at integer not null, source text not null, confidence text not null, session_used real, session_resets integer, weekly_used real, weekly_resets integer, monthly_used real, monthly_resets integer, models text, pace text);
    create index snapshots_provider on snapshots (provider, device, at);`,
    `alter table snapshots add column note text;`,
    `alter table snapshots add column account text;`,
    `alter table snapshots add column active integer;`]])),

  ops: {
    status: {
      words: ["provider", "format"],
      reads: false,
      description: "Every AI plan's limits (or one provider's), each Claude account apart: what's left, the pace, and whether it lasts to reset, in the providers' own words, plus which pool to spend. Fresh when older than 5 minutes, else the last read with its age.",
      input: {
        type: "object",
        properties: {
          provider: { type: "string", enum: ["all", "claude", "codex", "opencodego"], description: "all (the default) or one" },
          format: { type: "string", enum: ["text", "json"], description: "text (the default) to read, json for screens" },
          device: { type: "string", description: "whose numbers: a device's name, from shell_accounts; omit for this device" },
        },
      },
      async run(ctx, { provider = "all", format = "text", device } = {}) {
        const b = ctx.global || ctx.user || ctx.groups[0];
        if (!b) throw new Error("there's no board for you here yet");
        const now = ctx.now();
        const dev = await here(ctx, device);
        const ids = provider === "all" ? Object.keys(PROVIDERS) : [provider];
        const all = (await Promise.all(ids.map((id) => refresh(ctx, b, id, dev, now)))).flat();
        if (format === "json") {
          return {
            device: dev, at: now,
            providers: all.map((p) => ({
              id: p.provider, title: PROVIDERS[p.provider].title, account: p.source === "cswap" ? p.account : null, active: !!p.active, plan: p.plan || null, source: p.source || null,
              device: p.device || dev, updatedAgo: p.at ? ago(p.at, now) : null, estimated: p.confidence === "estimated",
              error: p.error || null, stale: p.stale || null,
              session: p.error ? null : { used: p.session_used, left: p.session_used == null ? null : 100 - p.session_used, resets: p.session_resets ? resets(p.session_resets, now) : null, countdown: p.session_resets ? countdown(p.session_resets, now) : null, pace: p.pace?.session?.summary || null, willLast: p.pace?.session?.willLast ?? null },
              weekly: p.error ? null : { used: p.weekly_used, left: p.weekly_used == null ? null : 100 - p.weekly_used, resets: p.weekly_resets ? resets(p.weekly_resets, now) : null, countdown: p.weekly_resets ? countdown(p.weekly_resets, now) : null, pace: p.pace?.weekly?.summary || null, willLast: p.pace?.weekly?.willLast ?? null },
              monthly: p.error || p.monthly_used == null ? null : { used: p.monthly_used, left: 100 - p.monthly_used, resets: p.monthly_resets ? resets(p.monthly_resets, now) : null, countdown: p.monthly_resets ? countdown(p.monthly_resets, now) : null },
              models: (p.models || []).map((m) => ({ name: m.name, used: m.used, left: m.used == null ? null : 100 - m.used })),
              extra: p.extra || null,
              steer: steer(p.models),
            })),
          };
        }
        return ids.map((id) => all.filter((p) => p.provider === id)).filter((r) => r.length).map((r) => section(r, now)).join("\n\n");
      },
    },
    warn: {
      reads: false,
      description: "Silent unless a window is nearly out (under 20% session, 15% weekly) or outruns its reset, on every Claude account at once: then its rows and which pool to spend. For every-prompt hooks; reads the last snapshots, never fetches.",
      input: {
        type: "object",
        properties: { device: { type: "string", description: "whose numbers: a device's name; omit for this device" } },
      },
      async run(ctx, { device } = {}) {
        const b = ctx.global || ctx.user || ctx.groups[0];
        if (!b) return "";
        const now = ctx.now();
        const dev = await here(ctx, device);
        const out = [];
        for (const id of Object.keys(PROVIDERS)) {
          const reads = await latest(b, id, dev);
          // With several accounts, claude-swap moves to one with room: only all of them nearly out is news.
          if (!reads.length || reads.some((r) => !r.error && !tripped(r))) continue;
          const p = reads[0];
          const t = tripped(p);
          const rows = [];
          if (t.out) rows.push(windowText("Session", { used: p.session_used, resetsAt: p.session_resets, pace: p.pace?.session }, now));
          if (t.outW) rows.push(windowText("Weekly", { used: p.weekly_used, resetsAt: p.weekly_resets, pace: p.pace?.weekly }, now));
          rows.push(steer(p.models) || null);
          out.push([`== ${PROVIDERS[id].title}${reads.length > 1 ? `: all ${reads.length} accounts nearly out` : ""} ==`, ...rows.filter(Boolean)].join("\n"));
        }
        return out.join("\n\n");
      },
    },
  },

  screens: {
    main: {
      title: "AI Usage",
      html: `
<style>
.tile { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 12px 0; }
.tile header { display: flex; align-items: baseline; justify-content: space-between; gap: 8px; }
.tile h3 { margin: 0; font-size: 17px; }
.tile .sub { color: var(--muted); font-size: 13px; }
.win { margin-top: 12px; }
.win .row { display: flex; justify-content: space-between; gap: 8px; font-size: 15px; }
.win .row b { font-weight: 600; }
.win .pace { color: var(--muted); font-size: 13px; margin-top: 2px; }
.bar { height: 6px; border-radius: 3px; background: var(--fill); overflow: hidden; margin: 6px 0 2px; }
.bar i { display: block; height: 100%; border-radius: 3px; }
.steer { margin-top: 10px; font-size: 13px; }
.err { color: var(--muted); font-size: 15px; }
</style>
<div id="list"></div>
<script>
const tone = (p) => p == null ? "var(--text)" : p >= 85 ? "var(--danger)" : p >= 70 ? "var(--warn)" : "var(--text)";
const bar = (used) => used == null ? "" : \`<div class="bar"><i style="width:\${used}%;background:\${tone(used)}"></i></div>\`;
function win(label, w) {
  if (!w || w.used == null) return "";
  return \`<div class="win"><div class="row"><span>\${label}</span><b>\${w.used}% used</b></div>\${bar(w.used)}
    <div class="row sub"><span>\${w.pace ? one.esc(w.pace) : ""}</span><span>\${w.countdown ? "Resets " + one.esc(w.countdown) : ""}</span></div></div>\`;
}
function tile(p) {
  const head = \`<header><h3>\${one.esc(p.title)}</h3><span class="sub">\${one.esc([p.plan, p.device, p.updatedAgo ? "updated " + p.updatedAgo : ""].filter(Boolean).join(" · "))}</span></header>\`
    + (p.account ? \`<div class="sub">\${one.esc(p.account)}\${p.active ? " · in use" : ""}</div>\` : "");
  if (p.error) return \`<div class="tile">\${head}<div class="err">\${one.esc(p.error)}</div></div>\`;
  const models = (p.models || []).map((m) => \`<div class="win"><div class="row"><span>\${one.esc(m.name)}</span><b>\${m.used}% used</b></div>\${bar(m.used)}</div>\`).join("");
  const steer = p.steer ? \`<div class="steer">\${one.esc(p.steer)}</div>\` : "";
  const extra = p.extra ? \`<div class="win"><div class="row"><span>Extra usage</span><b>$\${p.extra.used} of $\${p.extra.limit}</b></div></div>\` : "";
  const est = p.estimated ? \`<div class="pace">~estimated from this device only</div>\` : "";
  return \`<div class="tile">\${head}\${win("Session", p.session)}\${win("Weekly", p.weekly)}\${win("Monthly", p.monthly)}\${models}\${extra}\${steer}\${est}</div>\`;
}
async function show() {
  const r = await one.call("status", { format: "json" });
  if (!r.providers || !r.providers.length) return one.empty("No providers yet.");
  return r.providers.map(tile).join("");
}
one.on("changed", () => one.render("#list", show));
one.render("#list", show);
</script>`,
    },
  },

  widgets: {
    usage: {
      title: "AI Usage",
      description: "Every AI plan's limits, metered",
      size: "medium",
      sizes: ["small", "medium", "large"],
      async render(ctx) {
        const b = ctx.global || ctx.user || ctx.groups[0];
        if (!b) return { t: "text", text: "No providers yet", tone: "muted" };
        const now = ctx.now();
        const dev = await here(ctx, null);
        const rows = [];
        for (const id of Object.keys(PROVIDERS)) {
          const reads = await latest(b, id, dev);
          for (const p of reads) {
            const used = p.session_used ?? p.weekly_used;
            if (used == null) continue;
            rows.push({ name: reads.length > 1 && p.account ? `${PROVIDERS[id].title} · ${p.account}` : PROVIDERS[id].title, used });
          }
        }
        if (!rows.length) return { t: "text", text: "No readings yet", tone: "muted" };
        const show = ctx.size === "small" ? rows.slice(0, 1) : rows;
        const tone = (u) => (u >= 85 ? "bad" : u >= 70 ? "warn" : "good");
        return {
          t: "col", header: false, next: Date.now() + 300_000,
          c: show.map((r) => ({ t: "row", gap: 8, c: [{ t: "col", grow: true, c: [{ t: "text", text: r.name, size: "sm", lines: 1 }] }, { t: "progress", value: r.used / 100, tone: tone(r.used), w: 90, alt: `${r.name} ${r.used}% used` }, { t: "text", text: `${r.used}%`, size: "sm", tone: "muted", lines: 1 }] })),
        };
      },
    },
  },
};
