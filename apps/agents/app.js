// AGENTS.md: the AGENTS.md and skills every agent harness on every device gets. The agents sidecar, built into every
// device's node, writes them into each harness's folders there; this app is what people and agents use.

/// Runs one of the agents sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.agents[op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs agents for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "AGENTS.md runs on your devices, and none of them is online.";

const SKILL = { description: "the skill's name, from agents_skills, like writing-for-agents", type: "string" };

/// Runs an operation on one skill; one that isn't there names those that are.
async function named(ctx, op, input) {
  try {
    return await sidecar(ctx, op, input);
  } catch (err) {
    if (!String(err.message).startsWith("no skill")) throw err;
    const all = (await sidecar(ctx, "skills", {})).map((s) => s.name);
    throw new Error(`no skill ${input.name}: ${all.length ? `there's ${all.join(", ")}` : "there are none yet; agents_skill_set adds one"}`);
  }
}

export default {
  name: "agents",
  title: "AGENTS.md",
  icon: "brain",
  color: "#ff2d55",
  description: "Your agents' instructions and skills, on every device",
  instructions: "agents_get and agents_set read and replace the AGENTS.md every agent harness on every device gets (Codex, Amp, OpenCode). Skills live in every harness's skills folder: agents_skills lists them, agents_skill reads one, agents_skill_set adds or replaces one (all its files), agents_skill_remove removes one. agents_history lists AGENTS.md's revisions (or a skill's), and agents_restore brings an old one back. Every device's harnesses also get the board's prompt hook and this install's MCP server.",
  uses: { sidecars: ["agents"] },

  ops: {
    get: {
      reads: true,
      description: "The AGENTS.md every harness on every device gets, as { text }. Read it before agents_set, which replaces all of it.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "get", input),
    },
    set: {
      reads: false,
      confirm: true,
      description: "Replace the AGENTS.md every harness on every device gets: all of it, so start from agents_get's text. agents_history keeps the old one.",
      input: {"properties": {"text": {"description": "the whole new AGENTS.md, as markdown", "type": "string"}}, "required": ["text"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "set", input),
    },
    skills: {
      reads: true,
      description: "Every skill on every device: its name and description. agents_skill reads one's files.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skills", input),
    },
    skill: {
      reads: true,
      description: "One skill's files, as { name, files: { path: text } }.",
      input: {"properties": {"name": SKILL}, "required": ["name"], "type": "object"},
      run: (ctx, input) => named(ctx, "skill", input),
    },
    skill_set: {
      reads: false,
      description: "Add or replace a skill on every device: all of its files, SKILL.md among them, starting with frontmatter giving its name and description (what it's for, and when to use it).",
      input: {"properties": {"files": {"description": "each file's path within the skill → its text, like { \"SKILL.md\": \"---\\nname: writing-for-agents\\ndescription: ...\\n---\\n...\" }", "type": "object"}, "name": {"description": "the skill's folder name: lowercase words joined by -, like writing-for-agents", "type": "string"}}, "required": ["name", "files"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skill_set", input),
    },
    skill_remove: {
      reads: false,
      confirm: true,
      description: "Remove a skill from every device. agents_history with its name, then agents_restore, brings it back.",
      input: {"properties": {"name": SKILL}, "required": ["name"], "type": "object"},
      run: (ctx, input) => named(ctx, "skill_remove", input),
    },
    history: {
      reads: true,
      description: "Every revision of AGENTS.md, or of a skill if given its name, newest first: each one's id (for agents_restore), when (ISO 8601), who from which device, its size and its first changed line.",
      input: {"properties": {"name": {"description": "a skill's name, from agents_skills, like writing-for-agents; omit for AGENTS.md", "type": "string"}}, "type": "object"},
      run: async (ctx, input) => (await sidecar(ctx, "history", input)).map((r) => ({ ...r, time: new Date(r.time).toISOString() })),
    },
    restore: {
      reads: false,
      confirm: true,
      description: "Make an older revision of AGENTS.md or a skill the current one again, on every device. The one it replaces stays in agents_history.",
      input: {"properties": {"id": {"description": "the revision's id, from agents_history", "type": "string"}}, "required": ["id"], "type": "object"},
      run: async (ctx, input) => {
        try {
          return await sidecar(ctx, "restore", input);
        } catch (err) {
          if (String(err.message).startsWith("no revision")) throw new Error(`no revision ${input.id}: ids come from agents_history`);
          throw err;
        }
      },
    },
  },
};
