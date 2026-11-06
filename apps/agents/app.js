// AGENTS.md: the AGENTS.md and skills every agent harness on every device gets. They're records in the person's own
// database, oldest first, each a revision; the newest of each says what's current. Its service (service/), on each
// device it's switched on for, asks for them (_files) and writes them into each harness's folders there.

const SKILL = { description: "the skill's name, from agents_skills, like writing-for-agents", type: "string" };

async function records(ctx) {
  const db = ctx.user;
  if (!db) throw new Error("AGENTS.md is each person's own: this call reaches no one's");
  const rows = await db.query("select n, kind, data, time, by, device from records order by n");
  return rows.map((r) => ({ id: String(r.n), kind: r.kind, data: JSON.parse(r.data), time: r.time, by: r.by, device: r.device }));
}

// Written once the call returns.
function keep(ctx, kind, data) {
  ctx.user.exec("insert into records (kind, data, time, by, device) values (?, ?, ?, ?, ?)", [kind, JSON.stringify(data), ctx.now(), ctx.me.user_name, ctx.me.name]);
}

function state(all) {
  let text = "";
  const skills = new Map();
  for (const r of all) {
    if (r.kind === "agentsmd") text = String(r.data.text ?? "");
    else if (r.kind === "skill" && !invalid(r.data.name, r.data.files)) skills.set(r.data.name, r.data.files);
    else if (r.kind === "skill_removed") skills.delete(r.data.name);
  }
  return { text, skills: new Map([...skills].sort(([a], [b]) => (a < b ? -1 : 1))) };
}

function invalid(name, files) {
  if (typeof name !== "string" || !/^[a-z0-9-]+$/.test(name)) return "Not added: skill names are lowercase words joined by -";
  if (!files || typeof files !== "object" || typeof files["SKILL.md"] !== "string") return "Not added: a skill needs a SKILL.md";
  for (const [path, text] of Object.entries(files)) {
    if (typeof text !== "string") return `${path} isn't text`;
    if (path.startsWith("/") || path.includes("\\") || path.split("/").some((p) => p === "" || p === "." || p === "..")) return `${path} isn't a path inside the skill`;
  }
  return null;
}

