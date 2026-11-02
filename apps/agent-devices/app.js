// Agent Devices: the devices agents use, on the person's servers, which the agent-devices sidecar runs: Linux with a
// browser (Podman, or a Mac's containers), with a screen or without one, and Windows, Android and iPhones where a
// server can run them (the sidecar says which kinds each one can). Its page shows them;
// Computer Use (apps/computer) is what agents use them with. A browser profile a device starts with is a Vault item
// of type Browser profile: the sidecar keeps it as an encrypted blob, and only it, never this app, holds its key,
// which the node fills into the calls this app makes ({{<item id>:_profile_key}}).

/// Runs one of the agent-devices sidecar's operations, wherever it runs for the caller. When none of their servers
/// runs it, says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars["agent-devices"][op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs agent-devices for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Agent Devices needs a server with Podman, or a Mac, and none of yours runs it.";
const TYPE = "Browser profile";

/// The caller's online servers that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.servers()).filter((s) => s.online && (s.sidecars || []).includes("agent-devices")).map((s) => s.name);
}

/// The browser profiles: the Vault items of their type this app is switched on for.
async function profiles(ctx) {
  return (await ctx.vault.items()).filter((i) => i.type === TYPE);
}

/// The browser profile named (its title, or its item's id), or the one titled default when none is named; null for "".
async function profile(ctx, name) {
  if (name === "") return null;
  const all = await profiles(ctx);
  const want = String(name ?? "default").toLowerCase();
  const found = all.find((i) => i.id === name || i.title.toLowerCase() === want);
  if (found) return found;
  if (name === undefined || name === null) return null;
  throw new Error(`No browser profile ${name}: ${all.length ? `there's ${all.map((i) => i.title).join(", ")}` : "there are none yet"}. The person adds them in the Vault.`);
}

/// A profile as the sidecar takes it: its key is a placeholder the node fills in, for the sidecar only.
const given = (p) => ({ item: p.id, name: p.title, key: `{{${p.id}:_profile_key}}`, blob: p.fields._blob?.value || "", salt: p.fields._salt?.value || "" });

const kindArg = { type: "string", enum: ["linux", "windows", "android", "iphone"], description: "linux (with a browser; the default), windows, android or iphone: kinds says which servers run each" };
const deviceArg = { type: "string", description: "the device's name" };

const named = { type: "string", description: "a browser profile's name, like Personal or PTC: a Browser profile in the person's Vault" };

// The page: every agent device as a picture, those in use first, each with its name and one short line.
const PAGE = `
<style>
  body { max-width: 1120px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 22px 14px; margin-top: 6px; }
  .tile { display: block; color: inherit; text-decoration: none; min-width: 0; }
  .tile.opens { cursor: pointer; }
  .pic { position: relative; aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--faint); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  /* A phone's whole screen, upright in the tile, like a phone. */
  .pic img.phone { position: absolute; top: 8px; left: 50%; transform: translateX(-50%); width: auto; height: calc(100% - 16px); box-sizing: border-box; border: 3px solid #1c1c1e; border-radius: 12px; }
  .tile.paused .pic img { filter: grayscale(0.9) brightness(0.5); }
  .tile.opens:hover .pic { border-color: var(--accent); }
  .name { font-weight: 600; margin-top: 8px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--muted); font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 1.3em; }
</style>
<div class="grid" id="grid"><div class="empty" style="grid-column:1/-1">Loading…</div></div>
<script>
  one.title("Agent Devices");
  const grid = document.getElementById("grid");
  const pics = {}; // by server/name: a data URL
  let devices = [];

  const key = (c) => \`\${c.server || ""}/\${c.name}\`;
  const at = (c) => (c.server ? { device: c.name, server: c.server } : { device: c.name });
  const host = (s) => String(s || "").split(".")[0];
  const paused = (c) => c.state !== "running";
  const screenless = (c) => c.borrowed?.no_screen;
  const capital = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  const titles = { windows: "Windows", android: "Android", iphone: "iPhone" };
  const named = (c) => titles[c.name] || capital(c.name);
  const off = (c) => (c.state === "paused" ? "Paused" : "Off");
  const phone = (c) => c.kind === "android" || c.kind === "iphone";
  const glyph = (browser, c) => phone(c)
    ? '<svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="7" y="2.5" width="10" height="19" rx="2"/><path d="M11 18.5h2"/></svg>'
    : browser
    ? '<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="3" y="4.5" width="18" height="15" rx="2"/><path d="M3 8.5h18"/></svg>'
    : '<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';

  // Its name says what it's doing, else what it's called; its line, who's using it, or that it's paused, and where,
  // when the devices are on more than one server.
  function tile(c, servers) {
    const b = c.borrowed;
    const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : named(c);
    const line = [b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : paused(c) ? off(c) : "", servers > 1 ? host(c.server) : ""].filter(Boolean).join(" · ");
    const k = key(c);
    const pic = pics[k] ? \`<img src="\${pics[k]}" alt=""\${phone(c) ? ' class="phone"' : ""}>\` : glyph(screenless(c), c);
    // A phone or Windows opens even when off: it starts for whoever opens it.
    const opens = !screenless(c) && (!paused(c) || c.kind !== "linux");
    return \`<div class="tile\${paused(c) ? " paused" : ""}\${opens ? " opens" : ""}" data-k="\${one.esc(k)}"><div class="pic">\${pic}</div><div class="name">\${one.esc(name)}</div><div class="sub">\${one.esc(line)}</div></div>\`;
  }

  function draw() {
    if (!devices.length) {
      grid.innerHTML = \`<div class="empty" style="grid-column:1/-1">None yet. Agents make one when they need a browser.</div>\`;
      return;
    }
    const servers = new Set(devices.map((c) => c.server)).size;
    const order = (c) => (c.borrowed ? 0 : paused(c) ? 2 : 1);
    const shown = [...devices].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name));
    grid.innerHTML = shown.map((c) => tile(c, servers)).join("");
  }

  // A device with a screen opens to watch and use, here in the app.
  grid.addEventListener("click", (e) => {
    const c = devices.find((c) => key(c) === e.target.closest(".tile.opens")?.dataset.k);
    if (c) one.open("device", at(c));
  });

  // A picture of each running device with a screen.
  async function pictures() {
    await Promise.all(devices.filter((c) => !paused(c) && !screenless(c)).map(async (c) => {
      const k = key(c);
      try {
        const r = await one.call("_screen", { ...at(c), width: phone(c) ? 240 : 480 });
        const img = (r.content || []).find((x) => x.type === "image");
        if (img) pics[k] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
    }));
  }

  async function refresh() {
    try {
      devices = ((await one.call("list", {})).devices || []).filter((c) => !c.spare || c.borrowed);
    } catch (e) {
      grid.innerHTML = \`<div class="empty error" style="grid-column:1/-1">\${one.esc(e.message)}</div>\`;
      return;
    }
    draw();
    await pictures();
    draw();
  }

  let busy = false;
  async function tick() {
    if (busy || document.visibilityState === "hidden") return;
    busy = true;
    try { await refresh(); } finally { busy = false; }
  }
  tick();
  setInterval(tick, 5000);
  document.addEventListener("visibilitychange", tick);
  one.call("_running").then(({ servers, needs }) => {
    if (!servers.length) grid.innerHTML = \`<div class="empty" style="grid-column:1/-1">\${one.esc(needs)}</div>\`;
  });
</script>`;

// One device, to watch and use: its viewer fills the screen, titled with what it's doing, under a line saying whose
// it is, with Done Signing In while it's signing in to a profile.
const DEVICE = `
<style>
  body { max-width: none; padding: 0; }
  [hidden] { display: none !important; }
  .bar { display: flex; align-items: center; gap: 12px; padding: 0 16px; height: 52px; box-sizing: border-box; }
  .bar .what { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .bar .sub { color: var(--muted); }
  iframe { position: fixed; left: 0; right: 0; bottom: 0; top: 0; width: 100%; height: 100%; border: 0; background: #000; color-scheme: dark; }
  .bar:not([hidden]) ~ iframe { top: 52px; height: calc(100% - 52px); }
  .empty { margin: 40px 16px; }
</style>
<div class="bar" id="bar" hidden><div class="what sub" id="what"></div><button class="button primary" id="done" hidden>Done Signing In</button></div>
<div class="empty" id="note">Opening…</div>
<script>
  const at = one.params.server ? { device: one.params.device, server: one.params.server } : { device: one.params.device };
  const bar = document.getElementById("bar"), what = document.getElementById("what"), done = document.getElementById("done"), note = document.getElementById("note");
  let shown = null; // the device as last listed
  let frame = null;

  function say(text, error) {
    frame?.remove();
    frame = null;
    note.hidden = false;
    note.className = "empty" + (error ? " error" : "");
    note.textContent = text;
  }

  function describe(c) {
    const b = c.borrowed;
    const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : c.spare ? "Spare device" : c.name.charAt(0).toUpperCase() + c.name.slice(1);
    const line = b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : "";
    one.title(name);
    what.textContent = line;
    done.hidden = !b?.signing_in || !b.item;
    bar.hidden = !line && done.hidden;
  }

  async function look() {
    try {
      const all = (await one.call("list", {})).devices || [];
      const c = all.find((c) => c.name === at.device && (!at.server || c.server === at.server));
      if (!c) return shown && say("It's been given back.");
      shown = c;
      describe(c);
      if (!frame) await open();
    } catch (e) {
      say(e.message, true);
    }
  }

  async function open() {
    const r = await one.call("_view", { ...at, minutes: 240 });
    const url = typeof r === "string" ? r.split("\\n")[0] : r.url;
    if (!url || frame) return;
    note.hidden = true;
    frame = document.createElement("iframe");
    frame.allow = "clipboard-read; clipboard-write; fullscreen; microphone; keyboard-map";
    frame.src = url;
    document.body.append(frame);
    unzoom();
    frame.focus();
  }

  // The shell zooms its screens to its text size; the device's screen shows as it is, filling the window.
  function unzoom() {
    if (frame) frame.style.zoom = String(1 / (parseFloat(getComputedStyle(document.documentElement).zoom) || 1));
  }
  addEventListener("resize", unzoom);

  done.addEventListener("click", async () => {
    done.disabled = true;
    try {
      await one.call("save_profile", { item: shown.borrowed.item });
      done.hidden = true;
      say(\`Saved: the \${shown.borrowed.profile || "browser"} profile has your sign-ins now.\`);
      shown = null;
      clearInterval(timer);
    } catch (e) {
      say(e.message, true);
    } finally {
      done.disabled = false;
    }
  });

  look();
  const timer = setInterval(look, 5000);
</script>`;

export default {
  name: "agent-devices",
  title: "Agent Devices",
  icon: "browsers",
  color: "#a2845e",
  description: "Devices your agents use: Linux with a browser, Windows, Android and iPhones",
  instructions: "Agent Devices are devices on the person's servers that agents borrow through Computer Use (computer_borrow, computer_*): Linux with a browser (the default kind), and Windows, Android and iPhone (Simulator) where a server can run them, as agent-devices_kinds says. Borrow a Linux one for browsing: it's yours alone and deleted when given back. Windows, Android and iPhone devices are the person's lasting ones (by name, like windows or android), borrowed by one agent at a time, who leaves it as they found it; for anything that changes or might break it, borrow a throwaway copy (copy: true), which is made when the lasting one is busy too if there's room. Every kind takes computer_screen, computer_view and computer_use (computer_tools lists what each kind takes), and agent-devices_open, _install and _run. Browser profiles (like Personal or PTC) are Vault items of type Browser profile: a borrowed device's browser starts signed in to the sites its profile was signed in to. agent-devices_profiles lists them and the sites each was used on. When a site needs the person to sign in, agent-devices_sign_in with profile and url gives a link for them: they sign in there and it's saved when they're done (agent-devices_save_profile, or in the Vault). Never sign in for them. The rest (create, start, stop, resize, backup, restore, move, delete) is for lasting devices the person keeps.",
  uses: { sidecars: ["agent-devices"] },
  // Browser profiles live in the Vault: the person adds one there (add_profile), signs in to sites in it (sign_in, then
  // save_profile), renames and deletes it there.
  vault: {
    profile: { title: TYPE, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { label: "Key", secret: true } }, make: "add_profile", actions: { sign_in: "Sign In to a Site" } },
  },
  schema: {
    user: `create table saves (item text primary key, blob text not null)`,
  },


  ops: {
    borrow: {
      description: "Borrow a device for as long as you need it. Linux (the default kind): one of your own, quick, its browser starting with a copy of a browser profile (the one called default, unless you name one; \"\" for none). Windows, Android or iPhone: the person's lasting device, one agent at a time (others see who has it and what for), or with copy, a throwaway copy of it, made too when the lasting one is busy and its server has room. Give it back with give_back when done; a throwaway one is deleted then, or after 15 minutes without a call, with anything in it.",
      input: { type: "object", required: ["for"], properties: {
        for: { type: "string", description: "what you need it for, in a few words, e.g. Reading Messenger; the person sees it" },
        kind: kindArg,
        device: { type: "string", description: "a lasting device, by name, like windows" },
        copy: { type: "boolean", description: "a throwaway copy, not the lasting device: for anything that changes or might break it" },
        profile: named,
        screen: { type: "boolean", description: "false for a device with no screen to watch, only a browser" },
        by: { type: "string", description: "who's borrowing, e.g. your app's name; default the device you call from" },
      } },
      run: async (ctx, input) => {
        const other = (input.kind && input.kind !== "linux") || input.device;
        const p = other ? null : await profile(ctx, input.profile);
        const pass = Object.fromEntries(["kind", "device", "copy", "by"].filter((k) => input[k] !== undefined).map((k) => [k, input[k]]));
        return sidecar(ctx, "borrow", { for: input.for, ...pass, ...(input.screen === false ? { screen: false } : {}), ...(p ? { profile: given(p) } : {}) });
      },
    },
    give_back: {
      description: "Give back a borrowed device: a Linux or throwaway one is deleted with everything in it, its browser's changes to the profile too; a lasting one stays as it is, for the next agent. A device signing in to a profile saves it first.",
      input: { type: "object", required: ["device"], properties: { device: { type: "string", description: "the device's name" } } },
      run: (ctx, input) => sidecar(ctx, "give_back", input),
    },
    profiles: {
      reads: true,
      description: "The browser profiles devices can start with, like Personal and PTC, each with the sites it was used on and the day of each, and when it was last saved.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await profiles(ctx)).map((p) => ({ profile: p.title, sites: p.fields.sites?.value || "", saved: p.fields._saved?.value || null })),
    },
    sign_in: {
      description: "For the person: opens a page in a browser on a browser profile, on a device with a screen, and gives a link to use it from their own devices, to sign in to sites. It's saved when they're done (save_profile), or when its time is up.",
      input: { type: "object", properties: {
        profile: named,
        item: { type: "string", description: "the profile's Vault item, instead of its name" },
        url: { type: "string", description: "the page to open, e.g. a site's sign-in page" },
        minutes: { type: "integer", description: "how long until it's saved and closed; default 30, at most 240" },
      } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        const r = await sidecar(ctx, "sign_in", { profile: given(p), ...(input.url ? { url: input.url } : {}), ...(input.minutes ? { minutes: input.minutes } : {}) });
        return { ...r, screen: { screen: "device", params: { device: r.device, server: r.server } }, then: { op: "save_profile", title: "Done Signing In" } };
      },
    },
    save_profile: {
      description: "Save a browser profile now: from the device signing in to it, which closes, or with device, from that device's browser. Only when the person asked: borrowed devices start from saved profiles.",
      input: { type: "object", properties: { profile: named, item: { type: "string", description: "the profile's Vault item, instead of its name" }, device: { type: "string", description: "the device's name" } } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        return sidecar(ctx, "save_profile", { profile: given(p), ...(input.device ? { device: input.device } : {}) });
      },
    },
    add_profile: {
      description: "Add a browser profile with no sign-ins yet, as a Vault item, for the person to sign in to sites in with sign_in.",
      input: { type: "object", properties: { profile: { type: "string", description: "its name, like Personal or PTC" }, title: { type: "string", description: "the same, as the Vault gives it" } } },
      run: async (ctx, input) => {
        const title = String(input.profile || input.title || "").trim();
        if (!title) throw new Error("Name it, like Personal or PTC.");
        if ((await profiles(ctx)).some((p) => p.title.toLowerCase() === title.toLowerCase())) throw new Error(`There's already a ${title} profile.`);
        const item = await ctx.vault.create({ title, type: TYPE, hosts: ["agent-devices"], fields: { _profile_key: { generate: true } } });
        return { item, profile: title, text: `Added the ${title} profile. It has no sign-ins yet: sign in to sites in it with sign_in.` };
      },
    },
    _saved: {
      description: "The sidecar's: a browser profile was saved as a new blob, encrypted with a fresh salt, with the sites it was used on.",
      input: { type: "object", required: ["item", "blob", "salt"], properties: { item: { type: "string" }, blob: { type: "string" }, salt: { type: "string" }, sites: { type: "string" } } },
      run: async (ctx, { item, blob, salt, sites }) => {
        const all = await profiles(ctx);
        if (!all.some((p) => p.id === item)) throw new Error("That browser profile was deleted from the Vault.");
        ctx.vault.update(item, { fields: { _blob: { value: blob }, _salt: { value: salt }, _saved: { value: new Date(ctx.now()).toISOString() }, ...(sites ? { sites: { value: sites } } : {}) } });
        // The scope keeps each profile's newest save only, and none of a profile that was deleted.
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
      description: "Create a lasting device and start it, for the person, on the first server in Where Things Run (Settings > Devices) that runs its kind and has room: Linux with a browser of its own (the default), or Android or an iPhone, a new one. Takes a minute or two. Agents borrow instead.",
      input: {"properties": {"device": {"description": "its name: lowercase letters, digits and dashes", "type": "string"}, "kind": kindArg, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["device"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "create", input),
    },
    resize: {
      description: "Change how much CPU and memory a device may use, at once, without restarting it.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["device", "size"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "resize", input),
    },
    start: {
      description: "Start a stopped (or paused) device. It starts again after the server restarts.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}}, "required": ["device"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "start", input),
    },
    stop: {
      description: "Stop a device. Its files and sign-ins stay; it stays stopped until started.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}}, "required": ["device"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "stop", input),
    },
    delete: {
      description: "Delete a device and everything in it, its sign-ins too. Only when the person asked.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}}, "required": ["device"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "delete", input),
    },
    backup: {
      description: "Back up a device's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}}, "required": ["device"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backup", input),
    },
    backups: {
      description: "Every server's backups of devices, newest first.",
      input: {"properties": {"device": {"description": "only this device's", "type": "string"}, "here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backups", input),
    },
    restore: {
      description: "Make a device on this server from a backup: the newest of the device named, from any server, or the one with blob.",
      input: {"properties": {"as": {"description": "the new device's name; default the same", "type": "string"}, "blob": {"description": "a particular backup's blob", "type": "string"}, "device": {"description": "the backed-up device's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "restore", input),
    },
    move: {
      description: "Move a device to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"device": {"description": "the device's name", "type": "string"}, "to": {"description": "the server to move it to", "type": "string"}}, "required": ["device", "to"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "move", input),
    },
    kinds: {
      reads: true,
      description: "The kinds of device each of your servers runs (linux, windows, android, iphone), with what input each kind takes, whether it has room for another and how many lasting ones nobody has borrowed, and how much memory each server has to spare, in GB.",
      input: { type: "object", properties: {} },
      run: (ctx, input) => sidecar(ctx, "kinds", input),
    },
    open: {
      description: "Open a URL in a device's browser: Chrome on Linux, Edge on Windows, the default browser on Android and an iPhone.",
      input: { type: "object", required: ["device", "url"], properties: { device: deviceArg, url: { type: "string" } } },
      run: (ctx, input) => sidecar(ctx, "open", input),
    },
    install: {
      description: "Install an app on a device: a file on its server or a URL (an .apk on Android, an .msi, .msix or .exe on Windows, a simulator .app on an iPhone, a .deb on Linux), or a package's name (a winget id on Windows, an apt package on Linux; on Android it opens the Play Store page).",
      input: { type: "object", required: ["device", "app"], properties: { device: deviceArg, app: { type: "string" } } },
      run: (ctx, input) => sidecar(ctx, "install", input),
    },
    run: {
      description: "Run a command on a device and give its output: PowerShell on Windows, the shell on Android and Linux (as root), simctl spawn on an iPhone.",
      input: { type: "object", required: ["device", "command"], properties: { device: deviceArg, command: { type: "string" } } },
      run: (ctx, input) => sidecar(ctx, "run", input),
    },
    adopt: {
      description: "For the person: make something already on one of their servers a lasting device: a Windows VM's folder (its disk.qcow2, OVMF_VARS.fd, tpm and monitor.sock), an Android Virtual Device, or an iPhone simulator.",
      input: { type: "object", required: ["device", "kind", "on"], properties: {
        device: { type: "string", description: "its name: lowercase letters, digits and dashes" }, kind: kindArg, on: { type: "string", description: "the server it's on" },
        dir: { type: "string", description: "windows: its folder" }, id: { type: "string", description: "android: its AVD's name; iphone: its simulator's UDID" },
        port: { type: "integer", description: "windows: its SSH port on the server; android: its console port, like 5554" }, display: { type: "integer", description: "windows: its VNC display" },
        cpus: { type: "integer", description: "windows: CPUs it starts with" }, gb: { type: "integer", description: "windows: memory it starts with, in GB" },
      } },
      run: (ctx, { on, ...input }) => ctx.sidecars["agent-devices"].adopt({ ...input, here: true }, { server: on }),
    },
    list: {
      reads: true,
      description: "Every server's agent devices: each one's server and kind, whether it's running, paused or stopped, its size, the memory and CPU it uses now, who has borrowed it and what for, and of, for a throwaway copy, the lasting device it's a copy of.",
      input: { type: "object", properties: { here: { type: "boolean", description: "only this server's" } } },
      run: (ctx, input) => sidecar(ctx, "list", input),
    },
    _screen: {
      reads: true,
      description: "For the page: a small picture of a device.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => sidecar(ctx, "screen", input),
    },
    _view: {
      reads: true,
      description: "For the page: a link to watch and use a device.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => sidecar(ctx, "view", input),
    },
    _running: {
      reads: true,
      description: "Which of your servers it runs on, for the page to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ servers: await running(ctx), needs: NEEDS }),
    },
  },

  screens: {
    main: {
      title: "Agent Devices",
      html: PAGE,
    },
    device: {
      title: "Device",
      html: DEVICE,
    },
  },
};
