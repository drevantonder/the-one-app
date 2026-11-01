// Agent Computers: the computers agents use, each a Linux computer with a browser on the person's servers, with a
// screen or without one, which the agent-computers sidecar runs (Incus, or a Mac's containers). Its page shows them;
// Computer Use (apps/computer) is what agents use them with. A browser profile a computer starts with is a Vault item
// of type Browser profile: the sidecar keeps it as an encrypted blob, and only it, never this app, holds its key,
// which the node fills into the calls this app makes ({{<item id>:_profile_key}}).

/// Runs one of the agent-computers sidecar's operations, wherever it runs for the caller. When none of their servers
/// runs it, says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars["agent-computers"][op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs agent-computers for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Agent Computers needs a Linux server with Incus, or a Mac, and none of yours runs it.";
const TYPE = "Browser profile";

/// The caller's online servers that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.servers()).filter((s) => s.online && (s.sidecars || []).includes("agent-computers")).map((s) => s.name);
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

const named = { type: "string", description: "a browser profile's name, like Personal or PTC: a Browser profile in the person's Vault" };

// The page: every agent computer as a picture, those in use first, each with its name and one short line.
const PAGE = `
<style>
  body { max-width: 1120px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 22px 14px; margin-top: 6px; }
  .tile { display: block; color: inherit; text-decoration: none; min-width: 0; }
  .pic { aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--faint); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .tile.paused .pic img { filter: grayscale(0.9) brightness(0.5); }
  a.tile:hover .pic { border-color: var(--accent); }
  .name { font-weight: 600; margin-top: 8px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sub { color: var(--muted); font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 1.3em; }
</style>
<div class="grid" id="grid"><div class="empty" style="grid-column:1/-1">Loading…</div></div>
<script>
  one.title("Agent Computers");
  const grid = document.getElementById("grid");
  const pics = {}; // by server/name: a data URL
  const links = {}; // by server/name: { url, until }
  let computers = [];

  const key = (c) => \`\${c.server || ""}/\${c.name}\`;
  const at = (c) => (c.server ? { computer: c.name, server: c.server } : { computer: c.name });
  const host = (s) => String(s || "").split(".")[0];
  const paused = (c) => c.state !== "running";
  const screenless = (c) => c.borrowed?.no_screen;
  const capital = (s) => s.charAt(0).toUpperCase() + s.slice(1);
  const glyph = (browser) => browser
    ? '<svg width="44" height="44" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="3" y="4.5" width="18" height="15" rx="2"/><path d="M3 8.5h18"/></svg>'
    : '<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';

  // Its name says what it's doing, else what it's called; its line, who's using it, or that it's paused, and where,
  // when the computers are on more than one server.
  function tile(c, servers) {
    const b = c.borrowed;
    const name = b?.signing_in ? "Signing in to sites" : b ? b.for || "In use" : capital(c.name);
    const line = [b ? (b.signing_in ? b.profile || "" : b.by || "An agent") : paused(c) ? "Paused" : "", servers > 1 ? host(c.server) : ""].filter(Boolean).join(" · ");
    const k = key(c);
    const pic = pics[k] ? \`<img src="\${pics[k]}" alt="">\` : glyph(screenless(c));
    const tag = links[k] && !paused(c) ? \`a href="\${one.esc(links[k].url)}" target="_blank" rel="noopener"\` : "div";
    return \`<\${tag} class="tile\${paused(c) ? " paused" : ""}" title="\${one.esc(c.name)}"><div class="pic">\${pic}</div><div class="name">\${one.esc(name)}</div><div class="sub">\${one.esc(line)}</div></\${tag.split(" ")[0]}>\`;
  }

  function draw() {
    if (!computers.length) {
      grid.innerHTML = \`<div class="empty" style="grid-column:1/-1">None yet. Agents make one when they need a browser.</div>\`;
      return;
    }
    const servers = new Set(computers.map((c) => c.server)).size;
    const order = (c) => (c.borrowed ? 0 : paused(c) ? 2 : 1);
    const shown = [...computers].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name));
    grid.innerHTML = shown.map((c) => tile(c, servers)).join("");
  }

  // A picture of each running computer with a screen, and a link to open it, kept until shortly before it stops working.
  async function pictures() {
    await Promise.all(computers.filter((c) => !paused(c) && !screenless(c)).map(async (c) => {
      const k = key(c);
      try {
        const r = await one.call("_screen", { ...at(c), width: 480 });
        const img = (r.content || []).find((x) => x.type === "image");
        if (img) pics[k] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
      if (!links[k] || links[k].until < Date.now()) {
        try {
          const r = await one.call("_view", { ...at(c), minutes: 60 });
          const url = typeof r === "string" ? r.split("\\n")[0] : r.url;
          if (url) links[k] = { url, until: Date.now() + 50 * 60_000 };
        } catch {}
      }
    }));
  }

  async function refresh() {
    try {
      computers = ((await one.call("list", {})).computers || []).filter((c) => !c.spare || c.borrowed);
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

export default {
  name: "agent-computers",
  title: "Agent Computers",
  icon: "macwindow.on.rectangle",
  color: "#a2845e",
  description: "Computers your agents use, each with a browser",
  instructions: "Agent Computers are Linux computers on the person's servers, each with a browser, that agents borrow through Computer Use (computer_borrow, computer_*). Browser profiles (like Personal or PTC) are Vault items of type Browser profile: a borrowed computer's browser starts signed in to the sites its profile was signed in to. agent-computers_profiles lists them and the sites each was used on. When a site needs the person to sign in, agent-computers_sign_in with profile and url gives a link for them: they sign in there and it's saved when they're done (agent-computers_save_profile, or in the Vault). Never sign in for them. The rest (create, start, stop, resize, backup, restore, move, delete) is for lasting computers the person keeps.",
  uses: { sidecars: ["agent-computers"] },
  // Browser profiles live in the Vault: the person adds one there (add_profile), signs in to sites in it (sign_in, then
  // save_profile), renames and deletes it there.
  vault: {
    profile: { title: TYPE, type: TYPE, hosts: ["agent-computers"], fields: { _profile_key: { label: "Key", secret: true } }, make: "add_profile", actions: { sign_in: "Sign In to a Site" } },
  },
  schema: {
    user: `create table saves (item text primary key, blob text not null)`,
  },

  on: {
    // A profile's newest save, whose blob this scope keeps.
    saved(db, e) {
      db.exec("insert or replace into saves values (?, ?)", [e.data.item, e.data.blob]);
    },
    // A profile deleted from the Vault: its save goes.
    dropped(db, e) {
      db.exec("delete from saves where item = ?", [e.data.item]);
    },
  },

  ops: {
    borrow: {
      description: "Borrow a computer of your own for as long as you need a browser, its browser starting with a copy of a browser profile (the one called default, unless you name one; \"\" for none). Quick: a spare is kept ready. Give it back with give_back when done; it's deleted then, or after 15 minutes without a call, with anything in it.",
      input: { type: "object", required: ["for"], properties: {
        for: { type: "string", description: "what you need it for, in a few words, e.g. Reading Messenger; the person sees it" },
        profile: named,
        screen: { type: "boolean", description: "false for a computer with no screen to watch, only a browser" },
        by: { type: "string", description: "who's borrowing, e.g. your app's name; default the device you call from" },
      } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.profile);
        return sidecar(ctx, "borrow", { for: input.for, ...(input.by ? { by: input.by } : {}), ...(input.screen === false ? { screen: false } : {}), ...(p ? { profile: given(p) } : {}) });
      },
    },
    give_back: {
      description: "Give back a borrowed computer: it's deleted with everything in it, its browser's changes to the profile too. A computer signing in to a profile saves it first.",
      input: { type: "object", required: ["computer"], properties: { computer: { type: "string", description: "the computer's name" } } },
      run: (ctx, input) => sidecar(ctx, "give_back", input),
    },
    profiles: {
      reads: true,
      description: "The browser profiles computers can start with, like Personal and PTC, each with the sites it was used on and the day of each, and when it was last saved.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await profiles(ctx)).map((p) => ({ profile: p.title, sites: p.fields.sites?.value || "", saved: p.fields._saved?.value || null })),
    },
    sign_in: {
      description: "For the person: opens a page in a browser on a browser profile, on a computer with a screen, and gives a link to use it from their own devices, to sign in to sites. It's saved when they're done (save_profile), or when its time is up.",
      input: { type: "object", properties: {
        profile: named,
        item: { type: "string", description: "the profile's Vault item, instead of its name" },
        url: { type: "string", description: "the page to open, e.g. a site's sign-in page" },
        minutes: { type: "integer", description: "how long until it's saved and closed; default 30, at most 240" },
      } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        const r = await sidecar(ctx, "sign_in", { profile: given(p), ...(input.url ? { url: input.url } : {}), ...(input.minutes ? { minutes: input.minutes } : {}) });
        return { ...r, then: { op: "save_profile", title: "Done Signing In" } };
      },
    },
    save_profile: {
      description: "Save a browser profile now: from the computer signing in to it, which closes, or with computer, from that computer's browser. Only when the person asked: borrowed computers start from saved profiles.",
      input: { type: "object", properties: { profile: named, item: { type: "string", description: "the profile's Vault item, instead of its name" }, computer: { type: "string", description: "the computer's name" } } },
      run: async (ctx, input) => {
        const p = await profile(ctx, input.item || input.profile || "default");
        return sidecar(ctx, "save_profile", { profile: given(p), ...(input.computer ? { computer: input.computer } : {}) });
      },
    },
    add_profile: {
      description: "Add a browser profile with no sign-ins yet, as a Vault item, for the person to sign in to sites in with sign_in.",
      input: { type: "object", properties: { profile: { type: "string", description: "its name, like Personal or PTC" }, title: { type: "string", description: "the same, as the Vault gives it" } } },
      run: async (ctx, input) => {
        const title = String(input.profile || input.title || "").trim();
        if (!title) throw new Error("Name it, like Personal or PTC.");
        if ((await profiles(ctx)).some((p) => p.title.toLowerCase() === title.toLowerCase())) throw new Error(`There's already a ${title} profile.`);
        const item = await ctx.vault.create({ title, type: TYPE, hosts: ["agent-computers"], fields: { _profile_key: { generate: true } } });
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
          if (s.item !== item && !all.some((p) => p.id === s.item)) ctx.user.append("dropped", { item: s.item });
        }
        ctx.user.append("saved", { item, blob }, [blob]);
        return "Saved.";
      },
    },
    create: {
      description: "Create a lasting computer and start it, for the person: Linux, with a browser of its own. Takes a minute. Agents borrow instead.",
      input: {"properties": {"computer": {"description": "its name: lowercase letters, digits and dashes", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "create", input),
    },
    resize: {
      description: "Change how much CPU and memory a computer may use, at once, without restarting it.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["computer", "size"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "resize", input),
    },
    start: {
      description: "Start a stopped (or paused) computer. It starts again after the server restarts.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "start", input),
    },
    stop: {
      description: "Stop a computer. Its files and sign-ins stay; it stays stopped until started.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "stop", input),
    },
    delete: {
      description: "Delete a computer and everything in it, its sign-ins too. Only when the person asked.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "delete", input),
    },
    backup: {
      description: "Back up a computer's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}}, "required": ["computer"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backup", input),
    },
    backups: {
      description: "Every server's backups of computers, newest first.",
      input: {"properties": {"computer": {"description": "only this computer's", "type": "string"}, "here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backups", input),
    },
    restore: {
      description: "Make a computer on this server from a backup: the newest of the computer named, from any server, or the one with blob.",
      input: {"properties": {"as": {"description": "the new computer's name; default the same", "type": "string"}, "blob": {"description": "a particular backup's blob", "type": "string"}, "computer": {"description": "the backed-up computer's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "restore", input),
    },
    move: {
      description: "Move a computer to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"computer": {"description": "the computer's name", "type": "string"}, "to": {"description": "the server to move it to", "type": "string"}}, "required": ["computer", "to"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "move", input),
    },
    list: {
      reads: true,
      description: "Every server's agent computers: each one's server, whether it's running, paused or stopped, its size, the memory and CPU it uses now, and who has borrowed it and what for.",
      input: { type: "object", properties: { here: { type: "boolean", description: "only this server's" } } },
      run: (ctx, input) => sidecar(ctx, "list", input),
    },
    _screen: {
      reads: true,
      description: "For the page: a small picture of a computer.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => sidecar(ctx, "screen", input),
    },
    _view: {
      reads: true,
      description: "For the page: a link to watch and use a computer.",
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
      title: "Agent Computers",
      html: PAGE,
    },
  },
};
