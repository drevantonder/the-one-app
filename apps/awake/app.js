// Stay Awake: a Mac it's on for doesn't sleep while it's plugged in, even with its lid shut and its display off, so its
// user's other devices and their agents can still reach it. On battery it sleeps as usual. Its service (service/), on
// each Mac it's switched on for, holds that Mac awake and keeps whether it's on, which it is unless turned off.

const service = (ctx, fn, { device, ...input }) => ctx.services.awake[fn](input, device ? { device } : undefined);

const DEVICE = { type: "string", description: "the Mac, by name, as apps_devices lists it for awake; else the first that has it" };
const STATUS = {
  type: "object",
  required: ["on", "holding", "plugged_in"],
  properties: {
    on: { type: "boolean", description: "whether Stay Awake is on for this Mac" },
    holding: { type: "boolean", description: "whether it holds the Mac awake now" },
    plugged_in: { type: "boolean", description: "whether the Mac is on power, which it must be to stay awake" },
  },
};

// What a Mac's row says it's doing, in a few words.
const doing = (s) =>
  !s ? "Offline" : !s.on ? "Sleeps as usual" : !s.plugged_in ? "On battery: sleeps as usual" : s.holding ? "Awake while plugged in" : "Couldn't keep it awake";

export default {
  name: "awake",
  title: "Stay Awake",
  tagline: "Keep a Mac online",
  icon: "coffee",
  color: "#a2845e",
  advanced: true,
  description: "Keep a plugged-in Mac online, even with its lid shut",
  needs: "Needs a Mac",
  instructions: "A Mac with Stay Awake on doesn't sleep while it's plugged in, even with its lid shut, so your other devices and their agents can reach it; on battery it sleeps as usual. Its service must be switched on for the Mac (Settings › Devices, or one services on awake --device <name>); then it's on unless turned off. awake_set turns it on or off for a Mac, and awake_status says whether it's keeping that Mac awake now. Its widget has a switch for each Mac.",
  uses: { services: ["awake"] },
  service: { network: [], user: false, platforms: ["mac"] },

  tools: {
    status: {
      title: "Whether a Mac is kept awake",
      reads: true,
      description: "Whether Stay Awake is on for a Mac, whether it's keeping it from sleeping now, and whether it's plugged in: a Mac on battery sleeps as usual.",
      input: { type: "object", properties: { device: DEVICE } },
      output: STATUS,
      run: (ctx, input) => service(ctx, "status", input),
    },
    set: {
      title: "Turn Stay Awake on or off for a Mac",
      reads: false,
      description: "Turn Stay Awake on or off for a Mac. On, it doesn't sleep while it's plugged in, even with its lid shut; off, it sleeps as usual. Gives how it stands after.",
      input: { type: "object", required: ["on"], properties: { on: { type: "boolean" }, device: DEVICE } },
      output: STATUS,
      run: (ctx, input) => service(ctx, "set", input),
    },
  },

  widgets: {
    macs: {
      title: "Stay Awake",
      description: "Each Mac it's on for, with a switch",
      size: "small",
      sizes: ["small", "medium"],
      async render(ctx) {
        const macs = (await ctx.devices()).filter((d) => (d.services || []).includes("awake"));
        if (!macs.length) return { t: "text", text: "No Mac has it switched on.", tone: "muted" };
        const rows = await Promise.all(
          macs.map(async (d) => {
            const s = d.online ? await service(ctx, "status", { device: d.name }).catch(() => null) : null;
            const name = s ? { t: "toggle", label: d.name, on: s.on, tool: "set", input: { device: d.name } } : { t: "text", text: d.name, weight: "medium" };
            return { t: "col", gap: 2, c: [name, { t: "text", text: doing(s), size: "sm", tone: "muted", lines: 1 }] };
          }),
        );
        return { t: "col", gap: 8, c: rows };
      },
    },
  },
};
