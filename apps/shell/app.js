// Shell: commands run on the person's own servers. The shell driver, built into every server's node, runs them for
// its local user and logs each where only they can read it; this app is what people and agents use.

/// Runs one of the shell driver's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function driver(ctx, op, input) {
  try {
    return await ctx.drivers.shell[op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs shell for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Shell runs on your servers, and none of them is online.";

export default {
  name: "shell",
  title: "Shell",
  icon: "terminal",
  color: "#3a3a3c",
  description: "Run commands on your own devices",
  instructions: "Run a shell command on one of your own servers with shell_run; give server to choose which. Every command is logged where only you can read it.",
  uses: { drivers: ["shell"] },

  ops: {
    run: {
      description: "Run a shell command on this server (or the server given) and return its exit code and output. Every command is logged.",
      input: {"properties": {"command": {"description": "run by sh -c", "type": "string"}, "dir": {"description": "working directory; default is the home directory", "type": "string"}, "timeout": {"description": "seconds, default 60, at most 600", "type": "integer"}}, "required": ["command"], "type": "object"},
      run: (ctx, input) => driver(ctx, "run", input),
    },
  },
};
