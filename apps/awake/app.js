// Stay Awake: a Mac it is switched on for (Settings › Devices) doesn't sleep while it's plugged in, even with its lid
// shut and its display off, so its user's other devices and their agents can still reach it. On battery it sleeps as
// usual. Its service (service/) holds the Mac awake for as long as it runs.

export default {
  name: "awake",
  title: "Stay Awake",
  tagline: "Keep a Mac online",
  icon: "coffee",
  color: "#a2845e",
  advanced: true,
  description: "Keep a plugged-in Mac online, even with its lid shut",
  needs: "Needs a Mac",
  instructions: "A Mac with Stay Awake switched on doesn't sleep while it's plugged in, even with its lid shut, so your other devices and their agents can reach it; on battery it sleeps as usual. Switch it on or off for a device in Settings › Devices, or with one services on awake --device <name>. awake_status says whether a Mac is being kept awake now.",
  uses: { services: ["awake"] },
  service: { network: [], user: false, platforms: ["mac"] },

  tools: {
    status: {
      title: "Whether a Mac is kept awake",
      reads: true,
      description: "Whether Stay Awake is keeping a Mac from sleeping now, and whether it's plugged in: a Mac on battery sleeps as usual.",
      input: {
        type: "object",
        properties: {
          device: { type: "string", description: "the Mac, by name, as apps_devices lists it for awake; else the first that has it" },
        },
      },
      output: {
        type: "object",
        required: ["holding", "plugged_in"],
        properties: {
          holding: { type: "boolean", description: "whether it holds the Mac awake now" },
          plugged_in: { type: "boolean", description: "whether the Mac is on power, which it must be to stay awake" },
        },
      },
      run: (ctx, { device, ...input }) => ctx.services.awake.status(input, device ? { device } : undefined),
    },
  },
};
