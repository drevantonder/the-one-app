// Shell: commands run on the user's own devices. Its service (service/), on each device it's switched on for, runs
// them for that device's user and logs each in its folder there; this app is what users and agents use.

export default {
  name: "shell",
  title: "Shell",
  tagline: "Run commands",
  icon: "terminal-window",
  color: "#3a3a3c",
  advanced: true,
  description: "Run commands on your own devices",
  needs: "Needs one of your devices",
  instructions: "Run a shell command on one of your own devices with shell_run; give device (a name from apps_devices with app shell) to choose which, or it runs on the first online one in Device Activity's order. Every command is logged where only you can read it.",
  uses: { services: ["shell"] },
  service: { network: ["*"], user: true, platforms: ["linux", "mac"] },

  tools: {
    run: {
      title: "Run a command on a device",
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
      output: {
        type: "object",
        required: ["exit", "stdout", "stderr"],
        properties: {
          exit: { type: "integer", description: "the command's exit code: 0 when it worked, -1 when it was stopped for taking too long" },
          stdout: { type: "string", description: "what it printed, the last 64 KB when longer" },
          stderr: { type: "string", description: "what it printed as errors, the last 64 KB when longer" },
        },
      },
      run: (ctx, input) => ctx.services.shell.run(input),
    },
  },
};
