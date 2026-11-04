// Computer Use: the devices agents can use, and the tools they use them with. Agent devices are Linux devices with a
// browser on the person's devices, which the agent-devices sidecar runs (Podman, or a Mac's containers); the person's own
// devices are here too once they let agents use them, through the computer sidecar the Mac app hosts. Use,
// tools and screen go to a device when one is named; this app keeps who used which device when, as `session`
// entries in the person's own scope. Agent Devices (apps/agent-devices) is the person's page for them.

/// Runs one of the agent-devices sidecar's operations, wherever it runs for the caller. When none of their devices runs it,
/// says what's needed instead of how the call failed.
async function sidecar(ctx, op, input) {
  try {
    return await ctx.sidecars["agent-devices"][op](input);
  } catch (err) {
    if (String(err.message).includes("no device runs agent-devices for you")) throw new Error(NEEDS);
    throw err;
  }
}

const NEEDS = "No device of yours lets agents use it yet. Agents use your Mac once you turn on Let Agents Use This Mac in the Mac app's menu bar; agent devices, each with a browser, need Agent Devices on a Linux device with Podman, or a Mac.";

/// The caller's online devices that run the sidecar, by name.
async function running(ctx) {
  return (await ctx.devices()).filter((s) => s.online && (s.sidecars || []).includes("agent-devices")).map((s) => s.name);
}

/// The caller's devices whose computer sidecar serves them, by device name: `{ device, on, title, kind, using,
/// permissions, missing }`, or `{ device, why }` for one that didn't answer.
async function devices(ctx) {
  const mine = (s) => {
    const d = (s.installed_sidecars || []).find((d) => d.name === "computer");
    return d ? !d.users || d.users.includes(ctx.me.user_name) : (s.sidecars || []).includes("computer");
  };
  const theirs = (await ctx.devices()).filter((s) => s.online && mine(s));
  return Promise.all(theirs.map(async (s) => {
    try {
      return { device: s.name, ...(await ctx.sidecars.computer._status({}, { device: s.name })) };
    } catch (err) {
      return { device: s.name, on: false, why: err.message };
    }
  }));
}

/// The device of the person's a name means, if it's one: a device of the caller's running the computer sidecar for them.
async function device(ctx, name) {
  if (!name) return null;
  const n = String(name).toLowerCase();
  const s = (await ctx.devices()).find((s) => s.name.toLowerCase() === n && (s.sidecars || []).includes("computer"));
  return s ? s.name : null;
}

/// A device of the person's as the list shows it among the agent devices.
const asListed = (d) => ({ name: d.device, on: d.device, own: true, kind: d.kind, title: d.title, state: "running", use: d.using ? `${d.using.agent} is using it` : "agents may use it" });

const KINDS = { linux: "Linux with a browser", windows: "Windows", android: "Android", iphone: "iPhone" };

