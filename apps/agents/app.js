// AGENTS.md: the AGENTS.md and skills every agent harness on every device gets. The agents sidecar, built into every
// server's node, writes them into each harness's folders there; this app is what people and agents use.

/// Runs one of the agents sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
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
      description: "The AGENTS.md every harness on every device gets.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "get", input),
    },
    set: {
      confirm: true,
      description: "Replace the AGENTS.md every harness on every device gets (all of it; history keeps the old one).",
      input: {"properties": {"text": {"type": "string"}}, "required": ["text"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "set", input),
    },
    skills: {
      reads: true,
      description: "Every skill: its name and description.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skills", input),
    },
    skill: {
      reads: true,
      description: "One skill's files.",
      input: {"properties": {"name": {"description": "the skill's folder name, like writing-for-agents", "type": "string"}}, "required": ["name"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skill", input),
    },
    skill_set: {
      description: "Add or replace a skill on every device: all of its files, SKILL.md first among them.",
      input: {"properties": {"files": {"description": "file path within the skill → its text", "type": "object"}, "name": {"description": "the skill's folder name, like writing-for-agents", "type": "string"}}, "required": ["name", "files"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skill_set", input),
    },
    skill_remove: {
      confirm: true,
      description: "Remove a skill from every device.",
      input: {"properties": {"name": {"description": "the skill's folder name, like writing-for-agents", "type": "string"}}, "required": ["name"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "skill_remove", input),
    },
    history: {
      reads: true,
      description: "Every revision of AGENTS.md, or of a skill if given its name, newest first: its id, when, who from which device, its size and what changed.",
      input: {"properties": {"name": {"description": "the skill's folder name, like writing-for-agents", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "history", input),
    },
    restore: {
      confirm: true,
      description: "Make an older revision (an id from history) the current one again, on every device.",
      input: {"properties": {"id": {"description": "the revision's id, from history", "type": "string"}}, "required": ["id"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "restore", input),
    },
  },
};
