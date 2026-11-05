// Shell: commands run on the person's own devices. Its service (service/), on each device it's switched on for, runs
// them for that device's person and logs each in its folder there; this app is what people and agents use.

/// Runs one of the shell service's functions, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function service(ctx, fn, input) {
  try {
    return await ctx.services.shell[fn](input);
  } catch (err) {
    if (String(err.message).includes("no device runs shell for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Shell runs on your devices, and none of them is online.";

export default {
  name: "shell",
  title: "Shell",
  tagline: "Run commands",
  icon: "terminal-window",
  color: "#3a3a3c",
  advanced: true,
  description: "Run commands on your own devices",
  connector: true,
  needs: "Needs one of your devices",
  instructions: "Run a shell command on one of your own devices with shell_run; give device (a name from shell_devices) to choose which, or it runs on the first online one in Device Activity's order. Every command is logged where only you can read it.",
  uses: { services: ["shell"] },
  service: { network: ["*"], person: true, platforms: ["linux", "mac"] },

  tools: {
    devices: {
      reads: true,
      description: "The devices Shell runs on: each { device, online }, device the device's name. Give one as device to shell_run.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.services || []).includes("shell")).map((s) => ({ device: s.name, online: s.online })),
    },
    remove: {
      reads: false,
      confirm: true,
      description: "Stop using Shell on one of your devices: removes it from that device, for one of its owners, so no command runs there.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the device's name, as shell_devices lists it" } } },
      run: (ctx, { device }) => {
        ctx.service("shell", device, false);
        return `Shell is removed from ${device}.`;
      },
    },
    run: {
      reads: false,
      description: "Run a shell command on one of your devices, as its owner, and wait for it to finish. Gives { exit, stdout, stderr }: exit 0 when it worked, and what it printed, the last 64 KB of each when longer. Every command is logged where only you can read it.",
      input: {
        type: "object",
        required: ["command"],
        properties: {
          command: { type: "string", description: "the command line, run by sh -c, like \"df -h\" or \"cd src && git status\"; it reads no input" },
          dir: { type: "string", description: "the folder to run it in, as an absolute path; default the home folder" },
          timeout: { type: "integer", description: "seconds to wait before stopping it, 1 to 600; default 60" },
        },
      },
      run: (ctx, input) => service(ctx, "run", input),
    },
  },
};