// A SKILL.md's frontmatter as harnesses read it: `---` on the first line and on one after it, between them each
// `key: value`, a folded one (`key: >`) on the indented lines after it. A string says what's missing.
function frontmatter(md) {
  const lines = String(md).split("\n").map((l) => l.replace(/\s+$/, ""));
  if (lines[0] !== "---") return "first line must be ---, starting its frontmatter";
  const end = lines.indexOf("---", 1);
  if (end < 0) return "frontmatter must end with a line of ---";
  const front = lines.slice(1, end);
  const out = {};
  front.forEach((line, i) => {
    const at = line.indexOf(":");
    if (at <= 0 || /^[ \t]/.test(line)) return;
    let value = line.slice(at + 1).trim();
    if (/^[>|][-+0-9]*$/.test(value)) {
      const folded = [];
      for (const l of front.slice(i + 1)) {
        if (l !== "" && !/^[ \t]/.test(l)) break;
        if (l.trim()) folded.push(l.trim());
      }
      value = folded.join(" ");
    }
    value = value.replace(/^["']+|["']+$/g, "").trim();
    if (value) out[line.slice(0, at)] = value;
  });
  return out;
}

// Harnesses load a skill by its frontmatter's name and description.
function unintroduced(name, files) {
  const example = `---\nname: ${name}\ndescription: what it's for, and when to use it\n---`;
  const front = frontmatter(files["SKILL.md"]);
  if (typeof front === "string") return `Not added: SKILL.md's ${front}:\n${example}`;
  if (!front.name) return `Not added: SKILL.md's frontmatter has no name; it starts:\n${example}`;
  if (front.name !== name) return `Not added: SKILL.md's frontmatter names it ${front.name}, not ${name}: they must be the same`;
  if (!front.description) return `Not added: SKILL.md's frontmatter has no description, which says what it's for and when to use it:\n${example}`;
  return null;
}

function firstChange(before, now) {
  const [a, b] = [before ? before.split("\n") : [], now ? now.split("\n") : []];
  for (let i = 0; ; i++) {
    if (i < a.length && i < b.length && a[i] === b[i]) continue;
    if (i < b.length) return `+ ${[...b[i]].slice(0, 100).join("")}`;
    if (i < a.length) return `- ${[...a[i]].slice(0, 100).join("")}`;
    return "no change";
  }
}

function history(all, skill) {
  let before = null;
  const out = [];
  for (const r of all) {
    let now;
    if (!skill && r.kind === "agentsmd") now = String(r.data.text ?? "");
    else if (skill && r.kind === "skill" && r.data.name === skill) now = Object.keys(r.data.files || {}).sort().map((p) => `--- ${p}\n${r.data.files[p]}\n`).join("");
    else if (skill && r.kind === "skill_removed" && r.data.name === skill) now = null;
    else continue;
    const change = now === null ? "removed" : firstChange(before ?? "", now);
    out.push({ id: r.id, time: new Date(r.time).toISOString(), by: r.by, device: r.device, size: now === null ? 0 : new TextEncoder().encode(now).length, change });
    before = now;
  }
  return out.reverse();
}

function named(skills, name) {
  const files = skills.get(name);
  if (files) return files;
  const all = [...skills.keys()];
  throw new Error(`no skill ${name}: ${all.length ? `there's ${all.join(", ")}` : "there are none yet; agents_skill_set adds one"}`);
}

const EVERY_DEVICE = "every device's harnesses get it within a minute";

export default {
  name: "agents",
  title: "AGENTS.md",
  tagline: "Rules for your AI",
  icon: "brain",
  color: "#ff2d55",
  advanced: true,
  description: "Your agents' instructions and skills, on every device",
  instructions: "agents_get and agents_set read and replace the AGENTS.md every agent harness on every device gets (Codex, Amp, OpenCode). Skills live in every harness's skills folder: agents_skills lists them, agents_skill reads one, agents_skill_set adds or replaces one (all its files), agents_skill_remove removes one. agents_history lists AGENTS.md's revisions (or a skill's), and agents_restore brings an old one back. Every device's harnesses also get the board's prompt hook and this network's MCP server from its node.",
  service: { person: true, platforms: ["linux", "mac"] },
  schema: {
    user: [`
      create table records (n integer primary key, kind text not null, data text not null, time integer not null, by text not null, device text not null);
      create table moved (records integer not null);`,
      `drop table moved;`],
  },

  tools: {
    get: {
      reads: true,
      description: "The AGENTS.md every harness on every device gets, as { text }. Read it before agents_set, which replaces all of it.",
      input: { properties: {}, type: "object" },
      run: async (ctx) => ({ text: state(await records(ctx)).text }),
    },
    set: {
      reads: false,
      confirm: true,
      description: "Replace the AGENTS.md every harness on every device gets: all of it, so start from agents_get's text. agents_history keeps the old one.",
      input: { properties: { text: { description: "the whole new AGENTS.md, as markdown", type: "string" } }, required: ["text"], type: "object" },
      run: async (ctx, input) => {
        if (typeof input.text !== "string") throw new Error("give the text");
        await records(ctx);
        keep(ctx, "agentsmd", { text: input.text });
        return `AGENTS.md is set; ${EVERY_DEVICE}.`;
      },
    },
    skills: {
      reads: true,
      description: "Every skill on every device: its name and description. agents_skill reads one's files.",
      input: { properties: {}, type: "object" },
      run: async (ctx) => [...state(await records(ctx)).skills].map(([name, files]) => {
        const front = frontmatter(files["SKILL.md"]);
        return { name, description: (typeof front === "object" && front.description) || "" };
      }),
    },
    skill: {
      reads: true,
      description: "One skill's files, as { name, files: { path: text } }.",
      input: { properties: { name: SKILL }, required: ["name"], type: "object" },
      run: async (ctx, input) => ({ name: input.name, files: named(state(await records(ctx)).skills, input.name) }),
    },
    skill_set: {
      reads: false,
      description: "Add or replace a skill on every device: all of its files, SKILL.md among them, starting with frontmatter giving its name and description (what it's for, and when to use it).",
      input: { properties: { files: { description: "each file's path within the skill → its text, like { \"SKILL.md\": \"---\\nname: writing-for-agents\\ndescription: ...\\n---\\n...\" }", type: "object" }, name: { description: "the skill's folder name: lowercase words joined by -, like writing-for-agents", type: "string" } }, required: ["name", "files"], type: "object" },
      run: async (ctx, input) => {
        const why = invalid(input.name, input.files) || unintroduced(input.name, input.files);
        if (why) throw new Error(why);
        await records(ctx);
        keep(ctx, "skill", { name: input.name, files: input.files });
        return `${input.name} is set; ${EVERY_DEVICE}.`;
      },
    },
    skill_remove: {
      reads: false,
      confirm: true,
      description: "Remove a skill from every device. agents_history with its name, then agents_restore, brings it back.",
      input: { properties: { name: SKILL }, required: ["name"], type: "object" },
      run: async (ctx, input) => {
        named(state(await records(ctx)).skills, input.name);
        keep(ctx, "skill_removed", { name: input.name });
        return `${input.name} is removed; ${EVERY_DEVICE}.`;
      },
    },
    history: {
      reads: true,
      description: "Every revision of AGENTS.md, or of a skill if given its name, newest first: each one's id (for agents_restore), when (ISO 8601), who from which device, its size and its first changed line.",
      input: { properties: { name: { description: "a skill's name, from agents_skills, like writing-for-agents; omit for AGENTS.md", type: "string" } }, type: "object" },
      run: async (ctx, input) => history(await records(ctx), input.name),
    },
    restore: {
      reads: false,
      confirm: true,
      description: "Make an older revision of AGENTS.md or a skill the current one again, on every device. The one it replaces stays in agents_history.",
      input: { properties: { id: { description: "the revision's id, from agents_history", type: "string" } }, required: ["id"], type: "object" },
      run: async (ctx, input) => {
        const r = (await records(ctx)).find((r) => r.id === String(input.id));
        if (!r) throw new Error(`no revision ${input.id}: ids come from agents_history`);
        if (r.kind === "agentsmd") {
          keep(ctx, "agentsmd", r.data);
          return `That AGENTS.md is current again; ${EVERY_DEVICE}.`;
        }
        if (r.kind !== "skill") throw new Error(`${input.id} isn't a revision of AGENTS.md or a skill`);
        const why = invalid(r.data.name, r.data.files);
        if (why) throw new Error(why);
        keep(ctx, "skill", r.data);
        return `That ${r.data.name} is current again; ${EVERY_DEVICE}.`;
      },
    },
    _files: {
      reads: true,
      description: "AGENTS.md and every skill's files, for the service to write.",
      input: { properties: {}, type: "object" },
      run: async (ctx) => {
        const { text, skills } = state(await records(ctx));
        return { text, skills: Object.fromEntries(skills) };
      },
    },
  },
};
