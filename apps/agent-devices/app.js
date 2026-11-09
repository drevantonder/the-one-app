// Agent Devices: the devices agents use, on the user's own devices, which the agent-devices service runs: Linux with a
// browser (Podman, or a Mac's containers), with a screen or without one, and Windows, Android and iPhones where a
// device can run them (the service says which kinds each one can). Its page shows them;
// Computer Use (apps/computer) is what agents use them with. A browser profile a device starts with is a Vault item
// of type Browser profile: the service keeps it as an encrypted blob, and only it, never this app, holds its key,
// which the node fills into the calls this app makes ({{<item id>:_profile_key}}). Its screens are React, in screens/,
// built into dist/ when it's deployed (one deploy apps/agent-devices).

import mainHtml from "./dist/main.html" with { type: "text" };
import deviceHtml from "./dist/device.html" with { type: "text" };
import settingsHtml from "./dist/settings.html" with { type: "text" };

const service = (ctx, fn, input, on) => ctx.services["agent-devices"][fn](input, on ? { device: on } : undefined);

function fnv(s) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
  return (h >>> 0).toString(36);
}

/// The service's functions Computer Use acts on agent devices with (`_act`).
const ACTS = ["list", "view", "screen", "tools", "refresh_tools", "use", "get_browser_state", "browser_prepare", "browser_navigate", "browser_click", "browser_type", "browser_select", "browser_dialog", "browser_pointer"];
const TYPE = "Browser profile";

async function profiles(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === TYPE);
}