/// An agent device as an agent reads it: its name, kind, where it is and what it's doing, in words.
function shown(d) {
  const b = d.borrowed;
  const use = b?.signing_in ? `the person is signing in to sites in the ${b.profile || "browser"} profile`
    : b ? `borrowed by ${b.by || "an agent"} for ${b.for || "something"} since ${new Date(b.since).toISOString().slice(0, 16)}Z`
    : d.spare ? "the spare, kept ready for agent-devices_borrow"
    : d.kind === "linux" ? "a lasting device of the person's" : "free to borrow";
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

// The page: the person's own devices agents may use, each with a switch, a live picture while an agent uses it, and the
// recent sessions when there are any.
const PAGE = `
<style>
  body { max-width: 1120px; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(min(240px, 40vw), 1fr)); gap: 22px 14px; margin-top: 6px; }
  .pic { aspect-ratio: 16 / 10; border-radius: 10px; overflow: hidden; background: var(--card); border: 0.5px solid var(--line); display: grid; place-items: center; color: var(--faint); }
  .pic img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .head { display: flex; align-items: center; gap: 8px; margin-top: 8px; min-width: 0; }
  .name { font-weight: 600; flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .switch { transform: scale(0.78); transform-origin: right center; }
  .sub { color: var(--muted); font-size: 14px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; min-height: 1.3em; }
  .sub.bad { color: var(--danger); }
  .sub.warn { color: var(--warn); display: flex; align-items: center; gap: 5px; }
  .sub .link { color: var(--tint); cursor: pointer; }
  .perm-icon { width: 32px; height: 32px; border-radius: 8px; display: grid; place-items: center; flex: none; color: #fff; }
  .perm-icon .i { width: 18px; height: 18px; }
  .perm-lead { color: var(--muted); font-size: 15px; margin: 2px 4px 14px; line-height: 1.4; }
  .perm-status { font-size: 14px; font-weight: 500; flex: none; display: inline-flex; align-items: center; gap: 4px; }
  .perm-status.warn { color: var(--warn); font-weight: 600; }
  .perm-status.muted { color: var(--muted); }
  .step-num { width: 24px; height: 24px; border-radius: 50%; background: var(--fill); color: var(--muted); font-size: 13px; font-weight: 600; display: grid; place-items: center; flex: none; }
</style>
<div class="grid" id="grid"></div>
<div id="sessions-section" hidden>
<h2 class="section">Recent Sessions</h2>
<div class="list" id="sessions"></div>
</div>
<script>
  one.title("Computer Use");
  const grid = document.getElementById("grid");
  const pics = {}; // by device: a data URL
  const asking = new Set(); // devices this page is asking to let agents in
  const pending = new Map(); // by device: the switch's new state, until the device has it
  const notes = {}; // by device: why a switch went back
  let devs = [], sessions = [], drawn = "", listed = "", asked = 0;
  const host = (s) => String(s || "").split(".")[0];
  const laptop = '<svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="4" y="5" width="16" height="11" rx="1.5"/><path d="M2 19h20"/></svg>';

  // A device: its picture while an agent uses it, its name with a switch, and one short line when there's something to say.
  // macOS asks that screen capture can't happen without Screen Recording, so a device missing it doesn't answer
  // capture calls: its switch stays off and its tile says so.
  function view(d) {
    const permGate = d.permissions && d.kind === "mac";
    const missing = permGate && (!d.permissions.accessibility || !d.permissions.screen_recording);
    const warn = missing || asking.has(d.device) || d.asking;
    const lineHtml = notes[d.device] || (d.why ? "Not answering"
      : asking.has(d.device) || d.asking ? "Asking on the device…"
      : !d.on ? (d.missing || "")
      : d.using ? \`\${d.using.agent} is using it\`
      : "");
    let html;
    if (missing) {
      const which = !d.permissions.accessibility ? "Accessibility off" : "Screen Recording off";
      html = \`\${one.icon("warning-circle-fill")}<span>\${which} · <a class="link" data-fix="\${one.esc(d.device)}">Fix</a></span>\`;
    } else {
      html = one.esc(lineHtml);
    }
    return { src: (d.on && d.using && pics[d.device]) || "", lineHtml: html, warn: warn || missing, bad: !!notes[d.device], on: !!d.on,
      off: !!(d.why || d.missing || asking.has(d.device) || missing), title: d.title || d.device };
  }

  function tile(d) {
    const v = view(d);
    const mirror = d.kind === "mac" ? \`
      <div style="margin-top:14px;border-top:0.5px solid var(--line);padding-top:12px" data-mirror>
        <div class="pic" style="aspect-ratio:9/16;max-width:160px">
          \${d.mirroring?.running ? \`<img src="\${pics[d.device + ':mirror'] || ""}" alt="">\` : \`<svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="7" y="2.5" width="10" height="19" rx="2"/><path d="M11 18.5h2"/></svg>\`}
        </div>
        <div class="head">
          <div class="name">iPhone Mirroring</div>
          \${d.mirroring?.running ? \`<span class="perm-status muted"><span class="i">●</span> Mirroring</span>\` : \`<button class="button" data-open-mirror>\${one.icon("play")} Open</button>\`}
        </div>
        <div class="sub">\${d.mirroring?.running ? "" : "Pair on your iPhone's Settings → General → iPhone Mirroring. Capture stays inside the Mirroring window only."}</div>
      </div>\` : "";
    return \`<div data-tile="\${one.esc(d.device)}"><div class="pic">\${v.src ? \`<img src="\${v.src}" alt="">\` : laptop}</div><div class="head"><div class="name">\${one.esc(v.title)}</div>\` +
      \`<input type="checkbox" class="switch" data-device="\${one.esc(d.device)}" aria-label="Let agents use it"\${v.on ? " checked" : ""}\${v.off ? " disabled" : ""}></div>\` +
      \`<div class="sub\${v.bad ? " bad" : ""}\${v.warn ? " warn" : ""}">\${v.lineHtml}</div>\${mirror}</div>\`;
  }

  // The sheet a Fix opens, drawn again with each poll so a permission turned on shows here.
  let fixing = null; // { device, body, drawn }

  function fixHtml(d) {
    const a11y = !!d.permissions?.accessibility;
    const sr = !!d.permissions?.screen_recording;
    const name = d.title || d.device;
    return \`
        <p class="perm-lead">macOS requires two permissions before agents can view or control this Mac.</p>
        <div class="list">
          <div class="row">
            <span class="perm-icon" style="background:#007aff">\${one.icon("cursor-click-fill")}</span>
            <div class="main">
              <div class="title"><span>Accessibility</span></div>
              <div class="sub">Clicks, typing and window control</div>
            </div>
            \${a11y ? \`<span class="perm-status muted">\${one.icon("check")}</span>\` : \`<span class="perm-status warn">\${one.icon("warning-circle-fill")}Off</span>\`}
          </div>
          <div class="row">
            <span class="perm-icon" style="background:#5e5ce6">\${one.icon("desktop")}</span>
            <div class="main">
              <div class="title"><span>Screen Recording</span></div>
              <div class="sub">Lets agents see what’s on screen</div>
            </div>
            \${sr ? \`<span class="perm-status muted">\${one.icon("check")}</span>\` : \`<span class="perm-status warn">\${one.icon("warning-circle-fill")}Off</span>\`}
          </div>
        </div>

        <h2 class="section">How to turn it on</h2>
        <div class="list">
          <div class="row">
            <span class="step-num">1</span>
            <div class="main">
              <div class="title"><span>Open System Settings on \${one.esc(name)}</span></div>
              <div class="sub wrap">Go to <b>Privacy &amp; Security</b> → <b>\${a11y ? "Screen &amp; System Audio Recording" : "Accessibility"}</b>.</div>
            </div>
          </div>
          \${a11y && sr ? "" : \`<div class="row">
            <span class="step-num">2</span>
            <div class="main">
              <div class="title"><span>Turn on The One App</span></div>
              <div class="sub wrap">Enable the switch next to <b>The One App</b> and approve with Touch ID\${a11y ? ", then choose <b>Quit &amp; Reopen</b>" : ""}.</div>
            </div>
          </div>\`}
        </div>
        <p class="footer">This page updates automatically once permission is granted.</p>
      \`;
  }

  function showFix(d) {
    const html = fixHtml(d);
    const { body } = one.sheet({ title: d.title || d.device, html, onClose: () => (fixing = null) });
    fixing = { device: d.device, body, drawn: html };
  }

  grid.addEventListener("click", (e) => {
    const f = e.target.closest("[data-fix]");
    if (f) {
      const d = devs.find((x) => x.device === f.dataset.fix);
      if (d) showFix(d);
      return;
    }
    const om = e.target.closest("[data-open-mirror]");
    if (!om) return;
    const t = om.closest("[data-tile]");
    const d = devs.find((x) => x.device === t?.dataset.tile);
    if (!d) return;
    one.call("_mirror_launch", { device: d.device }).then(() => refresh()).catch((err) => {
      notes[d.device] = one.plain(err).title;
      setTimeout(() => (delete notes[d.device], paint()), 8000);
      paint();
    });
  });

  // Changes only what changed in a tile, so it doesn't flicker and a tap on it isn't lost.
  function patch(d) {
    const t = grid.querySelector(\`[data-tile="\${CSS.escape(d.device)}"]\`);
    if (!t) return;
    const v = view(d), pic = t.querySelector(".pic"), img = pic.querySelector("img");
    if (v.src && img) { if (img.getAttribute("src") !== v.src) img.src = v.src; }
    else if (v.src) { const i = new Image(); i.alt = ""; i.src = v.src; pic.replaceChildren(i); }
    else if (img) pic.innerHTML = laptop;
    t.querySelector(".name").textContent = v.title;
    const s = t.querySelector(".switch");
    s.checked = v.on;
    s.disabled = v.off;
    const sub = t.querySelector(".sub");
    sub.innerHTML = v.lineHtml;
    sub.classList.toggle("bad", v.bad);
    sub.classList.toggle("warn", v.warn);
  }

  const html = () => devs.length ? devs.map(tile).join("")
    : \`<div class="empty" style="grid-column:1/-1">Agents can use your Mac once you let them, in its menu bar.</div>\`;

  // The same devices as last time are patched in place; a device added or gone draws the grid again.
  function paint() {
    const ids = devs.map((d) => d.device).join("\\n");
    if (ids !== drawn || !grid.__drawn) {
      drawn = ids;
      one.render(grid, html);
    } else devs.forEach(patch);
    const s = sessions.slice(0, 10).map((s) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(s.agent)}</span></div>\` +
      \`<div class="sub small">\${one.esc(s.title || s.device)} · \${one.esc(one.ago(s.time, { long: true }))}\${s.came_from ? " · asked from " + one.esc(host(s.came_from)) : ""}</div></div></div>\`).join("");
    const section = document.getElementById("sessions-section");
    if (section.hidden !== !sessions.length) section.hidden = !sessions.length;
    if (s !== listed) document.getElementById("sessions").innerHTML = listed = s;
    const f = fixing && devs.find((d) => d.device === fixing.device);
    if (f && f.permissions) {
      const h = fixHtml(f);
      if (h !== fixing.drawn) fixing.body.innerHTML = fixing.drawn = h;
    }
  }

  // Whether this is the newest answer: an older poll that comes back late is dropped.
  async function load() {
    const n = ++asked, r = await one.call("_devices", {});
    if (n !== asked) return false;
    ({ devices: devs, sessions } = r);
    for (const d of devs) if (pending.has(d.device)) d.on = pending.get(d.device);
    return true;
  }

  async function pictures() {
    const live = devs.filter((d) => d.on && d.using);
    await Promise.all(live.map(async (d) => {
      try {
        const r = await one.call("screen", { device: d.device, width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics[d.device] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
    }));
    const mirrors = devs.filter((d) => d.on && d.mirroring?.running);
    await Promise.all(mirrors.map(async (d) => {
      try {
        const r = await one.call("_mirror_screen", { device: d.device, width: 480 });
        const img = (r.content || []).find((c) => c.type === "image");
        if (img) pics[d.device + ":mirror"] = \`data:\${img.mimeType};base64,\${img.data}\`;
      } catch {}
    }));
    paint();
  }

  grid.addEventListener("change", (e) => {
    const s = e.target.closest("[data-device]");
    if (s) allow(s.dataset.device, s.checked);
  });

  async function allow(device, on) {
    const d = devs.find((x) => x.device === device);
    if (!d) return;
    const was = !!d.on;
    delete notes[device];
    pending.set(device, on);
    d.on = on;
    if (on) asking.add(device);
    paint();
    try {
      await one.call("_allow", { device, on });
    } catch (err) {
      const now = devs.find((x) => x.device === device);
      if (now) now.on = was;
      notes[device] = one.plain(err).title;
      setTimeout(() => (delete notes[device], paint()), 8000);
    } finally {
      pending.delete(device);
      asking.delete(device);
      paint();
      refresh();
    }
  }

  // A poll that fails keeps the grid as it was.
  async function refresh() {
    try { if (!(await load())) return; } catch { return; }
    paint();
    await pictures();
  }

  let busy = true;
  one.render(grid, async () => {
    await load();
    drawn = devs.map((d) => d.device).join("\\n");
    return html();
  }, { skeleton: "cards" }).then(() => grid.__drawn && (paint(), pictures())).finally(() => (busy = false));

  async function tick() {
    if (busy || document.visibilityState === "hidden") return;
    busy = true;
    try { await refresh(); } finally { busy = false; }
  }
  setInterval(tick, 5000);
  document.addEventListener("visibilitychange", tick);
</script>`;

export default {
  name: "computer",
  title: "Computer Use",
  tagline: "AI uses a device",
  icon: "cursor-click",
  color: "#5e5ce6",
  advanced: true,
  description: "Devices your agents can use: agent devices with a browser, and your own devices when you let them",
  connector: true,
  needs: "Needs a Linux device with Podman, or a Mac",
  instructions: "Computer Use acts on devices: agent devices (Linux with a browser, Windows, Android and iPhones, on the person's own devices) and the person's own devices they let agents use (own: true in computer_list, like their Mac). Agent Devices (agent-devices_*) gets you one and gives it back; Computer Use (computer_*) acts on it.\n\nTo browse, agent-devices_borrow with for (a few words the person sees) gives you a Linux device whose browser starts signed in to a browser profile's sites. Pass its name as device to every call, then:\n1. computer_browser_prepare with allow_launch true and profile {\"mode\":\"isolated_named\",\"name\":<the profile borrow gave>} starts its browser.\n2. computer_use with tool list_windows gives the browser's pid and window_id.\n3. computer_get_browser_state with those gives target_id and tab_id; with those, it reads the page and the refs of what's on it.\n4. computer_browser_navigate, _click, _type, _select and _pointer act on it; on an agent device, click and pointer take input_route \"dom_event\". Read the page again after each.\nA browser idle for 5 minutes closes: prepare it again. Give the device back with agent-devices_give_back when done; one left 15 minutes without a call is given back for you.\n\nWhen a site needs the person to sign in, agent-devices_sign_in gives them a link to do it themselves; for a challenge, give them computer_view's link. Never sign in or submit forms for them unless they asked.\n\nAnything else on a device (apps, windows, keyboard and mouse, and Windows, Android and iPhones): computer_tools lists what it takes, computer_use runs one, and computer_screen shows the screen. An iPhone takes no clicks or typing.\n\nOn the person's own device, do only what they asked of it, and never read their messages, mail, browser sessions or files unless they asked. They see in their menu bar while you use it, and can stop you.",
  uses: { sidecars: ["agent-devices", "computer"] },

  schema: {
    user: `create table sessions (id text primary key, device text not null, title text, agent text not null, came_from text, time integer not null)`,
  },


  ops: {
    accounts: {
      reads: true,
      description: "Your devices that run Computer Use, as { account, ok }: account is the device's name, ok whether it's online. computer_list lists the devices agents use on them.",
      input: { type: "object", properties: {} },
      run: async (ctx) => (await ctx.devices()).filter((s) => (s.sidecars || []).some((d) => d === "agent-devices" || d === "computer")).map((s) => ({ account: s.name, ok: s.online })),
    },
    sign_out: {
      reads: false,
      confirm: true,
      description: "Stop agents using one of your devices or the agent devices on it: removes Computer Use from that device, for one of its owners. Its agent devices and backups stay there, and apps_install_on puts it back.",
      input: { type: "object", required: ["account"], properties: { account: { type: "string", description: "a device's name, from computer_accounts" } } },
      run: async (ctx, { account }) => {
        const all = await ctx.devices();
        const there = all.find((s) => s.name.toLowerCase() === String(account).toLowerCase() && (s.sidecars || []).some((d) => d === "agent-devices" || d === "computer"));
        if (!there) throw new Error(`Computer Use isn't on ${account}: it's on ${all.filter((s) => (s.sidecars || []).some((d) => d === "agent-devices" || d === "computer")).map((s) => s.name).join(", ") || "none of your devices"}`);
        for (const d of ["agent-devices", "computer"]) if ((there.sidecars || []).includes(d)) ctx.sidecar(d, there.name, false);
        return `Computer Use is removed from ${there.name}.`;
      },
    },
    list: {
      reads: true,
      description: "Every device agents can act on: the agent devices on all of your devices, and your own devices you let agents use (own: true, like your Mac). Each with its kind, the device it's on, whether it's running, paused or stopped, and who has borrowed it and what for. Borrow one with agent-devices_borrow; act on it with computer_screen, computer_use and the browser tools.",
      input: {"properties": {"here": {"description": "only the agent devices on the device you call from", "type": "boolean"}}, "type": "object"},
      run: async (ctx, input) => {
        const [listed, devs] = await Promise.all([sidecar(ctx, "list", input).catch((err) => err), input.here ? [] : devices(ctx)]);
        const on = devs.filter((d) => d.on).map(asListed);
        if (listed instanceof Error) {
          if (on.length && listed.message === NEEDS) return { devices: on };
          throw listed;
        }
        const { devices: theirs = [], ...rest } = listed || {};
        return { devices: [...on, ...theirs.map(shown)], ...rest };
      },
    },
    view: {
      reads: false,
      description: "A link for the person to watch and use an agent device in their browser: to solve a challenge or sign in, or to see what an agent is doing. Gives the link and when it stops working. Not for your own devices: the person sees those on the Computer Use page.",
      input: {"properties": {"device": DEVICE, "minutes": {"description": "how long the link works; default 60, at most 1440", "type": "integer"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) throw new Error(`${d} is the person's own device: they watch it on the Computer Use page and on the device itself, and stop agents there.`);
        return sidecar(ctx, "view", input);
      },
    },
    screen: {
      reads: true,
      description: "A picture of a device's screen as it is now: an agent device or your own device. To read a web page, computer_get_browser_state gives its text.",
      input: {"properties": {"device": DEVICE, "width": {"description": "the picture's width in pixels; default 640, at most 1920", "type": "integer"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) return ctx.sidecars.computer._screen({ width: input.width }, { device: d });
        return sidecar(ctx, "screen", input);
      },
    },
    tools: {
      reads: true,
      description: "What computer_use can do on a device, a line each (like list_windows, click, type_text); with name, that tool's full description and input.",
      input: {"properties": {"device": DEVICE, "name": {"description": "a tool's name, from computer_tools without name", "type": "string"}}, "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (d) return ctx.sidecars.computer._tools(input.name ? { name: input.name } : {}, { device: d });
        return sidecar(ctx, "tools", input);
      },
    },
    refresh_tools: {
      reads: false,
      description: "Read the agent devices' tools again from a running one, as after it was updated, when computer_tools seems out of date.",
      input: {"properties": {}, "type": "object"},
      run: (ctx, input) => sidecar(ctx, "refresh_tools", input),
    },
    use: {
      reads: false,
      description: "Run one of a device's tools, as computer_tools lists them (list_windows, launch_app, click, type_text, press_key, get_guide...), on an agent device or your own device, and give what it answers. For web pages, the browser tools are quicker.",
      input: {"properties": {"args": {"description": "the tool's input, as computer_tools with its name describes it, like { \"x\": 120, \"y\": 300 }", "type": "object"}, "device": DEVICE, "tool": {"description": "the tool's name, from computer_tools", "type": "string"}}, "required": ["tool"], "type": "object"},
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) return sidecar(ctx, "use", input);
        const r = await ctx.sidecars.computer._use({ tool: input.tool, args: input.args || {} }, { device: d });
        if (r.started) {
          const id = [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");
          ctx.user.exec("insert or replace into sessions values (?, ?, ?, ?, ?, ?)", [id, d, r.title || null, ctx.me.agent || ctx.me.name, ctx.me.name, ctx.now()]);
        }
        return r.result;
      },
    },
    get_browser_state: {
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
      run: (ctx, input) => sidecar(ctx, "get_browser_state", input),
    },
    browser_prepare: {
      reads: false,
      description: "Start the browser the browser tools act on, or attach them to one that's open. On an agent device: allow_launch true and profile { mode: \"isolated_named\", name: <the profile agent-devices_borrow gave> }. In a browser open on your own device: pid, window_id and strategy { kind: \"existing_profile\" }, when computer_get_browser_state asks for it. It says why if it can't, having changed nothing. A browser it started closes after 5 minutes idle; prepare it again.",
      input: {"additionalProperties": true, "properties": {
        "allow_launch": {"description": "true to start a browser of its own, with profile", "type": "boolean"},
        "delivery_mode": {"default": "background", "description": "background (the default) refuses where attaching would bring the browser in front of the person; foreground allows it and puts their app back after", "enum": ["background", "foreground"], "type": "string"},
        "device": DEVICE,
        "pid": {"description": "the browser's process id, from computer_use list_windows; not needed with allow_launch", "type": "integer"},
        "profile": {"additionalProperties": false, "description": "the browser profile to start with allow_launch: { mode: \"isolated_named\", name } keeps it after, { mode: \"isolated_new\" } is a fresh one", "properties": {"mode": {"enum": ["isolated_new", "isolated_named"], "type": "string"}, "name": {"description": "with isolated_named: the profile's name, as agent-devices_borrow gives it", "type": "string"}}, "required": ["mode"], "type": "object"},
        "session": SESSION,
        "strategy": {"additionalProperties": false, "description": "{ kind: \"existing_profile\" } attaches to the browser open at pid and window_id", "properties": {"kind": {"enum": ["existing_profile"], "type": "string"}}, "required": ["kind"], "type": "object"},
        "window_id": {"description": "the browser window's id, from computer_use list_windows; needed with strategy", "type": "integer"},
      }, "required": [], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_prepare", input),
    },
    browser_navigate: {
      reads: false,
      description: "Open a web address in a browser tab. Refs from earlier reads stop working: read the page again with computer_get_browser_state.",
      input: {"additionalProperties": true, "properties": {"device": DEVICE, "session": SESSION, "tab_id": TAB, "target_id": TARGET, "url": {"description": "an http, https or about address, like https://example.com", "type": "string"}}, "required": ["target_id", "tab_id", "url"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_navigate", input),
    },
    browser_click: {
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
      run: (ctx, input) => sidecar(ctx, "browser_click", input),
    },
    browser_type: {
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
      run: (ctx, input) => sidecar(ctx, "browser_type", input),
    },
    browser_select: {
      reads: false,
      description: "Pick an option of a drop-down on a web page, by its ref, without opening it: give label or value, which exactly one option must match. Read the page again to see the page took it.",
      input: {"additionalProperties": false, "properties": {
        "device": DEVICE,
        "label": {"description": "the option's text, exactly as shown", "type": "string"},
        "ref": REF, "session": SESSION, "tab_id": TAB, "target_id": TARGET,
        "value": {"description": "instead of label: the option's value", "type": "string"},
      }, "required": ["target_id", "tab_id", "ref"], "type": "object"},
      run: (ctx, input) => sidecar(ctx, "browser_select", input),
    },
    browser_dialog: {
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
      run: (ctx, input) => sidecar(ctx, "browser_dialog", input),
    },
    browser_pointer: {
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
      run: (ctx, input) => sidecar(ctx, "browser_pointer", input),
    },
    _devices: {
      description: "The person's devices that agents may use once they let them, each with whether they do and who is using it now, and the recent sessions.",
      input: { type: "object", properties: {} },
      run: async (ctx) => ({
        devices: await devices(ctx),
        sessions: ctx.user ? await ctx.user.query("select * from sessions order by time desc limit 30") : [],
      }),
    },
    _mirror_status: {
      description: "Whether iPhone Mirroring is open on a Mac, as { running, title, bounds }.",
      input: { type: "object", properties: { device: DEVICE } },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer sidecar runs there.`);
        return ctx.sidecars.computer._mirror_status({}, { device: d });
      },
    },
    _mirror_launch: {
      description: "Opens iPhone Mirroring on a Mac (pairing happens on the phone).",
      input: { type: "object", properties: { device: DEVICE } },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer sidecar runs there.`);
        return ctx.sidecars.computer._mirror_launch({}, { device: d });
      },
    },
    _mirror_screen: {
      description: "A picture of the iPhone Mirroring window as it is now.",
      input: { type: "object", properties: { device: DEVICE, width: { type: "integer", description: "the picture's width in pixels; default 640, at most 1920" } } },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer sidecar runs there.`);
        return ctx.sidecars.computer._mirror_screen({ width: input.width }, { device: d });
      },
    },
    _mirror_use: {
      description: "Runs one of a device's tools scoped to the iPhone Mirroring window.",
      input: { type: "object", properties: { device: DEVICE, tool: { type: "string" }, args: { type: "object" } }, required: ["tool"] },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't mirror ${input.device} yet: no computer sidecar runs there.`);
        const r = await ctx.sidecars.computer._mirror_use({ tool: input.tool, args: input.args || {} }, { device: d });
        if (r.started) {
          const id = [...crypto.getRandomValues(new Uint8Array(8))].map((x) => x.toString(16).padStart(2, "0")).join("");
          ctx.user.exec("insert or replace into sessions values (?, ?, ?, ?, ?, ?)", [id, d, r.title || "iPhone Mirroring", ctx.me.agent || ctx.me.name, ctx.me.name, ctx.now()]);
        }
        return r.result;
      },
    },
    _allow: {
      description: "Lets agents use one of the person's devices, or stops them. Turning it on asks the person at the device first.",
      input: { type: "object", properties: { device: { type: "string" }, on: { type: "boolean" } }, required: ["device", "on"] },
      run: async (ctx, input) => {
        const d = await device(ctx, input.device);
        if (!d) throw new Error(`Agents can't use ${input.device} yet: no computer sidecar runs there.`);
        return ctx.sidecars.computer._allow({ on: !!input.on }, { device: d });
      },
    },
  },

  screens: {
    main: {
      title: "Computer Use",
      html: PAGE,
    },
  },
};
