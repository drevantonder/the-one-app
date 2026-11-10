// Computer Use: the devices agents can use, and the tools they use them with. Agent devices are Linux devices with a
// browser on the user's devices, which Agent Devices runs (Podman, or a Mac's containers); the user's own
// devices are here too once they let agents use them, through the computer service, a helper app inside the Mac app (service/). Use,
// tools and screen go to a device when one is named. Agent Devices (apps/agent-devices) is the user's page for them. Its screen is
// React, in screens/, built into dist/ when it's deployed (`one deploy apps/computer`), which is committed because a
// release ships the app as it is.

import main from "./dist/main.html" with { type: "text" };

/// An agent device's tool. A name that's one of the user's own devices only gets here when that device can't take the call,
/// and Agent Devices, which knows only its own, would say there's no such device.
async function act(ctx, fn, input) {
  try {
    return await ctx.apps["agent-devices"]._act({ function: fn, input });
  } catch (err) {
    const n = String(input?.device || "").toLowerCase();
    const own = n && (await ctx.devices()).find((s) => s.name.toLowerCase() === n);
    if (!own) throw err;
    throw new Error(`${own.name} ${own.online ? "can't be used right now: it's still starting, or Computer Use isn't on there" : "is offline right now"}. Try again once it's back.`);
  }
}

/// The caller's devices whose computer service serves them, by device name: `{ device, on, title, kind, using,
/// permissions, missing }`, or `{ device, why }` for one that didn't answer.
async function devices(ctx) {
  const mine = (s) => {
    const d = (s.installed_services || []).find((d) => d.name === "computer");
    return d ? !d.users || d.users.includes(ctx.me.user_name) : (s.services || []).includes("computer");
  };
  const theirs = (await ctx.devices()).filter((s) => s.online && mine(s));
  return Promise.all(theirs.map(async (s) => {
    try {
      return { device: s.name, ...(await ctx.services.computer._status({}, { device: s.name })) };
    } catch (err) {
      return { device: s.name, on: false, why: err.message };
    }
  }));
}

async function device(ctx, name) {
  if (!name) return null;
  const n = String(name).toLowerCase();
  const s = (await ctx.devices()).find((s) => s.name.toLowerCase() === n && (s.services || []).includes("computer"));
  return s ? s.name : null;
}

/// A browser tool: on the user's own device when one is named, else an agent device.
const browser = (tool) => async (ctx, { device: name, ...args }) => {
  const d = await device(ctx, name);
  return d ? ctx.services.computer._use({ tool, args }, { device: d }) : act(ctx, tool, name ? { device: name, ...args } : args);
};

const asListed = (d) => ({ name: d.device, on: d.device, own: true, kind: d.kind, title: d.title, state: "running", use: d.using ? `${d.using.agent} is using it` : "agents may use it" });

const KINDS = { linux: "Linux with a browser", chrome: "Chrome", windows: "Windows", android: "Android", iphone: "iPhone" };

function shown(d) {
  const b = d.borrowed;
  const use = b?.signing_in ? `the user is signing in to sites in the ${b.profile || "browser"} profile`
    : b ? `borrowed by ${b.by || "an agent"} for ${b.for || "something"} since ${new Date(b.since).toISOString().slice(0, 16)}Z`
    : d.spare ? "the spare, kept ready for agent-devices_borrow"
    : d.kind === "linux" ? "a lasting device of the user's" : "free to borrow";
  return {
    name: d.name, kind: d.kind, title: KINDS[d.kind] || d.kind, on: d.on, ...(d.of ? { of: d.of } : {}),
    state: d.state, use, ...(d.size ? { size: d.size } : {}),
    ...(d.memory ? { memory: d.memory >= 1e9 ? `${(d.memory / 2 ** 30).toFixed(1)} GB in use` : `${Math.round(d.memory / 2 ** 20)} MB in use` } : {}),
  };
}

