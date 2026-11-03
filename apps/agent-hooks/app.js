// Agent Hooks: one hook per harness, every app behind it. A harness (Claude Code, Codex, OpenCode, Pi)
// fires a single shell hook at `one agent-hooks context`, which runs each registered provider's command on that
// device and joins what they say into one context, capped. A new app joins with agent-hooks_register: no harness
// changes, and one broken provider never stops the others.

const CAP = 800; // chars kept per provider, ~500 tokens for all of them
const TIMEOUT = 10; // seconds each provider gets

/// Where providers live: the install's board where there is one, else the caller's own.
function board(ctx) {
  return ctx.global || ctx.user || ctx.groups[0];
}

/// The harness event's name, from the JSON the harness hands the hook on stdin.
function eventName(event) {
  try {
    const e = typeof event === "string" ? JSON.parse(event) : event || {};
    return e.hook_event_name || e.hookEventName || "";
  } catch {
    return "";
  }
}

/// session-start providers run on SessionStart, every-prompt ones on UserPromptSubmit; the rest run always.
function wanted(events, name) {
  if (!events || events === "all") return true;
  const kinds = String(events).split(",").map((s) => s.trim());
  if (name === "SessionStart") return kinds.includes("session-start") || kinds.includes("all");
  if (name === "UserPromptSubmit") return kinds.includes("every-prompt") || kinds.includes("all");
  return true;
}

/// Runs each provider due for this event and joins what they say. Providers run as their own calls
/// (shell), so this works from ops and hooks alike; one failing says nothing.
async function due(ctx, name, device) {
  const b = board(ctx);
  const all = b ? await b.query("select command, events from providers order by name") : [];
  const list = all.filter((p) => wanted(p.events, name));
  const side = (input) => (device ? ctx.sidecars.shell.run(input, { device }) : ctx.sidecars.shell.run(input));
  const said = await Promise.all(list.map(async (p) => {
    const raw = String(p.command);
    // Without a device the --device flag goes too, or the call would miss its value.
    const command = (device ? raw.replaceAll("{device}", device) : raw.replace(/\s--device \{device\}/, "").replaceAll("{device}", "")).replaceAll(/  +/g, " ").trim();
    try {
      const r = await side({ command, timeout: TIMEOUT });
      const out = String(r?.stdout || "").trim();
      return r?.exit === 0 ? out.slice(0, CAP) : "";
    } catch {
      return "";
    }
  }));
  return said.filter(Boolean).join("\n").slice(0, CAP * list.length);
}

export default {
  name: "agent-hooks",
  title: "Agent Hooks",
  icon: "command",
  color: "#636366",
  description: "One hook per harness, every app behind it",
  instructions: `A harness fires one shell hook here and every app's context comes back as one. Register with agent-hooks_register: a command reading the device (like "one ai-usage status --format text --device {device}"), run when session-start, every-prompt or all. agent-hooks_context runs each in parallel and joins what they say, at most ${CAP} chars each; one failing says nothing. {device} in a command becomes the calling device, or nothing.`,

  uses: { sidecars: ["shell"] },

  schema: Object.fromEntries(["user", "group", "global"].map((kind) => [kind, [`
    create table providers (name text primary key, command text not null, events text not null default 'all', created integer not null);`]])),

  ops: {
    register: {
      words: ["name", "command"],
      reads: false,
      description: "Offer context to every harness: a command reading the device, run when session-start, every-prompt or all (the default). {device} in it becomes the calling device, or nothing. Says what's registered.",
      input: {
        type: "object",
        required: ["name", "command"],
        properties: {
          name: { type: "string", description: "this provider's name, like ai-usage" },
          command: { type: "string", description: `the command, like "one ai-usage status --format text --device {device}"` },
          events: { type: "string", description: "session-start, every-prompt, both joined with , or all (the default)" },
        },
      },
      run(ctx, { name, command, events = "all" }) {
        name = String(name || "").trim();
        command = String(command || "").trim();
        if (!name) throw new Error("give a name, like ai-usage");
        if (!command) throw new Error("give a command, like \"one ai-usage status --format text\"");
        const b = board(ctx);
        if (!b) throw new Error("there's no board for you here yet");
        b.exec("insert into providers values (?, ?, ?, ?) on conflict (name) do update set command = excluded.command, events = excluded.events", [name, command, events, ctx.now()]);
        return `Registered ${name}: ${command} (${events}).`;
      },
    },
    unregister: {
      words: ["name"],
      reads: false,
      description: "Take a provider back out: its command stops running on hooks. Says what's left.",
      input: {
        type: "object",
        required: ["name"],
        properties: { name: { type: "string", description: "the provider's name, from agent-hooks_providers" } },
      },
      run(ctx, { name }) {
        const b = board(ctx);
        if (!b) throw new Error("there's no board for you here yet");
        b.exec("delete from providers where name = ?", [name]);
        const left = b.query("select name from providers order by name").then((r) => r.map((x) => x.name));
        return left.then((names) => (names.length ? `Removed ${name}. Left: ${names.join(", ")}.` : `Removed ${name}. No providers left.`));
      },
    },
    providers: {
      reads: true,
      description: "Every registered provider: its name, command and when it runs.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const b = board(ctx);
        if (!b) return { providers: [] };
        return { providers: await b.query("select name, command, events from providers order by name") };
      },
    },
    context: {
      words: ["format"],
      reads: false,
      description: `Run each provider due for this event and join what they say, at most ${CAP} chars each, failures skipped. The harness's event JSON goes in event, as text. Gives plain text, or the harness's own answer shape with format claude or codex.`,
      input: {
        type: "object",
        additionalProperties: true,
        properties: {
          format: { type: "string", enum: ["text", "claude", "codex"], description: "text (the default) for SessionStart and UserPromptSubmit, or the harness's own answer shape" },
          event: { type: "string", description: "the harness's event JSON, as text" },
          hook_event_name: { type: "string", description: "the event's name, like SessionStart; stdin fills it when the event runs through -" },
          device: { type: "string", description: "the calling device, filling {device} in commands; omit to run where this runs" },
        },
      },
      async run(ctx, { format = "text", event, hook_event_name, device }) {
        const name = hook_event_name || eventName(event);
        const text = await due(ctx, name, device);
        if (format === "claude" || format === "codex") {
          if (!text) return {};
          return { hookSpecificOutput: { hookEventName: name || "SessionStart", additionalContext: text } };
        }
        return text;
      },
    },
  },
};
