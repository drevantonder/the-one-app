// Desktops: the person's page for their computers: each desktop as a live picture with a browser in it, to watch,
// take over and lend, their own devices agents may use, and the browser profiles desktops start with. The desktops
// sidecar runs the desktops (Incus, or a Mac's containers); Computer Use (apps/computer) is what agents use them with,
// and this page reads the computers through it.

/// Runs one of the desktops sidecar's operations, wherever it runs for the caller. When none of their servers runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars.desktops[op](input);
  } catch (err) {
    if (String(err.message).includes("no server runs desktops for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "Desktops needs a Linux server with Incus, or a Mac, and none of yours runs it.";

/// The caller's online servers that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.servers()).filter((s) => s.online && (s.sidecars || []).includes("desktops")).map((s) => s.name);
}

export default {
  name: "desktops",
  title: "Desktops",
  icon: "macwindow.on.rectangle",
  color: "#a2845e",
  description: "See, watch and lend your computers, and keep your browser profiles",
  instructions: "Desktops is the person's page for their computers: making and sizing lasting desktops, starting, stopping and deleting them, backing them up and moving them, and keeping browser profiles. To use a computer, agents use Computer Use (computer_*), not these.",
  uses: { apps: ["computer"], sidecars: ["desktops"] },

  ops: {
    create: {
      description: "Create a lasting desktop and start it, for the person: Linux, with a browser of its own. Takes a minute. Agents borrow instead.",
      input: {"properties": {"desktop": {"description": "its name: lowercase letters, digits and dashes", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "create", input),
    },
    resize: {
      description: "Change how much CPU and memory a desktop may use, at once, without restarting it.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "required": ["desktop", "size"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "resize", input),
    },
    start: {
      description: "Start a stopped (or paused) desktop. It starts again after the server restarts.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "start", input),
    },
    stop: {
      description: "Stop a desktop. Its files and sign-ins stay; it stays stopped until started.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "stop", input),
    },
    delete: {
      description: "Delete a desktop and everything in it, its sign-ins too. Only when the person asked.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "delete", input),
    },
    backup: {
      description: "Back up a desktop's home folder (its files and sign-ins, without caches) as a blob any server can fetch, keeping the newest 5. Not off-site: backups to a bucket aren't built yet.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backup", input),
    },
    backups: {
      description: "Every server's backups of desktops, newest first.",
      input: {"properties": {"desktop": {"description": "only this desktop's", "type": "string"}, "here": {"description": "only this server's", "type": "boolean"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "backups", input),
    },
    restore: {
      description: "Make a desktop on this server from a backup: the newest of the desktop named, from any server, or the one with blob.",
      input: {"properties": {"as": {"description": "the new desktop's name; default the same", "type": "string"}, "blob": {"description": "a particular backup's blob", "type": "string"}, "desktop": {"description": "the backed-up desktop's name", "type": "string"}, "size": {"description": "small (2 CPUs, 4 GB), medium (4 CPUs, 8 GB) or large (8 CPUs, 16 GB); default small", "type": "string"}}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "restore", input),
    },
    move: {
      description: "Move a desktop to another server: backs it up, restores it there and stops it here, kept until you delete it. Takes a few minutes.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "to": {"description": "the server to move it to", "type": "string"}}, "required": ["desktop", "to"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "move", input),
    },
    save_profile: {
      description: "Save a desktop's browser profile, closing its browsers first; a sign_in desktop is also given back. Only when the person asked: borrowed desktops start from saved profiles.",
      input: {"properties": {"desktop": {"description": "the desktop's name", "type": "string"}, "profile": {"description": "the profile; default the desktop's, or default", "type": "string"}}, "required": ["desktop"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "save_profile", input),
    },
    forget_profile: {
      description: "Delete a browser profile, signing agents out of every site in it. Only when the person asked.",
      input: {"properties": {"profile": {"description": "the profile's name", "type": "string"}}, "required": ["profile"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "forget_profile", input),
    },
    _list: {
      reads: true,
      description: "For the page: Computer Use's list.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer.list(input),
    },
    _profiles: {
      reads: true,
      description: "For the page: Computer Use's profiles.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer.profiles(input),
    },
    _screen: {
      reads: true,
      description: "For the page: Computer Use's screen.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer.screen(input),
    },
    _view: {
      reads: true,
      description: "For the page: Computer Use's view.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer.view(input),
    },
    _sign_in: {
      reads: true,
      description: "For the page: Computer Use's sign_in.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer.sign_in(input),
    },
    _devices: {
      reads: true,
      description: "For the page: the person's devices agents may use, and the recent sessions.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer._devices(input),
    },
    _allow: {
      description: "For the page: lets agents use one of the person's devices, or stops them.",
      input: { type: "object", additionalProperties: true },
      run: (ctx, input) => ctx.apps.computer._allow(input),
    },
    _running: {
      reads: true,
      description: "Which of your servers it runs on, for the screens to say when none does.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({ servers: await running(ctx), needs: NEEDS }),
    },
  },

  screens: {
    main: {
      title: "Desktops",
      html: `
<style>
  body { max-width: 1120px; }
  header { display: flex; align-items: center; gap: 12px; flex-wrap: wrap; margin: 4px 0 18px; }
  header .uses { flex: 1; min-width: 0; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; color: var(--muted); font-size: 13px; }
  .meter { display: inline-flex; align-items: center; gap: 4px; flex: none; }
  button.primary { font: inherit; font-size: 15px; font-weight: 600; border: 0; border-radius: 980px; padding: 7px 16px; color: white; background: var(--accent); cursor: pointer; }
  button.primary:disabled { opacity: 0.5; cursor: default; }
  button.plain { font: inherit; font-size: 15px; border: 0; background: none; color: var(--accent); cursor: pointer; padding: 0; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 20px 14px; }
  .tile { display: block; color: inherit; min-width: 0; }
  .pic { aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--muted); font-size: 13px; }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  a.tile:hover .pic { border-color: var(--accent); }
  .tile.free .pic { background: none; border: 1px dashed var(--faint); }
  .tile.free .name { color: var(--muted); font-weight: 500; }
  .name { font-weight: 600; margin-top: 8px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .line { display: flex; align-items: center; gap: 6px; color: var(--muted); font-size: 13px; white-space: nowrap; min-width: 0; }
  .line .who { overflow: hidden; text-overflow: ellipsis; min-width: 0; }
  .line .host { margin-left: auto; padding-left: 6px; }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--faint); flex: none; }
  .dot.on { background: #34c759; }
  .dot.busy { background: #ff9500; }
  .tile.device .pic { color: var(--muted); text-align: center; padding: 0 12px; }
  .line button.plain { font-size: 13px; flex: none; }
  .banner { background: var(--card); border-radius: var(--radius); padding: 14px 16px; margin: 0 0 18px; display: flex; gap: 12px; align-items: center; flex-wrap: wrap; }
  .banner .text { flex: 1; min-width: 200px; }
  svg { flex: none; display: block; }
</style>
<header>
  <div class="uses" id="uses"></div>
  <button class="primary" id="sign-in">Sign in to a site</button>
</header>
<div id="banner"></div>
<div class="grid" id="grid"><div class="empty">Loading…</div></div>
<div id="devices-section" hidden>
<h2 class="section">Agents on Your Devices</h2>
<div class="list" id="sessions"></div>
<p class="footer">Stop an agent from the device's menu bar.</p>
</div>
<h2 class="section">Browser Profiles</h2>
<div class="list" id="profiles"></div>
<p class="footer">Each is a browser with its own sign-ins.</p>
<script>
  one.title("Desktops");
  const grid = document.getElementById("grid");
  const pics = {}; // by server/name: a data URL
  const cpus = {}; // by server/name: recent CPU percentages, oldest first
  const links = {}; // by server/name: { url, until }
  let desktops = [];
  let devs = []; // the person's devices: { server, title, on, using, permissions, asking, why }
  let sessions = []; // { agent, device, title, came_from, time }
  const asking = new Set(); // devices this page is asking to let agents in
  let signing = null; // { desktop, server, login, url }, once a sign-in is open
  let profiles = []; // { profile, server, saved }

  const key = (d) => \`\${d.server || ""}/\${d.name}\`;
  const at = (d) => (d.server ? { desktop: d.name, server: d.server } : { desktop: d.name });
  const host = (s) => String(s || "").split(".")[0];

  function size(bytes) {
    if (!(bytes > 0)) return "0 MB";
    return bytes >= 1 << 30 ? \`\${(bytes / (1 << 30)).toFixed(1)} GB\` : \`\${Math.round(bytes / (1 << 20))} MB\`;
  }
  // Memory in use and the most it may use, in bytes, however the sidecar says it.
  function memory(d) {
    const m = d.memory;
    if (m && typeof m === "object") return [m.used ?? m.usage ?? 0, m.limit ?? m.total ?? 0];
    return [m ?? 0, d.memory_limit ?? 0];
  }
  const tone = (share) => (share >= 0.85 ? "#ff3b30" : share >= 0.6 ? "#ff9f0a" : "#34c759");
  function ring(share) {
    const r = 5.5, c = 2 * Math.PI * r, s = Math.max(0, Math.min(1, share || 0));
    return \`<svg width="14" height="14" viewBox="0 0 14 14"><circle cx="7" cy="7" r="\${r}" fill="none" stroke="var(--line)" stroke-width="2.5"/>\` +
      \`<circle cx="7" cy="7" r="\${r}" fill="none" stroke="\${tone(s)}" stroke-width="2.5" stroke-dasharray="\${c * s} \${c}" transform="rotate(-90 7 7)" stroke-linecap="round"/></svg>\`;
  }
  function spark(values) {
    const w = 32, h = 12, top = Math.max(100, ...values);
    const pts = (values.length > 1 ? values : [values[0] || 0, values[0] || 0]).map((v, i, a) => \`\${(i / (a.length - 1)) * w},\${h - 1 - (v / top) * (h - 2)}\`);
    return \`<svg width="\${w}" height="\${h}"><polyline points="\${pts.join(" ")}" fill="none" stroke="#0a84ff" stroke-width="1.5" stroke-linejoin="round"/></svg>\`;
  }
  function meters(used, limit, cpu, history) {
    const share = limit ? used / limit : 0;
    return \`<span class="meter">\${ring(share)}\${size(used)}</span><span class="meter">\${spark(history)}\${Math.round(cpu || 0)}%</span>\`;
  }

  // The sidecar's list: { desktops: [...] }, or, from a sidecar before that, lines of "name: state, ...".
  function parse(listed) {
    if (listed && Array.isArray(listed.desktops)) return listed.desktops;
    if (typeof listed !== "string" || listed.startsWith("No desktops")) return [];
    return listed.split("\\n").map((l) => {
      const [name, rest = ""] = l.split(": ");
      return { name, state: rest.split(",")[0] };
    });
  }

  const free = (d) => d.spare && !d.borrowed;
  // A tile's title says what the desktop is doing; its line, who for.
  function title(d) {
    const b = d.borrowed;
    if (b?.signing_in) return "Signing in to sites";
    if (b) return b.for || "Borrowed";
    if (free(d)) return "Free";
    return d.name[0].toUpperCase() + d.name.slice(1);
  }
  function who(d) {
    const b = d.borrowed;
    if (b?.signing_in) return b.login && b.login !== "default" ? \`Profile \${b.login}\` : "";
    if (b) return b.by || "An agent";
    if (free(d)) return "Ready for the next agent";
    return "";
  }

  // A device of the person's: off, with a way to let agents use it; on; or in use, with its picture and Stop.
  function deviceTile(d) {
    const k = "device/" + d.server, name = d.title || d.server;
    const allowed = d.permissions ? d.permissions.accessibility && d.permissions.screen_recording : true;
    let pic, state;
    if (d.why) {
      pic = one.esc(d.why);
      state = \`<span class="dot"></span><span class="who">Not answering</span>\`;
    } else if (!d.on) {
      pic = asking.has(d.server) || d.asking ? "Asking on the device…" : d.missing ? one.esc(d.missing) : \`<button class="primary" data-allow="\${one.esc(d.server)}">Let Agents Use It</button>\`;
      state = \`<span class="dot"></span><span class="who">Agents can't use it</span>\`;
    } else if (d.using) {
      pic = pics[k] ? \`<img src="\${pics[k]}" alt="">\` : "";
      state = \`<span class="dot busy"></span><span class="who">\${one.esc(d.using.agent)} is using it</span><button class="plain" data-stop="\${one.esc(d.server)}">Stop</button>\`;
    } else {
      pic = allowed ? "Agents may use it" : "Waiting for you to allow it in System Settings on the Mac: Privacy &amp; Security, Accessibility and Screen Recording";
      state = \`<span class="dot on"></span><span class="who">Agents may use it</span><button class="plain" data-stop="\${one.esc(d.server)}">Turn Off</button>\`;
    }
    return \`<div class="tile device" title="\${one.esc(d.server)}"><div class="pic">\${pic}</div><div class="name">\${one.esc(name)}</div>\` +
      \`<div class="line">\${state}<span class="host">\${one.esc(host(d.server))}</span></div></div>\`;
  }

  async function allow(server, on) {
    if (on) asking.add(server);
    draw();
    try {
      await one.call("_allow", { device: server, on });
    } catch (err) {
      alert(err.message);
    } finally {
      asking.delete(server);
      await refresh();
    }
  }

  function drawSessions() {
    document.getElementById("devices-section").hidden = !devs.length && !sessions.length;
    document.getElementById("sessions").innerHTML = sessions.length
      ? sessions.map((s) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(s.agent)}</span></div>\` +
          \`<div class="sub small">\${one.esc(s.title || s.device)} · \${one.esc(one.ago(s.time, { long: true }))}\${s.came_from ? " · asked from " + one.esc(host(s.came_from)) : ""}</div></div></div>\`).join("")
      : \`<div class="row plain"><div class="main"><div class="sub small">No agent has used your devices yet.</div></div></div>\`;
  }

  function draw() {
    drawSessions();
    const mine = devs.map(deviceTile).join("");
    if (!desktops.length && mine) {
      grid.innerHTML = mine;
    } else if (!desktops.length) {
      grid.innerHTML = \`<div class="empty" style="grid-column:1/-1">No desktops yet. An agent makes one when it needs a browser, or sign in to a site to start one.</div>\`;
    } else {
      const order = (d) => (d.borrowed?.signing_in ? 0 : d.borrowed ? 1 : d.spare ? 3 : 2);
      const shown = [...desktops].sort((a, b) => order(a) - order(b) || (Date.parse(a.borrowed?.since) || 0) - (Date.parse(b.borrowed?.since) || 0) || a.name.localeCompare(b.name));
      grid.innerHTML = mine + shown.map((d) => {
        const k = key(d), running = d.state === "running";
        const pic = free(d) ? "" : pics[k] ? \`<img src="\${pics[k]}" alt="">\` : running ? "" : one.esc(d.state || "stopped");
        const [used, limit] = memory(d);
        const line = [
          \`<span class="dot\${running ? " on" : ""}"></span>\`,
          who(d) ? \`<span class="who">\${one.esc(who(d))}</span>\` : "",
          running && d.memory != null && !free(d) ? meters(used, limit, d.cpu, cpus[k] || []) : "",
          d.server ? \`<span class="host">\${one.esc(host(d.server))}</span>\` : "",
        ].join("");
        const tag = links[k] && !free(d) ? \`a href="\${one.esc(links[k].url)}" target="_blank" rel="noopener"\` : "div";
        return \`<\${tag} class="tile\${free(d) ? " free" : ""}" title="\${one.esc(d.name)}"><div class="pic">\${pic}</div>\` +
          \`<div class="name">\${one.esc(title(d))}</div><div class="line">\${line}</div></\${tag.split(" ")[0]}>\`;
      }).join("");
    }
    for (const b of grid.querySelectorAll("[data-allow]")) b.onclick = () => allow(b.dataset.allow, true);
    for (const b of grid.querySelectorAll("[data-stop]")) b.onclick = () => allow(b.dataset.stop, false);
    // What they all use, together.
    const running = desktops.filter((d) => d.state === "running");
    let used = 0, limit = 0, cpu = 0;
    for (const d of running) {
      const [u, l] = memory(d);
      used += u; limit += l; cpu += d.cpu || 0;
    }
    const borrowed = desktops.filter((d) => d.borrowed && !d.borrowed.signing_in).length;
    const parts = [\`\${running.length} running\`];
    if (borrowed) parts.push(\`\${borrowed} borrowed\`);
    const total = cpus[""] || [];
    document.getElementById("uses").innerHTML = parts.map(one.esc).join(" · ") +
      (running.some((d) => d.memory != null) ? \` · \${meters(used, limit, cpu, total)}\` : "");
    drawBanner();
  }

  function drawBanner() {
    const banner = document.getElementById("banner");
    const open = signing || desktops.find((d) => d.borrowed?.signing_in);
    if (!open) return (banner.innerHTML = "");
    const s = signing || { desktop: open.name, server: open.server, login: open.borrowed.login };
    const url = s.url || links[key({ name: s.desktop, server: s.server })]?.url;
    banner.innerHTML = \`<div class="banner"><div class="text"><b>Signing in to the \${one.esc(s.login || "default")} profile on \${one.esc(s.desktop)}</b><div class="muted small">Open the desktop, sign in to your sites in its browser (messenger.com, say), then save. Agents' desktops start signed in to them.</div></div>\` +
      (url ? \`<a class="plain" href="\${one.esc(url)}" target="_blank" rel="noopener">Open desktop</a>\` : "") +
      \`<button class="primary" id="save">Save profile</button></div>\`;
    document.getElementById("save").onclick = () => save(s);
  }

  async function save(s) {
    const button = document.getElementById("save");
    button.disabled = true;
    button.textContent = "Saving…";
    try {
      await one.call("save_profile", { desktop: s.desktop, ...(s.server ? { server: s.server } : {}), ...(s.login ? { login: s.login } : {}) });
      signing = null;
      await refresh();
      await loadProfiles();
    } catch (e) {
      button.disabled = false;
      button.textContent = "Save profile";
      alert(e.message);
    }
  }

  // Opens a desktop's browser on a profile (made if new), for the person to sign in to sites in it.
  async function signIn(button, profile) {
    const label = button.textContent;
    button.disabled = true;
    button.textContent = "Opening a desktop…";
    try {
      const r = await one.call("_sign_in", profile ? { profile } : {});
      signing = { desktop: r.desktop, server: r.server, login: r.profile || r.login, url: r.url };
      await refresh();
      return true;
    } catch (err) {
      alert(err.message);
    } finally {
      button.disabled = false;
      button.textContent = label;
    }
  }
  document.getElementById("sign-in").onclick = (e) => signIn(e.currentTarget);

  async function loadProfiles() {
    try { profiles = (await one.call("_profiles", {})).profiles || []; } catch { profiles = []; }
    const list = document.getElementById("profiles");
    list.innerHTML = profiles.map((p) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(p.profile)}</span></div>\` +
      \`<div class="sub small">Saved \${one.esc(one.ago(Date.parse(p.saved)))}\${p.server ? " on " + one.esc(host(p.server)) : ""}</div></div>\` +
      \`<button class="plain" data-profile="\${one.esc(p.profile)}">Sign in to a site</button></div>\`).join("") +
      \`<div class="row plain"><div class="main"><input id="new-profile" class="search" style="margin:0" placeholder="New profile, like Personal or PTC" autocapitalize="words" autocorrect="off"></div>\` +
      \`<button class="plain" id="add-profile">Add</button></div>\`;
    for (const b of list.querySelectorAll("[data-profile]")) b.onclick = () => signIn(b, b.dataset.profile);
    const add = document.getElementById("add-profile");
    add.onclick = async () => {
      const name = document.getElementById("new-profile").value.trim();
      if (name && (await signIn(add, name))) document.getElementById("new-profile").value = "";
    };
  }
  loadProfiles();

  // A picture of each running desktop, and a link to open it, kept until shortly before the link stops working.
  async function pictures() {
    await Promise.all(devs.filter((d) => d.on && d.using).map(async (d) => {
      try {
        const r = await one.call("_screen", { desktop: d.server, width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics["device/" + d.server] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
    }));
    await Promise.all(desktops.filter((d) => d.state === "running" && !free(d)).map(async (d) => {
      const k = key(d);
      try {
        const r = await one.call("_screen", { ...at(d), width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics[k] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
      if (!links[k] || links[k].until < Date.now()) {
        try {
          const r = await one.call("_view", { ...at(d), minutes: 60 });
          const url = typeof r === "string" ? r.split("\\n")[0] : r.url;
          if (url) links[k] = { url, until: Date.now() + 50 * 60_000 };
        } catch {}
      }
    }));
  }

  async function refresh() {
    try {
      ({ devices: devs, sessions } = await one.call("_devices", {}));
    } catch {}
    try {
      desktops = parse(await one.call("_list", {})).filter((d) => !d.device);
    } catch (e) {
      if (devs.length) {
        desktops = [];
        draw();
      } else {
        grid.innerHTML = \`<div class="empty error" style="grid-column:1/-1">\${one.esc(e.message)}</div>\`;
      }
      return;
    }
    let all = 0;
    for (const d of desktops.filter((d) => d.state === "running")) {
      const h = (cpus[key(d)] ||= []);
      h.push(d.cpu || 0);
      if (h.length > 20) h.shift();
      all += d.cpu || 0;
    }
    (cpus[""] ||= []).push(all);
    if (cpus[""].length > 20) cpus[""].shift();
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
</script>
<div class="card" id="needs" hidden></div>
<script>
one.call("_running").then(({ servers, needs }) => {
  if (servers.length) return;
  const n = document.getElementById("needs");
  n.textContent = needs;
  n.hidden = false;
  
});
</script>`,
    },
  },
};