async function profile(ctx, name) {
  if (name === "") return null;
  const all = await profiles(ctx);
  const want = String(name ?? "default").toLowerCase();
  const found = all.find((i) => i.id === name || i.title.toLowerCase() === want);
  if (found) return found;
  if (name === undefined || name === null) return null;
  throw Object.assign(new Error(`No browser profile ${name}: ${all.length ? `there's ${all.map((i) => i.title).join(", ")}` : "there are none yet"}. The user adds them in the Vault.`), { code: "no_profile" });
}

/// A profile as the service takes it: its key is a placeholder the node fills in, for the service only.
const given = (p) => ({ item: p.id, name: p.title, key: `{{${p.id}:_profile_key}}`, blob: p.fields._blob?.value || "", salt: p.fields._salt?.value || "" });

const kindArg = { type: "string", enum: ["linux", "chrome", "windows", "android", "iphone"], description: "linux (with a browser, in a container; the default), chrome (Chrome straight on one of your devices: quicker, where its owner allows it), windows, android or iphone: kinds says which of your devices run each" };
const deviceArg = { type: "string", description: "an agent device's name, from computer_list, like pool-745f3e or windows" };
const sizeArg = { type: "string", enum: ["small", "medium", "large"], description: "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small" };

const named = { type: "string", description: "a browser profile's name, like Personal or Work: a Browser profile in the user's Vault" };

// A backup of a device's home folder, as the service keeps it.
const BACKUP = {
  type: "object",
  required: ["device", "on", "blob", "bytes", "size", "at"],
  properties: {
    device: { type: "string" },
    on: { type: "string", description: "the device of yours that keeps it" },
    blob: { type: "string", description: "for agent-devices_restore" },
    bytes: { type: "integer" },
    size: { type: "string", description: "the device's size" },
    at: { type: "string", description: "ISO 8601" },
  },
};

export default {
  name: "agent-devices",
  title: "Agent Devices",
  tagline: "Devices for AI",
  icon: "browsers",
  color: "#a2845e",
  advanced: true,
  description: "Devices your agents use: Linux with a browser, Windows, Android and iPhones",
  needs: "Needs a Linux device with Podman, or a Mac",
  instructions: "Agent Devices are devices, on the user's own devices, that agents borrow (agent-devices_borrow, then agent-devices_give_back) and use through Computer Use (computer_*): Linux with a browser (the default kind), and Windows, Android and iPhone (Simulator) where one of their devices can run them, as agent-devices_kinds says. Borrow a Linux one for browsing: it's yours alone and deleted when given back. Where kinds lists chrome as available, kind chrome is quicker (under a second) and lighter: the device's own Chrome, without a container, whose borrow gives target_id and tab_id for the browser tools at once, with no browser_prepare or list_windows; its owner allows it in Agent Devices' Settings. Windows, Android and iPhone devices are the user's lasting ones (by name, like windows or android), borrowed by one agent at a time, who leaves it as they found it; for anything that changes or might break it, borrow a throwaway copy (copy: true), which is made when the lasting one is busy too if there's room. Every kind takes computer_screen, computer_view and computer_use (computer_tools lists what each kind takes), and agent-devices_open, _install and _run. Browser profiles (like Personal or Work) are Vault items of type Browser profile: a borrowed device's browser starts signed in to the sites its profile was signed in to. agent-devices_profiles lists them and the sites each was used on. When a site needs the user to sign in, agent-devices_sign_in with profile and url gives a link for them: they sign in there and it's saved when they're done (agent-devices_save_profile, or in the Vault). Never sign in for them. The rest (create, start, stop, resize, backup, restore, move, delete) is for lasting devices the user keeps.",
  uses: { services: ["agent-devices"] },
  service: { network: ["*"], user: true, platforms: ["linux", "mac"] },
  vault: {
    profile: { title: TYPE, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { label: "Key", secret: true } }, make: "add_profile", actions: { sign_in: "Sign In to a Site" } },
  },
  schema: {
    user: `create table saves (item text primary key, blob text not null)`,
  },


  tools: {
    borrow: {
      title: "Borrow an agent device",
      reads: false,
      description: "Get a device to use with computer_* for as long as you need it, and its name. Linux (the default kind): one of your own, ready at once, its browser starting with a copy of a browser profile (the one called default, unless you name one; \"\" for none). Windows, Android or iPhone: the user's lasting device, one agent at a time, or with copy, a throwaway copy of it. Give it back with agent-devices_give_back when done; a throwaway one is deleted then, or after 15 minutes without a call, with anything in it.",
      input: { type: "object", required: ["for"], properties: {
        for: { type: "string", description: "what you need it for, in a few words, e.g. Reading Messenger; the user sees it" },
        kind: kindArg,
        device: { type: "string", description: "a lasting device's name, from computer_list, like windows; default the kind's first" },
        copy: { type: "boolean", description: "a throwaway copy, not the lasting device: for anything that changes or might break it" },
        profile: named,
        screen: { type: "boolean", description: "false for a device with no screen to watch, only a browser" },
        by: { type: "string", description: "who's borrowing, as others see it, like your app's name; default the device you call from" },
      } },
      output: {
        type: "object",
        required: ["device", "on", "text"],
        properties: {
          device: { type: "string", description: "the name to pass to the other tools" },
          on: { type: "string", description: "the device of yours it runs on" },
          profile: { type: "string", description: "the browser profile its browser starts with" },
          kind: { type: "string", description: "for a kind other than linux" },
          copy: { type: "boolean", description: "for a kind other than linux: whether it's a throwaway copy" },
          text: { type: "string", description: "what to do next, in words" },
        },
      },
      run: async (ctx, input) => {
        const other = (input.kind && input.kind !== "linux") || input.device;
        const p = other ? null : await profile(ctx, input.profile);
        const pass = Object.fromEntries(["kind", "device", "copy", "by"].filter((k) => input[k] !== undefined).map((k) => [k, input[k]]));
        return service(ctx, "borrow", { for: input.for, ...pass, ...(input.screen === false ? { screen: false } : {}), ...(p ? { profile: given(p) } : {}) });
      },
    },
    give_back: {
      title: "Give back a borrowed device",
      reads: false,
      description: "Give back a device you borrowed: a Linux or throwaway one is deleted with everything in it, its browser's changes to the profile too; a lasting one stays as it is, for the next agent. A device signing in to a profile saves it first.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the name agent-devices_borrow gave" } } },
      output: { type: "string", description: "that it's given back, in words" },
      run: (ctx, input) => service(ctx, "give_back", input),
    },
    profiles: {
      title: "Browser profiles",
      reads: true,
      description: "The browser profiles a borrowed device's browser can start with, like Personal and Work: each one's name, the sites it was used on and the day of each, and when it was last saved.",
      input: { type: "object", properties: {} },
      output: {
        type: "array",
        items: {
          type: "object",
          required: ["profile", "sites"],
          properties: {
            profile: { type: "string", description: "its name" },
            sites: { type: "string", description: "the sites it was used on and the day of each" },
            saved: { type: "string", description: "when it was last saved, ISO 8601; absent if never" },
          },
        },
      },
      run: async (ctx) => (await profiles(ctx)).map((p) => ({ profile: p.title, sites: p.fields.sites?.value || "", saved: p.fields._saved?.value || null })),
    },
    sign_in: {
      title: "Sign in to sites in a browser profile",
      reads: false,
      description: "For the user: opens a page in a browser on a browser profile, on a device with a screen, and gives a link for them to sign in to sites there themselves. Their sign-ins are saved when they're done (agent-devices_save_profile), or when its time is up.",
      input: { type: "object", properties: {
        profile: named,
        item: { type: "string", description: "the profile's Vault item, instead of its name" },
        url: { type: "string", description: "the page to open, like https://www.facebook.com/login" },
        minutes: { type: "integer", description: "how long until it's saved and closed; default 30, at most 240" },
      } },
      output: {
        type: "object",
        required: ["device", "on", "profile", "url", "until", "text", "screen", "then"],
        properties: {
          device: { type: "string", description: "the agent device the browser is on" },
          on: { type: "string", description: "the device of yours it runs on" },
          profile: { type: "string" },
          url: { type: "string", description: "the link for the user to sign in at" },
          until: { type: "string", description: "when it's saved and closed, ISO 8601" },
          text: { type: "string", description: "what to tell the user, in words" },
          screen: { type: "object", description: "the screen of this app that shows the device", additionalProperties: true },
          then: { type: "object", description: "the tool that saves it, { tool, title }", additionalProperties: true },
        },
      },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        const r = await service(ctx, "sign_in", { profile: given(p), ...(input.url ? { url: input.url } : {}), ...(input.minutes ? { minutes: input.minutes } : {}) });
        return { ...r, screen: { screen: "device", params: { device: r.device, on: r.on } }, then: { tool: "save_profile", title: "Done Signing In" } };
      },
    },
    save_profile: {
      title: "Save a browser profile's sign-ins",
      reads: false,
      description: "Save a browser profile's sign-ins now: from the device signing in to it, which closes, or with device, from that device's browser. Only when the user asked: borrowed devices start from saved profiles.",
      input: { type: "object", properties: { profile: named, item: { type: "string", description: "the profile's Vault item, instead of its name" }, device: deviceArg } },
      output: { type: "string", description: "that it's saved, in words" },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        return service(ctx, "save_profile", { profile: given(p), ...(input.device ? { device: input.device } : {}) });
      },
    },
    add_profile: {
      title: "Add a browser profile",
      reads: false,
      description: "Add a browser profile with no sign-ins yet, as a Vault item, for the user to sign in to sites in with agent-devices_sign_in.",
      input: { type: "object", properties: { profile: { type: "string", description: "its name, like Personal or Work" }, title: { type: "string", description: "the same, as the Vault gives it" } } },
      output: {
        type: "object",
        required: ["item", "profile", "text"],
        properties: { item: { type: "string", description: "its Vault item" }, profile: { type: "string" }, text: { type: "string", description: "in words" } },
      },
      run: async (ctx, input) => {
        const title = String(input.profile || input.title || "").trim();
        if (!title) throw new Error("Name it, like Personal or Work.");
        if ((await profiles(ctx)).some((p) => p.title.toLowerCase() === title.toLowerCase())) throw new Error(`There's already a ${title} profile.`);
        const item = await ctx.vault.create({ title, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { generate: true } } });
        return { item, profile: title, text: `Added the ${title} profile. It has no sign-ins yet: sign in to sites in it with sign_in.` };
      },
    },
    _saved: {
      title: "A browser profile was saved (for the service)",
      reads: false,
      description: "The service's: a browser profile was saved as a new blob, encrypted with a fresh salt, with the sites it was used on.",
      input: { type: "object", required: ["item", "blob", "salt"], properties: { item: { type: "string" }, blob: { type: "string" }, salt: { type: "string" }, sites: { type: "string" } } },
      output: { type: "string", description: "Saved." },
      run: async (ctx, { item, blob, salt, sites }) => {
        const all = await profiles(ctx);
        if (!all.some((p) => p.id === item)) throw new Error("That browser profile was deleted from the Vault.");
        ctx.vault.update(item, { fields: { _blob: { value: blob }, _salt: { value: salt }, _saved: { value: new Date(ctx.now()).toISOString() }, ...(sites ? { sites: { value: sites } } : {}) } });
        const kept = await ctx.user.query("select item, blob from saves");
        for (const s of kept) {
          if (s.item === item ? s.blob !== blob : !all.some((p) => p.id === s.item)) ctx.user.forget(s.blob);
          if (s.item !== item && !all.some((p) => p.id === s.item)) ctx.user.exec("delete from saves where item = ?", [s.item]);
        }
        // Its last save, as its item names it, even one this table doesn't hold.
        const was = all.find((p) => p.id === item).fields._blob?.value;
        if (was && was !== blob && !kept.some((s) => s.blob === was)) ctx.user.forget(was);
        ctx.user.exec("insert or replace into saves values (?, ?)", [item, blob], [blob]);
        return "Saved.";
      },
    },
    create: {
      title: "Make a lasting device",
      reads: false,
      description: "Create a lasting device and start it, for the user, on the first of their devices in run priority (device-activity_order) that runs its kind and has room: Linux with a browser of its own (the default), Android or an iPhone. Takes a minute or two. Agents borrow instead.",
      input: {"properties": {"device": {"description": "its name: lowercase letters, digits and dashes, like work-browser", "type": "string"}, "kind": kindArg, "size": sizeArg}, "required": ["device"], "type": "object"},
      output: { type: "string", description: "that it's made, in words" },
      run: (ctx, input) => service(ctx, "create", input),
    },
    resize: {
      title: "Resize a lasting device",
      reads: false,
      description: "Change how much CPU and memory a lasting device may use, at once, without restarting it.",
      input: {"properties": {"device": deviceArg, "size": sizeArg}, "required": ["device", "size"], "type": "object"},
      output: { type: "string", description: "its size now, in words" },
      run: (ctx, input) => service(ctx, "resize", input),
    },
    start: {
      title: "Start a lasting device",
      reads: false,
      description: "Start a stopped (or paused) lasting device. It starts again after the device it's on restarts.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      output: { type: "string", description: "that it's started, in words" },
      run: (ctx, input) => service(ctx, "start", input),
    },
    stop: {
      title: "Stop a lasting device",
      reads: false,
      description: "Stop a lasting device. Its files and sign-ins stay; it stays stopped until agent-devices_start.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      output: { type: "string", description: "that it's stopped, in words" },
      run: (ctx, input) => service(ctx, "stop", input),
    },
    delete: {
      title: "Delete a lasting device",
      reads: false,
      confirm: true,
      description: "Delete a lasting device and everything in it, its sign-ins too, for good (agent-devices_backup first keeps a copy). Only when the user asked. A borrowed one goes with agent-devices_give_back.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      output: { type: "string", description: "that it's deleted, in words" },
      run: (ctx, input) => service(ctx, "delete", input),
    },
    backup: {
      title: "Back up a device",
      reads: false,
      description: "Back up a device's home folder (its files and sign-ins, without caches) where any of your devices can fetch it, keeping the newest 5. agent-devices_restore makes a device from one.",
      input: {"properties": {"device": deviceArg}, "required": ["device"], "type": "object"},
      output: {
        type: "object",
        required: ["backup", "text"],
        properties: { backup: BACKUP, text: { type: "string", description: "in words" } },
      },
      run: (ctx, input) => service(ctx, "backup", input),
    },
    backups: {
      title: "Backups of agent devices",
      reads: true,
      description: "Backups of agent devices, from all of your devices, newest first: each one's device, when (at, ISO 8601), its bytes, the device's size, and its blob (for agent-devices_restore).",
      input: {"properties": {"device": {"description": "only this agent device's backups, by its name", "type": "string"}, "here": {"description": "only the backups kept on the device you call from", "type": "boolean"}}, "type": "object"},
      output: {
        type: "object",
        required: ["backups"],
        properties: {
          backups: { type: "array", items: BACKUP, description: "newest first" },
          unreachable: { description: "the devices that couldn't be asked, and why" },
        },
      },
      run: (ctx, input) => service(ctx, "backups", input),
    },
    restore: {
      title: "Make a device from a backup",
      reads: false,
      description: "Make a new agent device, on the device you call from, from a backup: the newest of the device named, or the one with blob. It never replaces a device that's there: give as for another name.",
      input: {"properties": {"as": {"description": "the new device's name; default the backed-up one's", "type": "string"}, "blob": {"description": "a particular backup's blob, from agent-devices_backups", "type": "string"}, "device": {"description": "the backed-up device's name, from agent-devices_backups", "type": "string"}, "size": sizeArg}, "type": "object"},
      output: { type: "string", description: "that it's made, in words" },
      run: (ctx, input) => service(ctx, "restore", input),
    },
    move: {
      title: "Move a lasting device",
      reads: false,
      description: "Move a lasting device to another of your devices: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"device": deviceArg, "to": {"description": "one of your devices, by name, from agent-devices_kinds", "type": "string"}}, "required": ["device", "to"], "type": "object"},
      output: { type: "string", description: "that it's moved, in words" },
      run: (ctx, input) => service(ctx, "move", input),
    },
    kinds: {
      title: "Which kinds of device each of yours runs",
      reads: true,
      description: "Which kinds of agent device each of your devices runs (linux, chrome, windows, android, iphone, as agent-devices_borrow's kind takes them; one it can't says why in unavailable): what input each kind takes, whether there's room for another, how many lasting ones nobody has borrowed, and the device's memory to spare (free_gb, in GB).",
      input: { type: "object", properties: {} },
      output: {
        type: "object",
        description: "{ devices } for every device; one device's own answer with here",
        properties: {
          devices: {
            type: "array",
            items: {
              type: "object",
              required: ["device", "kinds"],
              properties: {
                device: { type: "string" },
                kinds: { type: "array", items: { type: "object", required: ["kind", "title", "room"], additionalProperties: true }, description: "each { kind, title, room, input, unavailable, unborrowed, copies }" },
                free_gb: { type: "integer", description: "memory free, in GB" },
              },
            },
          },
          unreachable: { description: "the devices that couldn't be asked, and why" },
        },
        additionalProperties: true,
      },
      run: (ctx, input) => service(ctx, "kinds", input),
    },
    open: {
      title: "Open a web address on a device",
      reads: false,
      description: "Open a web address in a device's browser: Chrome on Linux, Edge on Windows, the default browser on Android and an iPhone. To read and act on the page on Linux, use computer_browser_prepare and computer_get_browser_state.",
      input: { type: "object", required: ["device", "url"], properties: { device: deviceArg, url: { type: "string", description: "the address, like https://example.com" } } },
      output: { type: "string", description: "that it's open, in words" },
      run: (ctx, input) => service(ctx, "open", input),
    },
    install: {
      title: "Install an app on a device",
      reads: false,
      description: "Install an app on a device, and say how it went: a file on the device it's on, a URL, or a package's name.",
      input: { type: "object", required: ["device", "app"], properties: { device: deviceArg, app: { type: "string", description: "a file's path on the device it's on, or a URL, to an .apk (Android), .msi, .msix or .exe (Windows), a simulator .app (iPhone) or a .deb (Linux); or a package's name: a winget id like Mozilla.Firefox, an apt package, or an Android package like com.whatsapp (it opens its Play Store page)" } } },
      output: { type: "string", description: "how it went, in words" },
      run: (ctx, input) => service(ctx, "install", input),
    },
    run: {
      title: "Run a command on an agent device",
      reads: false,
      description: "Run a command on an agent device and give its output. For a command on one of your own devices, shell_run.",
      input: { type: "object", required: ["device", "command"], properties: { device: deviceArg, command: { type: "string", description: "PowerShell on Windows, like Get-Process; a shell command on Android and Linux (as root), like ls /sdcard; on an iPhone, a program in the simulator, as simctl spawn takes it" } } },
      output: { type: "string", description: "what it printed" },
      run: (ctx, input) => service(ctx, "run", input),
    },
    reach: {
      title: "Let a device reach a port",
      reads: false,
      description: "Let a Windows, Android or iPhone device reach a port on the device it's on, at the same port on its own 127.0.0.1, until it stops: for an app on it to reach a server you run there, like a throwaway network.",
      input: { type: "object", required: ["device", "port"], properties: { device: deviceArg, port: { type: "integer", description: "the port your server listens on, on the device the agent device is on, like 4791" } } },
      output: { type: "string", description: "which port it reaches, in words" },
      run: (ctx, input) => service(ctx, "reach", input),
    },
    adopt: {
      title: "Make something a lasting device",
      reads: false,
      description: "For the user: make something already on one of their devices a lasting device: a Windows VM's folder with its disk, an Android Virtual Device, or an iPhone simulator.",
      input: { type: "object", required: ["device", "kind", "on"], properties: {
        device: { type: "string", description: "its name: lowercase letters, digits and dashes, like windows" }, kind: kindArg, on: { type: "string", description: "one of your devices, by name, where it is" },
        dir: { type: "string", description: "windows: its folder" }, id: { type: "string", description: "android: its AVD's name; iphone: its simulator's UDID" },
        port: { type: "integer", description: "windows: its SSH port on the device it's on; android: its console port, like 5554" }, display: { type: "integer", description: "windows: its VNC display" },
        cpus: { type: "integer", description: "windows: CPUs it starts with" }, gb: { type: "integer", description: "windows: memory it starts with, in GB" },
      } },
      output: { type: "string", description: "that it's a lasting device now, in words" },
      run: (ctx, { on, ...input }) => ctx.services["agent-devices"].adopt({ ...input, here: true }, { device: on }),
    },
    _list: {
      title: "The agent devices (for the page)",
      reads: true,
      description: "For the page: the agent devices on all of your devices, as the services last knew them: the device each is on, its kind, whether it's running, who has borrowed it and what for. None if it's the one the page has (have, its hash).",
      input: { type: "object", properties: { have: { type: "string" } } },
      output: {
        type: "object",
        required: ["hash"],
        properties: {
          same: { type: "boolean", description: "true when the page already has this hash" },
          hash: { type: "string" },
          devices: { type: "array", items: { type: "object", required: ["name", "on"], additionalProperties: true } },
          unreachable: { description: "the devices that couldn't be asked, and why" },
        },
      },
      run: async (ctx, { have }) => {
        const r = await service(ctx, "list", { kept: true });
        const devices = (r.devices || []).map(({ name, on, kind, state, borrowed, spare, of }) => ({ name, on, kind, state, borrowed, spare, of }));
        const hash = fnv(JSON.stringify([devices, r.unreachable]));
        return have === hash ? { same: true, hash } : { devices, unreachable: r.unreachable, hash };
      },
    },
    _screen: {
      title: "A device's picture (for the page)",
      reads: true,
      description: "For the page: a small picture of a device, the one the device it's on last took, or none if it's the one the page has (have, its hash).",
      input: { type: "object", additionalProperties: true },
      output: { type: "object", description: "{ picture, hash }, or { same } when the page has it", additionalProperties: true },
      run: (ctx, input) => service(ctx, "screen", { ...input, kept: true }, input.on),
    },
    _view: {
      title: "A link to watch a device (for the page)",
      reads: true,
      description: "For the page: a link to watch and use a device.",
      input: { type: "object", additionalProperties: true },
      output: { type: "string", description: "the link, and how long it works" },
      run: (ctx, input) => service(ctx, "view", input),
    },
    _chrome: {
      title: "Where agents may borrow Chrome (for Settings)",
      reads: true,
      description: "For Settings: each of your devices running Agent Devices, and whether agents may borrow Chrome run straight on it.",
      input: { type: "object", properties: {} },
      output: {
        type: "object",
        required: ["devices"],
        properties: { devices: { type: "array", items: { type: "object", required: ["on", "allowed", "can"], additionalProperties: true } } },
      },
      run: async (ctx) => {
        const r = await service(ctx, "chrome", {});
        return { devices: (r.devices || []).map(({ on, allowed, can }) => ({ on, allowed, can })).sort((a, b) => a.on.localeCompare(b.on)) };
      },
    },
    _set_chrome: {
      title: "Allow Chrome on a device (for Settings)",
      reads: false,
      description: "For Settings: lets agents borrow Chrome run straight on one of your devices, or stops them.",
      input: { type: "object", required: ["on", "allowed"], properties: { on: { type: "string" }, allowed: { type: "boolean" } } },
      output: { type: "object", description: "the device's row, { on, allowed, can }, and given_back (how many borrowed Chrome devices it gave back) when switched off", additionalProperties: true },
      run: (ctx, { on, allowed }) => service(ctx, "set_chrome", { on, allowed }),
    },
    _act: {
      title: "Act on an agent device (for Computer Use)",
      reads: false,
      description: "For Computer Use (apps/computer), which agents act on agent devices with: runs one of the service's functions it acts with (list, view, screen, tools, refresh_tools, use, get_browser_state and the browser tools) with its input.",
      input: { type: "object", required: ["function"], properties: { function: { type: "string", enum: ACTS }, input: { type: "object" } } },
      output: { description: "what the service function gave" },
      run: (ctx, { function: fn, input = {} }) => {
        if (!ACTS.includes(fn)) throw new Error(`Computer Use doesn't act on agent devices with ${fn}: only ${ACTS.join(", ")}`);
        return service(ctx, fn, input);
      },
    },
  },

  screens: {
    main: {
      title: "Agent Devices",
      html: mainHtml,
    },
    device: {
      title: "Device",
      html: deviceHtml,
    },
    settings: {
      title: "Agent Devices",
      html: settingsHtml,
    },
  },
};
