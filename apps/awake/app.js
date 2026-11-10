// Stay Awake: a Mac it's on for doesn't sleep while it's plugged in, even with its lid shut and its display off, so its
// user's other devices and their agents can still reach it. On battery it sleeps as usual. Its service (service/), on
// each Mac it's switched on for, holds that Mac awake and keeps whether it's on, which it is unless turned off. A widget
// can't call a service, so each Mac's setting is also kept here, as awake_set last left it, for the widget to show.

/// What's the caller's to change: nothing broke.
const refusal = (why) => Object.assign(new Error(why), { code: "refused" });

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

const macs = async (ctx) => (await ctx.devices()).filter((d) => (d.services || []).includes("awake"));

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

  schema: {
    user: `create table macs (device text primary key, awake integer not null, at integer not null)`,
  },

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
      async run(ctx, input) {
        const device = input.device || (await macs(ctx)).find((d) => d.online)?.name;
        if (!device) throw refusal("No Mac of yours that's online has Stay Awake switched on: one services on awake --device <mac>");
        const s = await service(ctx, "set", { ...input, device });
        ctx.user.exec("insert into macs (device, awake, at) values (?, ?, ?) on conflict (device) do update set awake = excluded.awake, at = excluded.at", [device, s.on ? 1 : 0, ctx.now()]);
        return s;
      },
    },
  },

  widgets: {
    macs: {
      title: "Stay Awake",
      description: "Each Mac it's on for, with a switch",
      size: "small",
      sizes: ["small", "medium"],
      async render(ctx) {
        const all = await macs(ctx);
        if (!all.length) return { t: "text", text: "No Mac has it switched on.", tone: "muted" };
        const set = Object.fromEntries((ctx.user ? await ctx.user.query("select device, awake from macs") : []).map((r) => [r.device, !!r.awake]));
        const line = (text) => ({ t: "text", text, size: "sm", tone: "muted", lines: 1 });
        const rows = all.map((d) => {
          const on = set[d.name] ?? true;
          const says = !d.online ? "Offline" : on ? "Awake while plugged in" : "Sleeps as usual";
          return { t: "col", gap: 2, c: [{ t: "toggle", label: d.name, on, tool: "set", input: { device: d.name } }, line(says)] };
        });
        return { t: "col", gap: 8, c: rows };
      },
    },
  },
};