const DEVICE = { description: "which device, by name, from computer_list; default the only one", type: "string" };
const SESSION = { description: "a short name for your work, the same on every browser call; its tabs and refs belong to it", type: "string" };
const TAB = { description: "the tab's id, from computer_get_browser_state", type: "string" };
const TARGET = { description: "the browser's target_id, from computer_get_browser_state", type: "string" };
const REF = { description: "the thing's ref on the page, like p3:12, from the latest computer_get_browser_state: a new read or a navigation replaces them", type: "string" };
const FOREGROUND = { default: "background", description: "background (the default) refuses where acting would bring the browser window to the front; foreground allows it, as on an agent device nobody is watching", enum: ["background", "foreground"], type: "string" };

// What a tool of a device gives, as MCP gives it: text, and pictures as base64.
const CONTENT = {
  type: ["object", "string"],
  description: "{ content }, a list of { type: text, text } and { type: image, mimeType, data }; text on an own device",
  properties: { content: { type: "array", items: { type: "object", required: ["type"], additionalProperties: true } } },
  additionalProperties: true,
};

export default {
  name: "computer",
  title: "Computer Use",
  tagline: "AI uses a device",
  icon: "cursor-click",
  color: "#5e5ce6",
  advanced: true,
  description: "Devices your agents can use: agent devices with a browser, and your own devices when you let them",
  needs: "Needs a Linux device with Podman, or a Mac",
  instructions: "Computer Use acts on devices: agent devices (Linux with a browser, Windows, Android and iPhones, on the user's own devices) and the user's own devices they let agents use (own: true in computer_list, like their Mac). Agent Devices (agent-devices_*) gets you one and gives it back; Computer Use (computer_*) acts on it.\n\nTo browse, agent-devices_borrow with for (a few words the user sees) gives you a Linux device whose browser starts signed in to a browser profile's sites. Pass its name as device to every call, then:\n1. computer_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":<the profile borrow gave>} starts its browser.\n2. computer_use with tool list_windows gives the browser's pid and window_id.\n3. computer_get_browser_state with those gives target_id and tab_id; with those, it reads the page and the refs of what's on it.\n4. computer_browser_navigate, _click, _type, _select and _pointer act on it; on an agent device, click and pointer take input_route \"dom_event\". Read the page again after each.\nA browser idle for 5 minutes closes: prepare it again.\nBorrowed with kind chrome (quicker, where agent-devices_kinds lists it), the device is that browser: borrow gives target_id and tab_id, so skip steps 1 to 3 and read the page with computer_get_browser_state. Give the device back with agent-devices_give_back when done; one left 15 minutes without a call is given back for you.\n\nWhen a site needs the user to sign in, agent-devices_sign_in gives them a link to do it themselves; for a challenge, give them computer_view's link. Never sign in or submit forms for them unless they asked.\n\nAnything else on a device (apps, windows, keyboard and mouse, and Windows, Android and iPhones): computer_tools lists what it takes, computer_use runs one, and computer_screen shows the screen. An iPhone takes no clicks or typing.\n\nOn the user's own device, do only what they asked of it, and never read their messages, mail, browser sessions or files unless they asked. They see in their menu bar while you use it, and can stop you.",
  uses: { apps: ["agent-devices"], services: ["computer"] },
  service: { network: [], user: true, platforms: ["mac"] },
  labs: {
    mirroring: { title: "iPhone Mirroring", description: "Use your iPhone through a Mac." },
  },

  schema: {
    user: [`create table sessions (id text primary key, device text not null, title text, agent text not null, came_from text, time integer not null)`,
      `drop table sessions;`],
  },


  tools: {
    list: {
      title: "Every device agents can act on",
      reads: true,
      description: "Every device agents can act on: the agent devices on all of your devices, and your own devices you let agents use (own: true, like your Mac). Each with its kind, the device it's on, whether it's running, paused or stopped, and who has borrowed it and what for. Borrow one with agent-devices_borrow; act on it with computer_screen, computer_use and the browser tools.",
      input: {"properties": {"here": {"description": "only the agent devices on the device you call from", "type": "boolean"}}, "type": "object"},
      output: {
        type: "object",
        required: ["devices"],
        properties: {
          devices: {
            type: "array",
            items: {
              type: "object",
              required: ["name", "on"],
              properties: {
                name: { type: "string", description: "for the device argument of the other tools" },
                kind: { type: "string" },
                title: { type: "string", description: "the kind, in words" },
                on: { type: "string", description: "the device of yours it runs on" },
                own: { type: "boolean", description: "true for one of your own devices you let agents use" },
                of: { type: "string", description: "for a throwaway copy, the device it copies" },
                state: { type: "string", description: "running, paused or stopped" },
                use: { type: "string", description: "who has it and what for, in words" },
                size: { type: "string" },
                memory: { type: "string" },
              },
            },
          },
          note: { type: "string" },
          unreachable: { description: "the devices that couldn't be asked, and why" },
        },
      },
      run: async (ctx, input) => {
        const [listed, devs] = await Promise.all([act(ctx, "list", input).catch((err) => err), input.here ? [] : devices(ctx)]);
        const on = devs.filter((d) => d.on).map(asListed);
        if (listed instanceof Error) {
          if (on.length && listed.code === "restarting") return { devices: on };
          throw listed;
        }
        const { devices: theirs = [], ...rest } = listed || {};
        return { devices: [...on, ...theirs.map(shown)], ...rest };
      },
    },
    view: {
      title: "A link to watch a device",
      reads: false,
      description: "A link for the user to watch and use an agent device in their browser: to solve a challenge or sign in, or to see what an agent is doing. Gives the link and when it stops working. Not for your own devices: the user sees those on the Computer Use page.",
      input: {"properties": {"device": DEVICE, "minutes": {"description": "how long the link works; default 60, at most 1440", "type": "integer"}}, "type": "object"},
      output: { type: "string", description: "the link, and when it stops working" },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) throw new Error(`${d} is the user's own device: they watch it on the Computer Use page and on the device itself, and stop agents there.`);
        return act(ctx, "view", input);
      },
    },
    screen: {
      title: "A device's screen",
      reads: true,
      description: "A picture of a device's screen as it is now: an agent device or your own device. To read a web page, computer_get_browser_state gives its text.",
      input: {"properties": {"device": DEVICE, "width": {"description": "the picture's width in pixels; default 640, at most 1920", "type": "integer"}}, "type": "object"},
      output: CONTENT,
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) return ctx.services.computer._screen({ width: input.width }, { device: d });
        return act(ctx, "screen", input);
      },
    },
    tools: {
      title: "What a device's tools can do",
      reads: true,
      description: "What computer_use can do on a device, a line each (like list_windows, click, type_text); with name, that tool's full description and input.",
      input: {"properties": {"device": DEVICE, "name": {"description": "a tool's name, from computer_tools without name", "type": "string"}}, "type": "object"},
      output: { type: "string", description: "a line for each tool; with name, its full description and input" },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) return ctx.services.computer._tools(input.name ? { name: input.name } : {}, { device: d });
        return act(ctx, "tools", input);
      },
    },
    refresh_tools: {
      title: "Read the devices' tools again",
      reads: false,
      description: "Read the agent devices' tools again from a running one, as after it was updated, when computer_tools seems out of date.",
      input: {"properties": {}, "type": "object"},
      output: { type: "string", description: "how many tools were read, in words" },
      run: (ctx, input) => act(ctx, "refresh_tools", input),
    },
    use: {
      title: "Run a device's tool",
      reads: false,
      description: "Run one of a device's tools, as computer_tools lists them (list_windows, launch_app, click, type_text, press_key, get_guide...), on an agent device or your own device, and give what it answers. For web pages, the browser tools are quicker.",
      input: {"properties": {"args": {"description": "the tool's input, as computer_tools with its name describes it, like { \"x\": 120, \"y\": 300 }", "type": "object"}, "device": DEVICE, "tool": {"description": "the tool's name, from computer_tools", "type": "string"}}, "required": ["tool"], "type": "object"},
      output: CONTENT,
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) return act(ctx, "use", input);
        return ctx.services.computer._use({ tool: input.tool, args: input.args || {} }, { device: d });
      },
    },
    get_browser_state: {
      title: "Read a web page",
      reads: true,
      description: "Read the web page in a browser window: its text, and a ref for each thing to click, type in or select. First call it with pid and window_id (from computer_use list_windows) to get target_id and tab_id; after that, with those. If the browser isn't ready, it says which computer_browser_prepare call makes it so.",
      input: {"additionalProperties": true, "properties": {
        "continuation": {"description": "from an earlier semantic_v2 read, to read on from there", "type": "string"},
        "device": DEVICE,
        "include_screenshot": {"default": false, "description": "true to add a picture of the tab", "type": "boolean"},
        "pid": {"description": "the browser's process id, from computer_use list_windows: with window_id, on the first call", "type": "integer"},
        "query": {"description": "only what matches these words, by role, label or visible text", "type": "string"},
        "scope_ref": {"description": "a ref from an earlier read: read only that part of the page", "type": "string"},
        "session": SESSION,
        "snapshot_format": {"description": "dom_refs_v1 (the default), or semantic_v2 for an outline of what's visible first", "enum": ["dom_refs_v1", "semantic_v2"], "type": "string"},
        "tab_id": TAB,
        "target_id": TARGET,
        "window_id": {"description": "the browser window's id, from computer_use list_windows: with pid, on the first call", "type": "integer"},
      }, "type": "object"},
      output: CONTENT,
      run: browser("get_browser_state"),
    },
    browser_prepare: {
      title: "Start or attach a browser",
      reads: false,
      description: "Start the browser the browser tools act on, or attach them to one that's open. On an agent device: allow_launch true and profile { mode: \"isolated_named\", name: <the profile agent-devices_borrow gave> }. In a browser open on your own device: pid, window_id and strategy { kind: \"existing_profile\" }, when computer_get_browser_state asks for it. It says why if it can't, having changed nothing. A browser it started closes after 5 minutes idle; prepare it again.",
      input: {"additionalProperties": true, "properties": {
        "allow_launch": {"description": "true to start a browser of its own, with profile", "type": "boolean"},
        "delivery_mode": {"default": "background", "description": "background (the default) refuses where attaching would bring the browser in front of the user; foreground allows it and puts their app back after", "enum": ["background", "foreground"], "type": "string"},
        "device": DEVICE,
        "pid": {"description": "the browser's process id, from computer_use list_windows; not needed with allow_launch", "type": "integer"},
        "profile": {"additionalProperties": false, "description": "the browser profile to start with allow_launch: { mode: \"isolated_named\", name } keeps it after, { mode: \"isolated_new\" } is a fresh one", "properties": {"mode": {"enum": ["isolated_new", "isolated_named"], "type": "string"}, "name": {"description": "with isolated_named: the profile's name, as agent-devices_borrow gives it", "type": "string"}}, "required": ["mode"], "type": "object"},
        "session": SESSION,
        "strategy": {"additionalProperties": false, "description": "{ kind: \"existing_profile\" } attaches to the browser open at pid and window_id", "properties": {"kind": {"enum": ["existing_profile"], "type": "string"}}, "required": ["kind"], "type": "object"},
        "window_id": {"description": "the browser window's id, from computer_use list_windows; needed with strategy", "type": "integer"},
      }, "required": [], "type": "object"},
      output: CONTENT,
      run: browser("browser_prepare"),
    },
    browser_navigate: {
      title: "Open a web address in a tab",
      reads: false,
      description: "Open a web address in a browser tab. Refs from earlier reads stop working: read the page again with computer_get_browser_state.",
      input: {"additionalProperties": true, "properties": {"device": DEVICE, "session": SESSION, "tab_id": TAB, "target_id": TARGET, "url": {"description": "an http, https or about address, like https://example.com", "type": "string"}}, "required": ["target_id", "tab_id", "url"], "type": "object"},
      output: CONTENT,
      run: browser("browser_navigate"),
    },
    browser_click: {
      title: "Click on a web page",
      reads: false,
      description: "Click something on a web page, by its ref or at a point. On an agent device give input_route \"dom_event\", or delivery_mode \"foreground\". Read the page again to see that it worked.",
      input: {"additionalProperties": true, "properties": {
        "delivery_mode": FOREGROUND,
        "device": DEVICE,
        "input_route": {"description": "trusted (the default) clicks as a mouse does; dom_event clicks in the page itself, needs ref, and some controls ignore it", "enum": ["trusted", "dom_event"], "type": "string"},
        "ref": REF, "session": SESSION, "tab_id": TAB, "target_id": TARGET,
        "x": {"description": "instead of ref: a point in the tab, in CSS pixels from its left", "type": "number"},
        "y": {"description": "instead of ref: a point in the tab, in CSS pixels from its top", "type": "number"},
      }, "required": ["target_id", "tab_id"], "type": "object"},
      output: CONTENT,
      run: browser("browser_click"),
    },
    browser_type: {
      title: "Type into a web page",
      reads: false,
      description: "Type text into a field on a web page, by its ref. It adds to what the field holds; replace true sets it instead (with \"\", clears it). \"\\n\" with mode keystrokes presses Enter.",
      input: {"additionalProperties": true, "properties": {
        "device": DEVICE,
        "mode": {"description": "insert_text (the default) puts it in at once; keystrokes presses each key, for fields that need them", "enum": ["insert_text", "keystrokes"], "type": "string"},
        "ref": REF,
        "replace": {"description": "true to replace what the field holds; false (the default) adds to it", "type": "boolean"},
        "session": SESSION, "tab_id": TAB, "target_id": TARGET,
        "text": {"description": "what to type", "type": "string"},
      }, "required": ["target_id", "tab_id", "ref", "text"], "type": "object"},
      output: CONTENT,
      run: browser("browser_type"),
    },
    browser_select: {
      title: "Pick a drop-down option",
      reads: false,
      description: "Pick an option of a drop-down on a web page, by its ref, without opening it: give label or value, which exactly one option must match. Read the page again to see the page took it.",
      input: {"additionalProperties": false, "properties": {
        "device": DEVICE,
        "label": {"description": "the option's text, exactly as shown", "type": "string"},
        "ref": REF, "session": SESSION, "tab_id": TAB, "target_id": TARGET,
        "value": {"description": "instead of label: the option's value", "type": "string"},
      }, "required": ["target_id", "tab_id", "ref"], "type": "object"},
      output: CONTENT,
      run: browser("browser_select"),
    },
    browser_dialog: {
      title: "See or answer a page's dialog",
      reads: false,
      description: "See or answer a web page's own alert, confirm or prompt: inspect gives its text and dialog_id, then accept or dismiss it with that id. On an agent device, accept and dismiss need delivery_mode \"foreground\".",
      input: {"additionalProperties": true, "properties": {
        "action": {"description": "inspect, accept or dismiss", "enum": ["inspect", "accept", "dismiss"], "type": "string"},
        "delivery_mode": FOREGROUND,
        "device": DEVICE,
        "dialog_id": {"description": "the dialog's id, from inspect", "type": "string"},
        "prompt_text": {"description": "what to answer a prompt with, when accepting it", "type": "string"},
        "session": SESSION, "tab_id": TAB, "target_id": TARGET,
      }, "required": ["target_id", "tab_id", "action"], "type": "object"},
      output: CONTENT,
      run: browser("browser_dialog"),
    },
    browser_pointer: {
      title: "Hover, scroll or drag on a web page",
      reads: false,
      description: "Hover, right-click, double-click, scroll or drag on a web page, at a ref or a point. On an agent device give input_route \"dom_event\", or delivery_mode \"foreground\".",
      input: {"additionalProperties": true, "properties": {
        "action": {"description": "hover, right_click, double_click, scroll (with delta_x or delta_y) or drag (to destination_ref, or to_x and to_y)", "enum": ["hover", "right_click", "double_click", "scroll", "drag"], "type": "string"},
        "delivery_mode": FOREGROUND,
        "delta_x": {"description": "scroll: how far right, in CSS pixels (negative for left)", "type": "number"},
        "delta_y": {"description": "scroll: how far down, in CSS pixels (negative for up)", "type": "number"},
        "device": DEVICE,
        "destination_ref": {"description": "drag: the ref to drop on", "type": "string"},
        "input_route": {"default": "trusted", "description": "trusted (the default) acts as a mouse does; dom_event acts in the page itself and needs ref", "enum": ["trusted", "dom_event"], "type": "string"},
        "ref": {"description": "where to start, as a ref from computer_get_browser_state; or give x and y", "type": "string"},
        "session": {"description": "the session you read the page in: required here", "type": "string"},
        "tab_id": TAB, "target_id": TARGET,
        "to_x": {"description": "drag: where to drop, in CSS pixels from the tab's left", "type": "number"},
        "to_y": {"description": "drag: where to drop, in CSS pixels from the tab's top", "type": "number"},
        "x": {"description": "where to start, in CSS pixels from the tab's left", "type": "number"},
        "y": {"description": "where to start, in CSS pixels from the tab's top", "type": "number"},
      }, "required": ["target_id", "tab_id", "session", "action"], "type": "object"},
      output: CONTENT,
      run: browser("browser_pointer"),
    },
    _devices: {
      title: "Your devices agents may use (for the page)",
      description: "The user's devices that agents may use once they let them, each with whether they do and who is using it now.",
      input: { type: "object", properties: {} },
      output: {
        type: "object",
        required: ["devices"],
        properties: {
          devices: { type: "array", items: { type: "object", required: ["device"], additionalProperties: true }, description: "each { device, on, title, kind, using, permissions, missing }, or { device, on: false, why } for one that didn't answer" },
          mirroring: { type: "boolean", description: "whether the iPhone Mirroring lab is on" },
        },
      },
      run: async (ctx) => ({
        devices: (await devices(ctx)).map(({ mirroring, ...d }) => (ctx.labs.mirroring ? { ...d, mirroring } : d)),
        mirroring: ctx.labs.mirroring,
      }),
    },
    _mirror_status: {
      title: "Whether iPhone Mirroring is open",
      lab: "mirroring",
      description: "Whether iPhone Mirroring is open on a Mac, as { running, title, bounds }.",
      input: { type: "object", properties: { device: DEVICE } },
      output: { type: "object", description: "{ running, title, bounds }", additionalProperties: true },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer service runs there.`);
        return ctx.services.computer._mirror_status({}, { device: d });
      },
    },
    _mirror_launch: {
      title: "Open iPhone Mirroring",
      lab: "mirroring",
      description: "Opens iPhone Mirroring on a Mac (pairing happens on the phone).",
      input: { type: "object", properties: { device: DEVICE } },
      output: { type: "object", description: "{ running, title, bounds }", additionalProperties: true },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer service runs there.`);
        return ctx.services.computer._mirror_launch({}, { device: d });
      },
    },
    _mirror_screen: {
      title: "A picture of iPhone Mirroring",
      lab: "mirroring",
      description: "A picture of the iPhone Mirroring window as it is now.",
      input: { type: "object", properties: { device: DEVICE, width: { type: "integer", description: "the picture's width in pixels; default 640, at most 1920" } } },
      output: CONTENT,
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer service runs there.`);
        return ctx.services.computer._mirror_screen({ width: input.width }, { device: d });
      },
    },
    _mirror_use: {
      title: "Run a tool on iPhone Mirroring",
      lab: "mirroring",
      description: "Runs one of a device's tools scoped to the iPhone Mirroring window.",
      input: { type: "object", properties: { device: DEVICE, tool: { type: "string" }, args: { type: "object" } }, required: ["tool"] },
      output: CONTENT,
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer service runs there.`);
        return ctx.services.computer._mirror_use({ tool: input.tool, args: input.args || {} }, { device: d });
      },
    },
    _allow: {
      title: "Let agents use a device",
      description: "Lets agents use one of the user's devices, or stops them. Turning it on asks the user at the device first.",
      input: { type: "object", properties: { device: { type: "string" }, on: { type: "boolean" } }, required: ["device", "on"] },
      output: { type: "object", description: "the device's status, as the computer service gives it", additionalProperties: true },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't use ${input.device} yet: no computer service runs there.`);
        return ctx.services.computer._allow({ on: !!input.on }, { device: d });
      },
    },
  },

  screens: {
    main: {
      title: "Computer Use",
      html: main,
    },
  },
};
